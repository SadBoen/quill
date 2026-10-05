use sqlx::SqlitePool;

use crate::password::{validate_password, PasswordHasher, Pbkdf2Params};
use crate::user::{normalize_username, validate_display_name, validate_username};
use crate::{ControlError, OsEntropySource, UserId};

/// 仅供令牌引导使用：账号没有密码，只能靠 Bearer 令牌进来。
pub const TOKEN_ONLY_ALGO: &str = "token-only";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provision {
    Created,

    AlreadyPresent,
}

/// 密码账号的建档结果。
/// - `Created`：这个用户名此前不存在。
/// - `Upgraded`：原本是只有令牌、没有口令的账号，这次补上了口令。
/// - `Rotated`：本来就有口令，这次按配置换成了新值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordProvision {
    Created,

    Upgraded,

    Rotated,
}

pub async fn ensure_token_user(
    pool: &SqlitePool,
    id: UserId,
    username: &str,
    is_admin: bool,
) -> Result<Provision, ControlError> {
    let norm = normalize_username(username);
    if norm.is_empty() {
        return Err(ControlError::UsernameInvalid {
            raw: username.to_string(),
            reason: "归一后为空串",
        });
    }

    let blob = id.as_bytes().to_vec();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = ?")
        .bind(&blob)
        .fetch_one(pool)
        .await
        .map_err(storage)?;
    if n > 0 {
        return Ok(Provision::AlreadyPresent);
    }

    let now = now_ms();
    let role = if is_admin { "owner" } else { "member" };

    let r = sqlx::query(
        "INSERT INTO users(id,username,username_norm,display_name,password_hash,\
         password_salt,password_algo,role,pwd_changed_at,created_at,updated_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&blob)
    .bind(&norm)
    .bind(&norm)
    .bind(&norm)
    .bind(b"token-only-no-password".as_slice())
    .bind(b"token-only".as_slice())
    .bind(TOKEN_ONLY_ALGO)
    .bind(role)
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await;

    match r {
        Ok(_) => Ok(Provision::Created),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Ok(Provision::AlreadyPresent)
        }
        Err(e) => Err(storage(e)),
    }
}

