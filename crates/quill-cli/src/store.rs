use crate::Outcome;
use quill_store::configure_pool;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

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

    quill_store::migrate(&pool).await.map_err(|e| {
        Outcome::undet(format!(
            "schema 迁移失败：{e}\n\
             下一步：这是**库本身**的问题，不是命令用法问题。\
             不要删库重来——先 `cp {db_path} {db_path}.bak` 再排查。"
        ))
    })?;

    Ok(pool)
}

pub async fn resolve_user(
    pool: &SqlitePool,
    name: &str,
    create: bool,
) -> Result<(quill_adapters::UserId, bool), Outcome> {
    if name.trim().is_empty() {
        return Err(Outcome::fail(String::from(
            "用户名不能为空。用 `--as <用户名>` 指定。",
        )));
    }

    let uid = quill_control::derive_user_id(name).map_err(|e| Outcome::fail(e.to_string()))?;
    let norm = name.trim().to_lowercase();

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

pub fn data_root(root: &str) -> Result<PathBuf, Outcome> {
    let p = PathBuf::from(root).join("data");
    if !p.is_dir() {
        std::fs::create_dir_all(&p)
            .map_err(|e| Outcome::undet(format!("建数据目录 {} 失败：{e}", p.display())))?;
    }
    Ok(p)
}
