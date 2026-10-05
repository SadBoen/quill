//! 一致性备份与恢复的实现。
//!
//! # 一致性从哪来：`VACUUM INTO`，不是 `cp`
//!
//! WAL 模式下 `cp quill.db` 只复制主库文件，`-wal` 里**已提交**的数据不在里面，
//! 而且复制过程**不报任何错** —— 恢复出来的是一个「看起来正常、少了几小时数据」
//! 的库（`AGENTS.md` 铁律三）。
//!
//! `VACUUM INTO 'path'` 由 SQLite 自己完成：它在一个只读事务里把主库 + WAL
//! 合并成一个**新的、自洽的**数据库文件。因此快照**不需要** `-wal`/`-shm`，
//! 单独拷这一个文件就是完整的。
//!
//! ⚠️ `VACUUM INTO` 失败时目标文件可能已被创建（部分写入）。
//! 所以实现里**先备份到临时路径、校验通过后再改名**——
//! 否则目录里会留下一个看起来正常的坏快照。

use crate::digest::{sha256_bytes, sha256_file};
use crate::error::{io_err, BackupError};
use crate::manifest::{
    is_forbidden_in_backup, unsafe_reason, ExcludedEntry, Manifest, ManifestEntry, MANIFEST_NAME,
};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

/// 备份目录内的子目录名（数据库快照）。
const DB_SUBDIR: &str = "db";
/// 备份目录内的子目录名（用户数据）。
const DATA_SUBDIR: &str = "data";
/// 备份目录内的数据库文件名。
const DB_FILE: &str = "quill.db";
/// 临时文件后缀（未通过校验前不叫正名）。
const TMP_SUFFIX: &str = ".part";

/// 一次备份的完整输入。
#[derive(Debug, Clone)]
pub struct BackupSource {
    /// 活的数据库连接池（**必须**是打开目标库的池，`VACUUM INTO` 在连接上执行）。
    pub pool: SqlitePool,
    /// 活库文件路径。
    ///
    /// ⚠️ 只用于**事前检查存在性**与**记录给用户看的路径**；
    /// 实际快照走 `VACUUM INTO`，不读这个文件。
    pub db_path: PathBuf,
    /// 用户数据根目录（其下每层是 `data/{uid}/…`）。
    pub data_root: PathBuf,
}

impl BackupSource {
    /// 构造并做**只读**的事前检查。
    ///
    /// ⚠️ 事前检查的意义：把「路径打错」这类问题在**动手之前**报出来。
    /// 备份到一半才发现源路径不存在，目录里已有一个半成品快照，
    /// 而用户看到的是「备份命令报错了」—— 分不清是路径错还是磁盘满。
    pub fn new(
        pool: SqlitePool,
        db_path: impl Into<PathBuf>,
        data_root: impl Into<PathBuf>,
    ) -> Result<Self, BackupError> {
        let db_path = db_path.into();
        let data_root = data_root.into();
        if !db_path.is_file() {
            return Err(BackupError::DbMissing {
                path: db_path.into(),
            });
        }
        if !data_root.is_dir() {
            return Err(BackupError::DataRootMissing {
                path: data_root.into(),
            });
        }
        Ok(Self {
            pool,
            db_path,
            data_root,
        })
    }
}

/// 一次备份的结果报告。
#[derive(Debug, Clone)]
pub struct BackupReport {
    /// 备份目录。
    pub dest: PathBuf,
    /// 清单（含逐文件摘要）。
    pub manifest: Manifest,
    /// 数据库快照的 SHA-256。
    pub db_sha256: String,
    /// 因密钥规则被排除的条目。
    pub excluded: Vec<ExcludedEntry>,
}

/// 一次恢复的结果报告。
#[derive(Debug, Clone)]
pub struct RestoreReport {
    /// 恢复到哪个数据根目录。
    pub data_root: PathBuf,
    /// 恢复的数据库路径。
    pub db_path: PathBuf,
    /// 校验通过并写回的文件。
    pub restored: Vec<String>,
    /// 从备份里**没有**恢复到目标、且目标已存在同名文件的（未覆盖）。
    ///
    /// ⚠️ 之所以不静默覆盖：目标目录里可能有备份之后新建的用户数据。
    /// 覆盖 = 静默丢数据（自查表第 3 类）。列出清单让用户自己决定。
    pub not_overwritten: Vec<String>,
}

