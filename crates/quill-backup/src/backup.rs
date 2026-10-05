use crate::digest::{sha256_bytes, sha256_file};
use crate::error::{io_err, BackupError};
use crate::manifest::{
    is_forbidden_in_backup, unsafe_reason, ExcludedEntry, Manifest, ManifestEntry, MANIFEST_NAME,
};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

const DB_SUBDIR: &str = "db";

const DATA_SUBDIR: &str = "data";

const DB_FILE: &str = "quill.db";

const TMP_SUFFIX: &str = ".part";

#[derive(Debug, Clone)]
pub struct BackupSource {
    pub pool: SqlitePool,

    pub db_path: PathBuf,

    pub data_root: PathBuf,
}

impl BackupSource {
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

#[derive(Debug, Clone)]
pub struct BackupReport {
    pub dest: PathBuf,

    pub manifest: Manifest,

    pub db_sha256: String,

    pub excluded: Vec<ExcludedEntry>,
}

#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub data_root: PathBuf,

    pub db_path: PathBuf,

    pub restored: Vec<String>,

    pub not_overwritten: Vec<String>,
}

pub async fn create_backup(
    src: &BackupSource,
    dest: impl AsRef<Path>,
) -> Result<BackupReport, BackupError> {
    let dest = dest.as_ref();

    if dest.exists() {
        let mut has_any = false;
        let rd = std::fs::read_dir(dest).map_err(|e| io_err("读取备份目标目录", dest, e))?;
        for e in rd {
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

    let db_tmp = db_dir.join(format!("{DB_FILE}{TMP_SUFFIX}"));
    let db_final = db_dir.join(DB_FILE);
    vacuum_into(&src.pool, &db_tmp).await?;

    if let Err(e) = verify_snapshot_database(&db_tmp).await {
        let _ = std::fs::remove_file(&db_tmp);
        return Err(e);
    }

    let db_bytes = file_size(&db_tmp)?;
    let db_sha256 = sha256_file(&db_tmp).map_err(|e| io_err("计算数据库快照摘要", &db_tmp, e))?;

    let mut files: Vec<ManifestEntry> = Vec::new();
    let mut excluded: Vec<ExcludedEntry> = Vec::new();
    copy_tree(
        &src.data_root,
        &src.data_root,
        &data_dir,
        &mut files,
        &mut excluded,
    )?;

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

    std::fs::rename(&db_tmp, &db_final).map_err(|e| io_err("数据库快照转正", &db_tmp, e))?;

    let excluded_for_report = manifest.excluded.clone();
    Ok(BackupReport {
        dest: dest.to_path_buf(),
        manifest,
        db_sha256,
        excluded: excluded_for_report,
    })
}

async fn vacuum_into(pool: &SqlitePool, dest: &Path) -> Result<(), BackupError> {
    let path = dest.to_string_lossy().replace('\'', "''");
    let sql = format!("VACUUM INTO '{path}'");

    sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| BackupError::Sqlx {
            stmt: "生成数据库一致性快照（VACUUM INTO）".to_string(),
            source: Box::new(e),
        })?;
    Ok(())
}

async fn verify_snapshot_database(path: &Path) -> Result<(), BackupError> {
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

fn copy_tree(
    src_root: &Path,
    cur: &Path,
    dest_root: &Path,
    files: &mut Vec<ManifestEntry>,
    excluded: &mut Vec<ExcludedEntry>,
) -> Result<(), BackupError> {
    let rd = std::fs::read_dir(cur).map_err(|e| io_err("读取用户数据目录", cur, e))?;

    let mut children: Vec<PathBuf> = Vec::new();
    for e in rd {
        children.push(e.map_err(|e| io_err("读取用户数据目录", cur, e))?.path());
    }
    children.sort();

    for child in children {
        let rel = child
            .strip_prefix(src_root)
            .map_err(|_| {
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

pub async fn restore_backup(
    backup_dir: &Path,
    db_path: &Path,
    data_root: &Path,
) -> Result<RestoreReport, BackupError> {
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

    std::fs::create_dir_all(data_root).map_err(|e| io_err("创建数据根目录", data_root, e))?;
    let mut restored = Vec::with_capacity(manifest.files.len());
    let mut not_overwritten = Vec::new();
    for f in &manifest.files {
        let target = data_root.join(&f.rel);
        if target.exists() {
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

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err("创建数据库所在目录", parent, e))?;
    }

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

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let tmp = with_tmp_suffix(path);
    std::fs::write(&tmp, bytes).map_err(|e| io_err("写入临时文件", &tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| io_err("临时文件转正", &tmp, e))?;
    Ok(())
}

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

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn digest_of(text: &str) -> String {
    sha256_bytes(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_escaping_doubles_single_quotes() {
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
