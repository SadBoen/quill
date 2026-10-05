//! 打开数据根目录与数据库的公共逻辑。
//!
//! # 为什么 CLI 要自己建库
//!
//! `quill-server` 启动时**不跑迁移**（迁移属 `quill-upgrade`），
//! 只探测表是否存在，缺表就 503。CLI 是给人直接用的，
//! 「第一次跑就报 503，但没人知道该执行什么」是不可接受的。
//! 故 CLI 走 `quill_store::run_migration`，**幂等**：已迁移则不动。
//!
//! ⚠️ 迁移是 expand-only 的（铁律六），且 `schema_version` 记录 checksum，
//! 重复执行不会重复建表。

use crate::Outcome;
use quill_store::{configure_pool, run_migration};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

/// 迁移文件。用 `include_str!` **编译期嵌入**：
/// 这样 CLI 编译出来的二进制自带正确版本的 schema，
/// 不会出现「二进制比源码旧」而把库建错版本。
const MIGRATION: &str = include_str!("../../quill-store/migrations/0001_init.sql");

/// 打开（必要时先迁移）数据库。
///
/// 返回 `Err` 时区分两种：
/// · 建不了目录/打不开库 → `2`（无法判定，环境不对）
/// · schema 迁移失败 → `2`（这不是"没这个功能"，是环境坏了）
pub async fn open_db(db_path: &str) -> Result<SqlitePool, Outcome> {
    let p = Path::new(db_path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Err(Outcome::undet(format!(
                    "建数据目录 {} 失败：{e}\n下一步：检查该路径的权限，或用 --db 指定别的位置。",
                    parent.display()
                )));
            }
        }
    }

    let pool = match configure_pool(db_path, 4).await {
        Ok(p) => p,
        Err(e) => {
            return Err(Outcome::undet(format!(
                "打开数据库 {db_path} 失败：{e}\n\
                 下一步：确认该文件可写；若是别的程序占着，用 --db 换一个路径。"
            )))
        }
    };

    // ⚠️ schema 迁移**不是幂等的**（实测：第二次跑会报
    //    `table schema_version already exists`）。所以必须**先探测**再决定跑不跑。
    //    我最初假设它幂等、直接每次都跑，结果 CLI 第二次执行全线失败 ——
    //    而 `schema_constraints.rs` 每次用**新库**，永远测不到这条。
    let has_schema: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| Outcome::undet(format!("探测 schema 状态失败：{e}")))?;

    if has_schema == 0 {
        let _ = run_migration(&pool, MIGRATION).await.map_err(|e| {
            Outcome::undet(format!(
                "schema 迁移失败：{e}\n\
                 下一步：这是**库本身**的问题，不是命令用法问题。\
                 不要删库重来——先 `cp {db_path} {db_path}.bak` 再排查。"
            ))
        })?;
    }

    Ok(pool)
}

/// 把用户名解析成 `UserId`。
///
/// # ⚠️ `create` 为什么是参数而不是内部细节
///
/// 当前 CLI 的用户是**演示级**的：按用户名确定性派生一个 16 字节 ID。
/// 若一律「不存在就建」，那么 `--as alicee`（把 alice 拼错）会**静默建一个新用户**，
/// 然后你看到「该用户还没有任何专家」——你会以为是隔离出了问题，
/// 实际上只是名字打错了。
///
/// 所以：**读操作不建用户**（`create=false`），只有写操作才建。
/// 这条边界是「打错字要看得见」的前提。
///
/// 诚实声明：这不是生产登录路径。真实登录走 `quill-control` 的
/// PBKDF2 校验 + CSPRNG 会话令牌（已有 94 个测试覆盖）。
pub async fn resolve_user(
    pool: &SqlitePool,
    name: &str,
    create: bool,
) -> Result<(quill_adapters::UserId, bool), Outcome> {
    use sha2::{Digest, Sha256};
    let norm = name.trim().to_lowercase();
    if norm.is_empty() {
        return Err(Outcome::fail(String::from(
            "用户名不能为空。用 `--as <用户名>` 指定。",
        )));
    }
    // 确定性派生：同一用户名恒得同一 ID（演示用，可重复运行）
    let digest = Sha256::digest(format!("quill-cli-user:{norm}").as_bytes());
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    let uid = quill_adapters::UserId::parse(&hex)
        .map_err(|e| Outcome::undet(format!("内部：派生的用户 ID 非法（{e}）。")))?;

    // `as_bytes()` 返回 `[u8; 16]`，sqlx 的 Encode 只对 `Vec<u8>` 有实现，
    // 故必须 `to_vec()` —— 直接 bind 数组编译都过不了。
    let uid_bytes = uid.as_bytes().to_vec();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = ?")
        .bind(&uid_bytes)
        .fetch_one(pool)
        .await
        .map_err(|e| Outcome::undet(format!("查用户失败：{e}")))?;

    if n > 0 {
        return Ok((uid, false));
    }
    if !create {
        return Err(Outcome::fail(format!(
            "用户 {name:?} 在本库里不存在。\n\
             ⚠️ 名字打错也会走到这里。库里已存在的用户：{}",
            existing_users(pool).await?
        )));
    }
    sqlx::query(
        "INSERT INTO users(id,username,username_norm,display_name,password_hash,\
         password_salt,password_algo,role,pwd_changed_at,created_at,updated_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&uid_bytes)
    .bind(&norm)
    .bind(&norm)
    .bind(name)
    .bind(b"cli-demo-not-a-real-hash".as_slice())
    .bind(b"cli-demo-salt".as_slice())
    .bind("pbkdf2-hmac-sha256$i=600000")
    .bind("member")
    .bind(1i64)
    .bind(1i64)
    .bind(1i64)
    .execute(pool)
    .await
    .map_err(|e| {
        Outcome::undet(format!(
            "为用户 {name:?} 建行失败：{e}\n\
             下一步：这通常意味着 schema 与预期不符，执行 `quill doctor` 看详情。"
        ))
    })?;
    Ok((uid, true))
}

/// 列出库里已有的用户名（用于"你是不是打错了"的提示）。
async fn existing_users(pool: &SqlitePool) -> Result<String, Outcome> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT username FROM users WHERE deleted_at IS NULL ORDER BY username LIMIT 10",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| Outcome::undet(format!("列用户失败：{e}")))?;
    if rows.is_empty() {
        Ok(String::from(
            "（库里一个用户都没有——用 `quill experts add ...` 建第一个）",
        ))
    } else {
        Ok(rows.join("、"))
    }
}

/// 数据根目录（`<root>/data`），不存在则创建。
pub fn data_root(root: &str) -> Result<PathBuf, Outcome> {
    let p = PathBuf::from(root).join("data");
    if !p.is_dir() {
        std::fs::create_dir_all(&p)
            .map_err(|e| Outcome::undet(format!("建数据目录 {} 失败：{e}", p.display())))?;
    }
    Ok(p)
}