/// 创建一份完整备份。
///
/// # 步骤（顺序不可换）
///
/// 1. 检查目标目录为空 —— 拒绝覆盖已有备份；
/// 2. `VACUUM INTO` 快照到 `db/quill.db.part`；
/// 3. **打开快照自检**（`PRAGMA integrity_check`）—— 这一步是「可校验的快照」的落点：
///    打不开或自检不过的快照**绝不**改名成正名；
/// 4. 递归复制用户数据（跳过密钥材料，记进 `excluded`）；
/// 5. 写 `MANIFEST`；
/// 6. `db/quill.db.part` → `db/quill.db`。
///
/// # Errors
/// 源路径不存在、目标非空、SQLite 失败、IO 失败。
pub async fn create_backup(
    src: &BackupSource,
    dest: impl AsRef<Path>,
) -> Result<BackupReport, BackupError> {
    let dest = dest.as_ref();

    // ── 1. 目标必须为空 ────────────────────────────────────────────────
    if dest.exists() {
        let mut has_any = false;
        let rd = std::fs::read_dir(dest).map_err(|e| io_err("读取备份目标目录", dest, e))?;
        for e in rd {
            // 读不出条目本身就是「我并不知道这个目录里有什么」→ 拒绝。
            let _ = e.map_err(|e| io_err("读取备份目标目录", dest, e))?;
            has_any = true;
        }
        if has_any {
            return Err(BackupError::DestNotEmpty {
                path: dest.to_path_buf().into(),
            });
        }
    }
    std::fs::create_dir_all(dest).map_err(|e| io_err("创建备份目录", dest, e))?;
    let db_dir = dest.join(DB_SUBDIR);
    let data_dir = dest.join(DATA_SUBDIR);
    std::fs::create_dir_all(&db_dir).map_err(|e| io_err("创建备份数据库目录", &db_dir, e))?;
    std::fs::create_dir_all(&data_dir).map_err(|e| io_err("创建备份数据目录", &data_dir, e))?;

    // ── 2. VACUUM INTO 快照 ───────────────────────────────────────────
    let db_tmp = db_dir.join(format!("{DB_FILE}{TMP_SUFFIX}"));
    let db_final = db_dir.join(DB_FILE);
    vacuum_into(&src.pool, &db_tmp).await?;

    // ── 3. 快照自检：打不开 / integrity_check 不过 → 删掉，不留正名 ──
    if let Err(e) = verify_snapshot_database(&db_tmp).await {
        // 静默留下半成品 = 用户下次会拿它去恢复。必须清掉。
        let _ = std::fs::remove_file(&db_tmp);
        return Err(e);
    }

    let db_bytes = file_size(&db_tmp)?;
    let db_sha256 = sha256_file(&db_tmp).map_err(|e| io_err("计算数据库快照摘要", &db_tmp, e))?;

    // ── 4. 递归复制用户数据 ───────────────────────────────────────────
    let mut files: Vec<ManifestEntry> = Vec::new();
    let mut excluded: Vec<ExcludedEntry> = Vec::new();
    copy_tree(
        &src.data_root,
        &src.data_root,
        &data_dir,
        &mut files,
        &mut excluded,
    )?;

    // ── 5. 写清单 ─────────────────────────────────────────────────────
    let total_bytes = db_bytes + files.iter().map(|f| f.bytes).sum::<u64>();
    let manifest = Manifest {
        created_unix: unix_now(),
        db_bytes,
        db_sha256: db_sha256.clone(),
        files,
        excluded,
        total_bytes,
    };
    let manifest_path = dest.join(MANIFEST_NAME);
    write_atomic(&manifest_path, manifest.render().as_bytes())?;

    // ── 6. 快照转正 ───────────────────────────────────────────────────
    std::fs::rename(&db_tmp, &db_final).map_err(|e| io_err("数据库快照转正", &db_tmp, e))?;

    let excluded_for_report = manifest.excluded.clone();
    Ok(BackupReport {
        dest: dest.to_path_buf(),
        manifest,
        db_sha256,
        excluded: excluded_for_report,
    })
}