/// 建立（或改写）一个**能用用户名 + 密码登录**的账号。
///
/// 这是本实例**唯一**能产生密码账号的入口，也是「注册默认关闭」的实现方式：
/// 没有 `POST /api/auth/register` 的成功路径，账号只能由部署者写进
/// `QUILL_PASSWORD_USERS` 后重启服务。
///
/// 与 [`ensure_token_user`] 的区别：
/// - 令牌引导写 `password_algo = 'token-only'`，**明确记下自己没有口令**，
///   拿口令试登录会落到统一的凭据错误上，不会看起来像「口令为空」。
/// - 这里写真实的 PBKDF2 摘要，且**每次启动都以配置为准覆盖** —— 关闭注册后
///   没有别的地方能改口令，所以「改环境变量 + 重启」必须真的生效。
pub async fn ensure_password_user(
    pool: &SqlitePool,
    username: &str,
    password: &str,
    is_admin: bool,
    params: Pbkdf2Params,
) -> Result<PasswordProvision, ControlError> {
    let norm = normalize_username(username);
    if norm.is_empty() {
        return Err(ControlError::UsernameInvalid {
            raw: username.to_string(),
            reason: "归一后为空串",
        });
    }
    validate_username(&norm)?;
    validate_password(password, &norm)?;
    let display = norm.clone();
    validate_display_name(&display)?;

    let id: UserId = crate::derive_user_id(username)?;
    let digest = PasswordHasher::new(params).hash(&OsEntropySource, password);
    let blob = id.as_bytes().to_vec();

    let existing: Option<String> = sqlx::query_scalar("SELECT password_algo FROM users WHERE id = ?")
        .bind(&blob)
        .fetch_optional(pool)
        .await
        .map_err(storage)?;

    let now = now_ms();
    let role = if is_admin { "owner" } else { "member" };

    match existing {
        None => {
            sqlx::query(
                "INSERT INTO users(id,username,username_norm,display_name,password_hash,\
                 password_salt,password_algo,role,pwd_changed_at,created_at,updated_at) \
                 VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&blob)
            .bind(&norm)
            .bind(&norm)
            .bind(&display)
            .bind(digest.hash.as_slice())
            .bind(digest.salt.as_slice())
            .bind(&digest.algo_tag)
            .bind(role)
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .map_err(|e| match e {
                sqlx::Error::Database(dbe) if dbe.is_unique_violation() => ControlError::Storage {
                    detail: format!("建密码账号时用户名 {norm} 已被占用：{dbe}"),
                },
                other => storage(other),
            })?;
            Ok(PasswordProvision::Created)
        }
        Some(prev_algo) => {
            // 已有账号：只换口令与角色，不动 id / status / created_at，
            // 这样用户已有的会话、专家、团队都不会因为改了口令而丢关联。
            sqlx::query(
                "UPDATE users SET password_hash = ?, password_salt = ?, password_algo = ?,\
                 role = ?, pwd_changed_at = ?, updated_at = ? WHERE id = ?",
            )
            .bind(digest.hash.as_slice())
            .bind(digest.salt.as_slice())
            .bind(&digest.algo_tag)
            .bind(role)
            .bind(now)
            .bind(now)
            .bind(&blob)
            .execute(pool)
            .await
            .map_err(storage)?;

            Ok(if prev_algo == TOKEN_ONLY_ALGO {
                PasswordProvision::Upgraded
            } else {
                PasswordProvision::Rotated
            })
        }
    }
}

fn storage(e: sqlx::Error) -> ControlError {
    ControlError::Storage {
        detail: format!("引导账号时写库失败：{e}"),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive_user_id;
    use crate::password::PBKDF2_ALGO_PREFIX;
    use crate::{ControlPlane, ManualClock};
    use std::sync::Arc;

    async fn pool() -> SqlitePool {
        let p = quill_store::in_memory().await.expect("内存库");
        quill_store::migrate(&p).await.expect("迁移");
        p
    }

    async fn count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(pool)
            .await
            .expect("计数")
    }

    fn control(p: &SqlitePool) -> ControlPlane {
        ControlPlane::new(
            p.clone(),
            Arc::new(ManualClock::new(1_000)),
            Arc::new(OsEntropySource),
            Pbkdf2Params::for_tests(),
        )
    }

    const PW: &str = "correct-horse-battery";
    const PW2: &str = "another-long-passphrase";

    async fn pw_algo(pool: &SqlitePool, username: &str) -> String {
        let id = derive_user_id(username).expect("id");
        sqlx::query_scalar("SELECT password_algo FROM users WHERE id = ?")
            .bind(id.as_bytes().to_vec())
            .fetch_one(pool)
            .await
            .expect("读 algo")
    }

    #[tokio::test]
    async fn token_user_is_created_once_and_is_idempotent() {
        let p = pool().await;
        let id = derive_user_id("alice").expect("id");

        assert_eq!(
            ensure_token_user(&p, id, "alice", false).await.expect("首次"),
            Provision::Created
        );
        assert_eq!(
            ensure_token_user(&p, id, "alice", false).await.expect("再次"),
            Provision::AlreadyPresent
        );
        assert_eq!(count(&p).await, 1, "不得因重复引导插出第二行");
    }

    #[tokio::test]
    async fn provisioned_user_can_actually_hold_child_rows() {
        let p = pool().await;
        let id = derive_user_id("alice").expect("id");
        ensure_token_user(&p, id, "alice", false).await.expect("引导");

        let sid = quill_domain::SessionId::from_bytes([9u8; 16]);
        let r = sqlx::query(
            "INSERT INTO sessions(user_id,id,kind,room_id,workspace_path,created_at,updated_at,last_active_at)
             VALUES(?,?,'solo','room-1','ws/1',1,1,1)",
        )
        .bind(id.as_bytes().to_vec())
        .bind(sid.as_bytes().to_vec())
        .execute(&p)
        .await;

        assert!(
            r.is_ok(),
            "引导后的用户必须能写 sessions（外键会拦住不存在的人）：{:?}",
            r.err()
        );
    }

    #[tokio::test]
    async fn token_only_account_records_that_it_has_no_password() {
        let p = pool().await;
        let id = derive_user_id("alice").expect("id");
        ensure_token_user(&p, id, "alice", false).await.expect("引导");

        let algo: String = sqlx::query_scalar("SELECT password_algo FROM users WHERE id = ?")
            .bind(id.as_bytes().to_vec())
            .fetch_one(&p)
            .await
            .expect("读 algo");
        assert_eq!(algo, TOKEN_ONLY_ALGO, "不得谎称有密码");
    }

    #[tokio::test]
    async fn admin_token_provisions_an_owner_row() {
        let p = pool().await;
        let id = derive_user_id("boss").expect("id");
        ensure_token_user(&p, id, "boss", true).await.expect("引导");

        let role: String = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
            .bind(id.as_bytes().to_vec())
            .fetch_one(&p)
            .await
            .expect("读 role");
        assert_eq!(role, "owner");
    }

    #[tokio::test]
    async fn blank_username_is_refused() {
        let p = pool().await;
        let id = derive_user_id("alice").expect("id");
        assert!(ensure_token_user(&p, id, "   ", false).await.is_err());
        assert_eq!(count(&p).await, 0);
    }

    // ---------- 密码账号：登录链路的唯一来源 ----------

    #[tokio::test]
    async fn password_user_is_created_and_can_log_in() {
        let p = pool().await;
        let out = ensure_password_user(&p, "carol", PW, false, Pbkdf2Params::for_tests())
            .await
            .expect("建密码账号");
        assert_eq!(out, PasswordProvision::Created);

        let s = control(&p).login("carol", PW, None, None).await.expect("应能登录");
        assert_eq!(s.username_norm, "carol");
    }

    #[tokio::test]
    async fn a_password_account_never_records_token_only() {
        let p = pool().await;
        ensure_password_user(&p, "carol", PW, false, Pbkdf2Params::for_tests())
            .await
            .expect("建");

        let algo = pw_algo(&p, "carol").await;
        assert_ne!(algo, TOKEN_ONLY_ALGO, "有口令的账号不得自称 token-only");
        assert!(algo.starts_with(PBKDF2_ALGO_PREFIX), "实际 algo = {algo}");
    }

    #[tokio::test]
    async fn token_only_user_is_upgraded_and_can_then_log_in() {
        // 实际部署最常见的路径：QUILL_TOKENS 先引导出 alice，
        // 后来在 QUILL_PASSWORD_USERS 里补了口令。
        let p = pool().await;
        ensure_token_user(&p, derive_user_id("alice").expect("id"), "alice", false)
            .await
            .expect("令牌引导");
        assert_eq!(pw_algo(&p, "alice").await, TOKEN_ONLY_ALGO);

        let out = ensure_password_user(&p, "alice", PW, false, Pbkdf2Params::for_tests())
            .await
            .expect("补口令");
        assert_eq!(out, PasswordProvision::Upgraded);

        assert!(control(&p).login("alice", PW, None, None).await.is_ok());
    }

    #[tokio::test]
    async fn reconfiguring_the_password_takes_effect_on_the_next_boot() {
        let p = pool().await;
        let params = Pbkdf2Params::for_tests();
        ensure_password_user(&p, "alice", PW, false, params)
            .await
            .expect("首次");

        let out = ensure_password_user(&p, "alice", PW2, false, params)
            .await
            .expect("二次");
        assert_eq!(out, PasswordProvision::Rotated);

        let cp = control(&p);
        assert!(
            cp.login("alice", PW, None, None).await.is_err(),
            "旧口令必须失效，否则改配置等于没改"
        );
        assert!(cp.login("alice", PW2, None, None).await.is_ok());
    }

    #[tokio::test]
    async fn token_only_user_cannot_log_in_with_any_password() {
        // 关闭注册后，只有 QUILL_PASSWORD_USERS 里的人能登录。
        // 令牌账号必须**任何口令都登不进来**，否则等于开了后门。
        let p = pool().await;
        ensure_token_user(&p, derive_user_id("alice").expect("id"), "alice", false)
            .await
            .expect("令牌引导");

        let cp = control(&p);
        assert!(cp.login("alice", PW, None, None).await.is_err());
        assert!(cp.login("alice", "", None, None).await.is_err());
    }

    #[tokio::test]
    async fn a_short_password_is_refused_and_no_row_is_written() {
        let p = pool().await;
        ensure_password_user(&p, "alice", "short", false, Pbkdf2Params::for_tests())
            .await
            .expect_err("太短的口令必须被拒");
        assert_eq!(count(&p).await, 0, "被拒的条目不得留下半截账号");
    }

    #[tokio::test]
    async fn reconfiguring_never_drops_the_existing_id_or_children() {
        // 改口令时 id 必须不变，否则会话/专家/团队的外键会全断。
        let p = pool().await;
        let params = Pbkdf2Params::for_tests();
        ensure_password_user(&p, "alice", PW, false, params)
            .await
            .expect("首次");
        let before = derive_user_id("alice").expect("id").as_bytes().to_vec();

        sqlx::query(
            "INSERT INTO sessions(user_id,id,kind,room_id,workspace_path,created_at,updated_at,last_active_at)
             VALUES(?,?,'solo','room-1','ws/1',1,1,1)",
        )
        .bind(before.clone())
        .bind(quill_domain::SessionId::from_bytes([7u8; 16]).as_bytes().to_vec())
        .execute(&p)
        .await
        .expect("建会话");

        ensure_password_user(&p, "alice", PW2, false, params)
            .await
            .expect("改口令");

        let still: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE user_id = ?")
            .bind(before)
            .fetch_one(&p)
            .await
            .expect("查会话");
        assert_eq!(still, 1, "改口令不得让已有会话变成孤儿");
    }
}