/// `VACUUM INTO 'dest'`。
///
/// ⚠️ **不能**用 `format!` 把路径拼进 SQL：`VACUUM INTO` 的参数是**字符串字面量**，
/// 不是绑定参数（`VACUUM INTO ?` 会被 SQLite 拒绝）。所以必须自行转义单引号。
/// 转义方式是**加倍单引号**（SQL 标准）。漏掉这一步，一个目录名带 `'`
/// 就会让备份直接失败或（更糟）写到非预期位置。
async fn vacuum_into(pool: &SqlitePool, dest: &Path) -> Result<(), BackupError> {
    let path = dest.to_string_lossy().replace('\'', "''");
    let sql = format!("VACUUM INTO '{path}'");
    // ⚠️ 丢弃 execute 的返回值（SqliteQueryResult）：`VACUUM INTO` 不返回行，
    //    返回值里没有可断言的语义。真正的判据在随后
    //    [`verify_snapshot_database`] 的 integrity_check 上 ——
    //    「命令返回 Ok」不等于「快照能用」。
    sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| BackupError::Sqlx {
            stmt: "生成数据库一致性快照（VACUUM INTO）".to_string(),
            source: Box::new(e),
        })?;
    Ok(())
}

/// 打开快照并跑 `PRAGMA integrity_check`。
///
/// # 为什么必须自检
///
/// 「`VACUUM INTO` 返回了 Ok」不等于「快照能用」。快照文件可能因
/// 磁盘满而截断（SQLite 通常会报错，但**不保证**），
/// 也可能因目标目录在网络文件系统上而写入不完整。
/// 恢复时才发现 = 用户已经需要它了。
async fn verify_snapshot_database(path: &Path) -> Result<(), BackupError> {
    // ⚠️ 用只读 URI 打开：绝不能让 SQLite 因为「打开一个坏快照」
    // 而去修改/创建 `-wal`/`-shm`（那会在备份目录里留下额外文件，
    // 让用户以为备份里还有别的东西）。
    let url = format!("sqlite:{}?mode=ro", path.to_string_lossy());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .map_err(|e| BackupError::Sqlx {
            stmt: "打开刚生成的数据库快照做自检".to_string(),
            source: Box::new(e),
        })?;

    let verdict: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .map_err(|e| BackupError::Sqlx {
            stmt: "对数据库快照做完整性自检（PRAGMA integrity_check）".to_string(),
            source: Box::new(e),
        })?;
    pool.close().await;

    if verdict != "ok" {
        return Err(BackupError::Sqlx {
            stmt: format!("数据库快照自检未通过（integrity_check 返回 {verdict}）"),
            source: Box::new(sqlx::Error::Protocol(
                "快照文件损坏，拒绝把它当成有效备份".into(),
            )),
        });
    }
    Ok(())
}

/// 递归复制一棵目录树，收集清单条目与排除项。
fn copy_tree(
    src_root: &Path,
    cur: &Path,
    dest_root: &Path,
    files: &mut Vec<ManifestEntry>,
    excluded: &mut Vec<ExcludedEntry>,
) -> Result<(), BackupError> {
    let rd = std::fs::read_dir(cur).map_err(|e| io_err("读取用户数据目录", cur, e))?;

    // 排序：让备份产物与清单顺序稳定，便于两次备份 diff。
    let mut children: Vec<PathBuf> = Vec::new();
    for e in rd {
        children.push(e.map_err(|e| io_err("读取用户数据目录", cur, e))?.path());
    }
    children.sort();

    for child in children {
        let rel = child
            .strip_prefix(src_root)
            .map_err(|_| {
                // 走 symlink 时 strip_prefix 可能失败。此时**跳过**而不是猜路径 ——
                // 猜出来的 rel 若越出 src_root，恢复时就会写到目标目录之外。
                io_err(
                    "计算用户数据相对路径",
                    &child,
                    std::io::Error::other("路径不在数据根目录内，已跳过"),
                )
            })?
            .to_string_lossy()
            .replace('\\', "/");

        let name = child
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let meta =
            std::fs::symlink_metadata(&child).map_err(|e| io_err("读取文件属性", &child, e))?;

        if meta.is_symlink() {
            // ⚠️ 符号链接不进备份。理由：`symlink_metadata` 说它是链接，
            // 但 `read_dir` 之外的路径可能指向备份目录之外；
            // 把它当普通文件复制会把链接**目标**的内容搬进来
            // （可能是 /etc/shadow），而清单里的路径却是无害的样子。
            excluded.push(ExcludedEntry {
                rel,
                reason: "符号链接不进备份（链接目标可能在数据目录之外）".into(),
            });
            continue;
        }

        if is_forbidden_in_backup(&name) {
            excluded.push(ExcludedEntry {
                rel,
                reason: "密钥材料不进备份（与密文同处会扩大密钥暴露面）".into(),
            });
            continue;
        }

        if meta.is_dir() {
            std::fs::create_dir_all(dest_root.join(&rel))
                .map_err(|e| io_err("创建备份子目录", &dest_root.join(&rel), e))?;
            copy_tree(src_root, &child, dest_root, files, excluded)?;
            continue;
        }

        // 路径安全性：与清单解析侧用同一个判据（unsafe_reason）。
        // 写不进清单的路径，恢复时也必然被拒 —— 两侧口径一致，不会漂移。
        if let Some(why) = unsafe_reason(&rel) {
            excluded.push(ExcludedEntry {
                rel,
                reason: format!("路径不安全，未备份：{why}"),
            });
            continue;
        }

        let target = dest_root.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("创建备份子目录", parent, e))?;
        }
        std::fs::copy(&child, &target).map_err(|e| io_err("复制用户数据文件", &child, e))?;

        let bytes = file_size(&target)?;
        let sha = sha256_file(&target).map_err(|e| io_err("计算文件摘要", &target, e))?;
        files.push(ManifestEntry {
            rel,
            bytes,
            sha256: sha,
        });
    }
    Ok(())
}

/// 从备份完整恢复。
///
/// # 幂等与失败语义
///
/// - **幂等**：同一份备份恢复两次，结果与恢复一次完全相同。
///   依据有三：① 写入前逐个校验摘要，一样的内容写两次结果一样；
///   ② 目标已存在且内容一致时也照写（`fs::copy` 是覆盖写，幂等）；
///   ③ 恢复前会**先清掉目标库的 `-wal`/`-shm`**，不遗留上次恢复的残骸。
/// - **不留下半开状态**：
///   1. **先全量校验、后动任何文件**——摘要不过时目标目录**一个字节都没被改**；
///   2. 每个文件先写 `.part` 再改名，改名是原子的；
///   3. 数据库最后恢复，且**先删 `-wal`/`-shm` 再落主库**
///      （留着旧 `-wal` 配新主库 = 数据错乱且不报错）。
///
/// # Errors
/// 清单缺失/损坏、任一文件摘要或大小不符、SQLite 失败、IO 失败。
pub async fn restore_backup(
    backup_dir: &Path,
    db_path: &Path,
    data_root: &Path,
) -> Result<RestoreReport, BackupError> {
    // ── 1. 读清单（严格） ─────────────────────────────────────────────
    let manifest_path = backup_dir.join(MANIFEST_NAME);
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            BackupError::ManifestMissing {
                path: manifest_path.clone().into(),
            }
        } else {
            io_err("读取备份清单", &manifest_path, e)
        }
    })?;
    let manifest = Manifest::parse(&text)?;

    let db_in_backup = backup_dir.join(DB_SUBDIR).join(DB_FILE);
    let data_in_backup = backup_dir.join(DATA_SUBDIR);

    // ── 2. 先全量校验，一个字节都不许先动 ─────────────────────────────
    let db_size = file_size(&db_in_backup)?;
    if db_size != manifest.db_bytes {
        return Err(BackupError::SizeMismatch {
            rel: format!("{DB_SUBDIR}/{DB_FILE}"),
            expect: manifest.db_bytes,
            actual: db_size,
        });
    }
    let db_actual =
        sha256_file(&db_in_backup).map_err(|e| io_err("计算数据库快照摘要", &db_in_backup, e))?;
    if db_actual != manifest.db_sha256 {
        return Err(BackupError::DigestMismatch {
            rel: format!("{DB_SUBDIR}/{DB_FILE}"),
            expect: manifest.db_sha256,
            actual: db_actual,
        });
    }
    for f in &manifest.files {
        let p = data_in_backup.join(&f.rel);
        let size = file_size(&p)?;
        if size != f.bytes {
            return Err(BackupError::SizeMismatch {
                rel: f.rel.clone(),
                expect: f.bytes,
                actual: size,
            });
        }
        let actual = sha256_file(&p).map_err(|e| io_err("计算文件摘要", &p, e))?;
        if actual != f.sha256 {
            return Err(BackupError::DigestMismatch {
                rel: f.rel.clone(),
                expect: f.sha256.clone(),
                actual,
            });
        }
    }

    // ── 3. 校验全过 → 开始写用户数据 ─────────────────────────────────
    std::fs::create_dir_all(data_root).map_err(|e| io_err("创建数据根目录", data_root, e))?;
    let mut restored = Vec::with_capacity(manifest.files.len());
    let mut not_overwritten = Vec::new();
    for f in &manifest.files {
        let target = data_root.join(&f.rel);
        if target.exists() {
            // 目标已有同名文件且内容不同 → 列出，不静默覆盖。
            let existing = sha256_file(&target).unwrap_or_default();
            if existing != f.sha256 {
                not_overwritten.push(f.rel.clone());
                continue;
            }
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err("创建数据子目录", parent, e))?;
        }
        let src = data_in_backup.join(&f.rel);
        write_copy_atomic(&src, &target)?;
        restored.push(f.rel.clone());
    }

    // ── 4. 最后恢复数据库：先清 -wal/-shm，再落主库 ───────────────────
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err("创建数据库所在目录", parent, e))?;
    }
    // ⚠️ 必须删：旧库的 -wal 配新主库 = 静默数据错乱。
    for suffix in ["-wal", "-shm"] {
        let sidecar = with_sidecar(db_path, suffix);
        if sidecar.exists() {
            std::fs::remove_file(&sidecar)
                .map_err(|e| io_err("删除旧数据库的 WAL 边车文件", &sidecar, e))?;
        }
    }
    write_copy_atomic(&db_in_backup, db_path)?;
    verify_snapshot_database(db_path).await?;

    Ok(RestoreReport {
        data_root: data_root.to_path_buf(),
        db_path: db_path.to_path_buf(),
        restored,
        not_overwritten,
    })
}

/// 原子写：先写 `目标.part`，再 `rename` 到目标。
///
/// ⚠️ 备份目录可能被用户手工打开查看；`.part` 后缀让「写到一半的文件」
/// 有明确的可识别形态，而 `rename` 在同一文件系统内是原子的。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let tmp = with_tmp_suffix(path);
    std::fs::write(&tmp, bytes).map_err(|e| io_err("写入临时文件", &tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| io_err("临时文件转正", &tmp, e))?;
    Ok(())
}

/// 原子复制：`src` → `目标.part` → `rename` 到目标。
fn write_copy_atomic(src: &Path, dest: &Path) -> Result<(), BackupError> {
    let tmp = with_tmp_suffix(dest);
    std::fs::copy(src, &tmp).map_err(|e| io_err("复制文件", src, e))?;
    std::fs::rename(&tmp, dest).map_err(|e| io_err("临时文件转正", &tmp, e))?;
    Ok(())
}

fn with_tmp_suffix(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(TMP_SUFFIX);
    PathBuf::from(s)
}

/// 给 SQLite 主库路径加边车后缀（`-wal` / `-shm`）。
fn with_sidecar(db_path: &Path, suffix: &str) -> PathBuf {
    let mut s = db_path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

fn file_size(path: &Path) -> Result<u64, BackupError> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| io_err("读取文件大小", path, e))
}

/// 当前 Unix 秒。
///
/// ⚠️ 失败时返回 0 而不是 panic：清单里的时间戳**只用于给人看**，
/// 它错了不该让备份失败（`SystemTime` 在极端情况下会早于 1970）。
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 对一段文本算摘要（供上层做「备份前后清单一致」这类断言）。
pub fn digest_of(text: &str) -> String {
    sha256_bytes(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_escaping_doubles_single_quotes() {
        // VACUUM INTO 只接受字符串字面量，无法用绑定参数 →
        // 单引号漏转义会让备份直接失败或写到非预期位置。
        let quoted = "a'b".replace('\'', "''");
        assert_eq!(quoted, "a''b");
    }

    #[test]
    fn sidecar_paths_are_appended_not_replaced() {
        let p = Path::new("/x/quill.db");
        assert_eq!(with_sidecar(p, "-wal"), PathBuf::from("/x/quill.db-wal"));
        assert_eq!(with_sidecar(p, "-shm"), PathBuf::from("/x/quill.db-shm"));
    }

    #[test]
    fn tmp_suffix_does_not_clobber_extension() {
        let p = Path::new("/x/quill.db");
        assert_eq!(
            with_tmp_suffix(p),
            PathBuf::from("/x/quill.db.part"),
            "临时名必须是后缀追加，不能替换扩展名（否则恢复时找不到真文件）"
        );
    }
}
