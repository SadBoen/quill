use sqlx::error::ErrorKind;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use quill_domain::{SessionId, UserId, UuidBytes};

use crate::error::ControlError;
use crate::password::PasswordDigest;
use crate::user::{UserProfile, UserRole, UserStatus};

pub(crate) type Tx<'a> = Transaction<'a, Sqlite>;

pub(crate) fn map_db_error(e: sqlx::Error, what: &str) -> ControlError {
    let kind = e.as_database_error().map(|d| d.kind());
    match kind {
        Some(ErrorKind::UniqueViolation) => ControlError::Storage {
            detail: format!("{what}：唯一索引冲突"),
        },
        Some(ErrorKind::CheckViolation) => ControlError::Storage {
            detail: format!("{what}：被 schema 的 CHECK 约束拒绝（数据形状不合法）"),
        },
        Some(ErrorKind::ForeignKeyViolation) => ControlError::Storage {
            detail: format!("{what}：外键约束不满足（引用了不存在的父行）"),
        },
        Some(ErrorKind::NotNullViolation) => ControlError::Storage {
            detail: format!("{what}：NOT NULL 约束不满足"),
        },
        _ => ControlError::Storage {
            detail: format!("{what}：{e}"),
        },
    }
}

pub(crate) fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().map(|d| d.kind()) == Some(ErrorKind::UniqueViolation)
}

fn invariant(col: &str) -> impl Fn(sqlx::Error) -> ControlError + '_ {
    move |e| ControlError::InvariantBroken {
        detail: format!("读列 {col} 失败：{e}"),
    }
}

fn bytes16(row: &SqliteRow, col: &str) -> Result<[u8; 16], ControlError> {
    let v: Vec<u8> = row.try_get(col).map_err(invariant(col))?;
    if v.len() != 16 {
        return Err(ControlError::InvariantBroken {
            detail: format!("列 {col} 期望 16 字节，实际 {} 字节", v.len()),
        });
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&v);
    Ok(out)
}

fn bytes32(row: &SqliteRow, col: &str) -> Result<[u8; 32], ControlError> {
    let v: Vec<u8> = row.try_get(col).map_err(invariant(col))?;
    if v.len() != 32 {
        return Err(ControlError::InvariantBroken {
            detail: format!("列 {col} 期望 32 字节，实际 {} 字节", v.len()),
        });
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    Ok(out)
}

const PROFILE_COLS: &str = "id, username, username_norm, display_name, role, status,
     token_epoch, created_at, last_login_at";

fn profile_from_row(row: &SqliteRow) -> Result<UserProfile, ControlError> {
    let role_s: String = row.try_get("role").map_err(invariant("role"))?;
    let status_s: String = row.try_get("status").map_err(invariant("status"))?;
    Ok(UserProfile {
        id: UserId::from_bytes(bytes16(row, "id")?),
        username: row.try_get("username").map_err(invariant("username"))?,
        username_norm: row
            .try_get("username_norm")
            .map_err(invariant("username_norm"))?,
        display_name: row
            .try_get("display_name")
            .map_err(invariant("display_name"))?,
        role: UserRole::parse(&role_s)?,
        status: UserStatus::parse(&status_s)?,
        token_epoch: row
            .try_get("token_epoch")
            .map_err(invariant("token_epoch"))?,
        created_at_ms: row.try_get("created_at").map_err(invariant("created_at"))?,
        last_login_at_ms: row
            .try_get("last_login_at")
            .map_err(invariant("last_login_at"))?,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct UserCredentials {
    pub id: UserId,

    pub digest: PasswordDigest,

    pub status: UserStatus,

    pub role: UserRole,

    pub login_fail_count: i32,

    pub locked_until_ms: Option<i64>,
}

const INSERT_USER_SQL: &str = "INSERT INTO users(
    id, username, username_norm, display_name,
    password_hash, password_salt, password_algo, role,
    status, locale, token_epoch, settings_json,
    pwd_changed_at, login_fail_count, created_at, updated_at
) VALUES(?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, 1, '{}', ?, 0, ?, ?)";

#[derive(Debug, Clone, Copy)]
pub(crate) struct NewUser<'a> {
    pub id: &'a UserId,
    pub username: &'a str,
    pub username_norm: &'a str,
    pub display_name: &'a str,
    pub digest: &'a PasswordDigest,
    pub role: UserRole,
    pub locale: &'a str,
    pub now_ms: i64,
}

pub(crate) async fn insert_user_tx(tx: &mut Tx<'_>, u: NewUser<'_>) -> Result<(), ControlError> {
    let NewUser {
        id,
        username,
        username_norm,
        display_name,
        digest,
        role,
        locale,
        now_ms,
    } = u;
    let res = sqlx::query(INSERT_USER_SQL)
        .bind(id.as_bytes().as_slice())
        .bind(username)
        .bind(username_norm)
        .bind(display_name)
        .bind(digest.hash.to_vec())
        .bind(digest.salt.to_vec())
        .bind(&digest.algo_tag)
        .bind(role.as_str())
        .bind(locale)
        .bind(now_ms)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut **tx)
        .await;
    match res {
        Ok(_) => Ok(()),
        Err(e) if is_unique_violation(&e) => {
            if username_exists_tx(tx, username_norm).await.unwrap_or(false) {
                Err(ControlError::UsernameTaken {
                    username_norm: username_norm.to_string(),
                })
            } else {
                Err(ControlError::InvariantBroken {
                    detail: format!(
                        "创建用户时撞了唯一索引，但 username_norm={username_norm:?} 并不存在 \
                         —— 冲突来自主键 users.id（标识碰撞）"
                    ),
                })
            }
        }
        Err(e) => Err(map_db_error(e, "创建用户")),
    }
}

pub(crate) async fn username_exists_tx(
    tx: &mut Tx<'_>,
    username_norm: &str,
) -> Result<bool, ControlError> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE username_norm=? AND deleted_at IS NULL",
    )
    .bind(username_norm)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| map_db_error(e, "查用户名占用"))?;
    Ok(n > 0)
}

pub(crate) async fn insert_user(pool: &SqlitePool, u: NewUser<'_>) -> Result<(), ControlError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| map_db_error(e, "开启建号事务"))?;
    let r = insert_user_tx(&mut tx, u).await;
    match r {
        Ok(()) => tx
            .commit()
            .await
            .map_err(|e| map_db_error(e, "提交建号事务")),

        Err(e) => Err(e),
    }
}

pub(crate) async fn find_credentials(
    pool: &SqlitePool,
    username_norm: &str,
) -> Result<Option<UserCredentials>, ControlError> {
    let row = sqlx::query(
        "SELECT id, password_hash, password_salt, password_algo, status, role,
         login_fail_count, locked_until FROM users
         WHERE username_norm = ? AND deleted_at IS NULL",
    )
    .bind(username_norm)
    .fetch_optional(pool)
    .await
    .map_err(|e| map_db_error(e, "按用户名读凭据"))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let algo: String = row
        .try_get("password_algo")
        .map_err(invariant("password_algo"))?;
    let status_s: String = row.try_get("status").map_err(invariant("status"))?;
    let role_s: String = row.try_get("role").map_err(invariant("role"))?;
    Ok(Some(UserCredentials {
        id: UserId::from_bytes(bytes16(&row, "id")?),
        digest: digest_from_row(&row, &algo)?,
        status: UserStatus::parse(&status_s)?,
        role: UserRole::parse(&role_s)?,
        login_fail_count: row
            .try_get("login_fail_count")
            .map_err(invariant("login_fail_count"))?,
        locked_until_ms: row
            .try_get("locked_until")
            .map_err(invariant("locked_until"))?,
    }))
}

/// 从行里取出口令摘要。
///
/// **必须先看 `password_algo` 再看字节长度。** `token-only` 账号（只靠
/// Bearer 令牌进来的那些，`QUILL_TOKENS` 引导时建的）存的是哨兵字符串
/// `token-only` / `token-only-no-password`，长度分别是 10 和 26 字节 ——
/// 它们本来就不是摘要，压根不该按 16/32 字节去校验。
///
/// 之前这里无条件要求 16 字节 salt，于是**任何 token-only 账号一用口令登录
/// 就撞不变量**：不是 401「凭据无效」，而是 500「内部不变量被破坏」。
/// 那等于把一个正常的登录失败报成服务端故障，还顺手把它计进限流额度里
/// 当成「一次失败尝试」——双重错误。
fn digest_from_row(row: &SqliteRow, algo: &str) -> Result<PasswordDigest, ControlError> {
    if algo == crate::bootstrap::TOKEN_ONLY_ALGO {
        // 零值摘要。`verify_stored` 会先 `parse_algo("token-only")` 失败，
        // 直接返回 false（见 password.rs），所以这两个字节数组永远不会被
        // 拿去参与比较——它们只是为了让结构体有个合法值。
        return Ok(PasswordDigest {
            algo_tag: algo.to_string(),
            salt: [0u8; 16],
            hash: [0u8; 32],
        });
    }
    Ok(PasswordDigest {
        algo_tag: algo.to_string(),
        salt: salt16(row)?,
        hash: bytes32(row, "password_hash")?,
    })
}

fn salt16(row: &SqliteRow) -> Result<[u8; 16], ControlError> {
    let v: Vec<u8> = row
        .try_get("password_salt")
        .map_err(invariant("password_salt"))?;
    if v.len() != 16 {
        return Err(ControlError::InvariantBroken {
            detail: format!("password_salt 期望 16 字节，实际 {} 字节", v.len()),
        });
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&v);
    Ok(out)
}

pub(crate) async fn find_profile(
    pool: &SqlitePool,
    id: &UserId,
) -> Result<Option<UserProfile>, ControlError> {
    let sql = format!("SELECT {PROFILE_COLS} FROM users WHERE id=? AND deleted_at IS NULL");
    let row = sqlx::query(&sql)
        .bind(id.as_bytes().as_slice())
        .fetch_optional(pool)
        .await
        .map_err(|e| map_db_error(e, "按 id 读用户"))?;
    row.as_ref().map(profile_from_row).transpose()
}

pub(crate) async fn list_profiles(pool: &SqlitePool) -> Result<Vec<UserProfile>, ControlError> {
    let sql = format!(
        "SELECT {PROFILE_COLS} FROM users WHERE deleted_at IS NULL
         ORDER BY created_at ASC, id ASC"
    );
    let rows = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| map_db_error(e, "列出用户"))?;
    rows.iter().map(profile_from_row).collect()
}

pub(crate) async fn count_users(pool: &SqlitePool) -> Result<i64, ControlError> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL")
        .fetch_one(pool)
        .await
        .map_err(|e| map_db_error(e, "统计用户数"))
}

pub(crate) async fn record_login_success(
    pool: &SqlitePool,
    id: &UserId,
    now_ms: i64,
) -> Result<(), ControlError> {
    sqlx::query(
        "UPDATE users SET last_login_at = ?, login_fail_count = 0, locked_until = NULL,
         updated_at = ? WHERE id = ?",
    )
    .bind(now_ms)
    .bind(now_ms)
    .bind(id.as_bytes().as_slice())
    .execute(pool)
    .await
    .map_err(|e| map_db_error(e, "记录登录成功"))?;
    Ok(())
}

pub(crate) async fn record_login_failure(
    pool: &SqlitePool,
    id: &UserId,
    fail_count: i32,
    locked_until: Option<i64>,
    now_ms: i64,
) -> Result<(), ControlError> {
    sqlx::query("UPDATE users SET login_fail_count=?, locked_until=?, updated_at=? WHERE id=?")
        .bind(fail_count)
        .bind(locked_until)
        .bind(now_ms)
        .bind(id.as_bytes().as_slice())
        .execute(pool)
        .await
        .map_err(|e| map_db_error(e, "记录登录失败"))?;
    Ok(())
}

pub(crate) async fn replace_password(
    pool: &SqlitePool,
    id: &UserId,
    digest: &PasswordDigest,
    now_ms: i64,
) -> Result<(), ControlError> {
    sqlx::query(
        "UPDATE users SET password_hash = ?, password_salt = ?, password_algo = ?,
         pwd_changed_at = ?, token_epoch = token_epoch + 1,
         login_fail_count = 0, locked_until = NULL, updated_at = ?
         WHERE id = ?",
    )
    .bind(digest.hash.to_vec())
    .bind(digest.salt.to_vec())
    .bind(&digest.algo_tag)
    .bind(now_ms)
    .bind(now_ms)
    .bind(id.as_bytes().as_slice())
    .execute(pool)
    .await
    .map_err(|e| map_db_error(e, "修改密码"))?;
    Ok(())
}

pub(crate) async fn set_status(
    pool: &SqlitePool,
    id: &UserId,
    status: UserStatus,
    now_ms: i64,
) -> Result<(), ControlError> {
    sqlx::query("UPDATE users SET status=?, updated_at=? WHERE id=? AND deleted_at IS NULL")
        .bind(status.as_str())
        .bind(now_ms)
        .bind(id.as_bytes().as_slice())
        .execute(pool)
        .await
        .map_err(|e| map_db_error(e, "修改账号状态"))?;
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct SessionRow {
    pub id: SessionId,

    pub user_id: UserId,

    pub family_id: SessionId,

    pub issued_at_ms: i64,

    pub expires_at_ms: i64,

    pub revoked_at_ms: Option<i64>,

    pub revoked_reason: Option<String>,

    pub user_status: UserStatus,

    pub pwd_changed_at_ms: i64,
}

const SESSION_SELECT: &str = "SELECT s.id, s.user_id, s.family_id, s.issued_at, s.expires_at,
     s.revoked_at, s.revoked_reason, u.status, u.pwd_changed_at
     FROM sessions_auth s JOIN users u ON u.id = s.user_id
     WHERE s.token_hash = ? AND u.deleted_at IS NULL";

pub(crate) async fn find_session_by_digest(
    pool: &SqlitePool,
    digest: &[u8; 32],
) -> Result<Option<SessionRow>, ControlError> {
    let row = sqlx::query(SESSION_SELECT)
        .bind(digest.to_vec())
        .fetch_optional(pool)
        .await
        .map_err(|e| map_db_error(e, "按令牌摘要查会话"))?;
    row.as_ref().map(session_from_row).transpose()
}

fn session_from_row(row: &SqliteRow) -> Result<SessionRow, ControlError> {
    let status_s: String = row.try_get("status").map_err(invariant("status"))?;
    Ok(SessionRow {
        id: SessionId::from_bytes(bytes16(row, "id")?),
        user_id: UserId::from_bytes(bytes16(row, "user_id")?),
        family_id: SessionId::from_bytes(bytes16(row, "family_id")?),
        issued_at_ms: row.try_get("issued_at").map_err(invariant("issued_at"))?,
        expires_at_ms: row.try_get("expires_at").map_err(invariant("expires_at"))?,
        revoked_at_ms: row.try_get("revoked_at").map_err(invariant("revoked_at"))?,
        revoked_reason: row
            .try_get("revoked_reason")
            .map_err(invariant("revoked_reason"))?,
        user_status: UserStatus::parse(&status_s)?,
        pwd_changed_at_ms: row
            .try_get("pwd_changed_at")
            .map_err(invariant("pwd_changed_at"))?,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NewSession<'a> {
    pub id: &'a SessionId,
    pub user_id: &'a UserId,
    pub token_digest: &'a [u8; 32],
    pub family_id: &'a SessionId,
    pub parent_id: Option<&'a SessionId>,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub user_agent: Option<&'a str>,
    pub peer_addr: Option<&'a str>,
}

pub(crate) async fn insert_session(
    pool: &SqlitePool,
    s: NewSession<'_>,
) -> Result<(), ControlError> {
    let NewSession {
        id,
        user_id,
        token_digest,
        family_id,
        parent_id,
        issued_at_ms,
        expires_at_ms,
        user_agent,
        peer_addr,
    } = s;
    let sql = "INSERT INTO sessions_auth(
        id, user_id, token_hash, family_id, parent_id,
        issued_at, expires_at, user_agent, peer_addr, created_at
    ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
    sqlx::query(sql)
        .bind(id.as_bytes().as_slice())
        .bind(user_id.as_bytes().as_slice())
        .bind(token_digest.to_vec())
        .bind(family_id.as_bytes().as_slice())
        .bind(parent_id.map(|p| p.as_bytes().to_vec()))
        .bind(issued_at_ms)
        .bind(expires_at_ms)
        .bind(user_agent)
        .bind(peer_addr)
        .bind(issued_at_ms)
        .execute(pool)
        .await
        .map_err(|e| map_db_error(e, "签发会话"))?;
    Ok(())
}

pub(crate) async fn revoke_session(
    pool: &SqlitePool,
    id: &SessionId,
    now_ms: i64,
    reason: &'static str,
) -> Result<u64, ControlError> {
    let res = sqlx::query(
        "UPDATE sessions_auth SET revoked_at = ?, revoked_reason = ?
         WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(now_ms)
    .bind(reason)
    .bind(id.as_bytes().as_slice())
    .execute(pool)
    .await
    .map_err(|e| map_db_error(e, "撤销会话"))?;
    Ok(res.rows_affected())
}

pub(crate) async fn revoke_family(
    pool: &SqlitePool,
    family_id: &SessionId,
    now_ms: i64,
    reason: &'static str,
) -> Result<u64, ControlError> {
    let res = sqlx::query(
        "UPDATE sessions_auth SET revoked_at = ?, revoked_reason = ?
         WHERE family_id = ? AND revoked_at IS NULL",
    )
    .bind(now_ms)
    .bind(reason)
    .bind(family_id.as_bytes().as_slice())
    .execute(pool)
    .await
    .map_err(|e| map_db_error(e, "撤销令牌家族"))?;
    Ok(res.rows_affected())
}

pub(crate) async fn revoke_user_sessions(
    pool: &SqlitePool,
    user_id: &UserId,
    now_ms: i64,
    reason: &'static str,
) -> Result<u64, ControlError> {
    let res = sqlx::query(
        "UPDATE sessions_auth SET revoked_at = ?, revoked_reason = ?
         WHERE user_id = ? AND revoked_at IS NULL",
    )
    .bind(now_ms)
    .bind(reason)
    .bind(user_id.as_bytes().as_slice())
    .execute(pool)
    .await
    .map_err(|e| map_db_error(e, "撤销用户全部会话"))?;
    Ok(res.rows_affected())
}

#[derive(Debug, Clone)]
pub(crate) struct InviteRow {
    pub id: UuidBytes,

    pub created_by: UserId,

    pub role: UserRole,

    pub max_uses: i32,

    pub used_count: i32,

    pub expires_at_ms: i64,

    pub revoked_at_ms: Option<i64>,

    pub created_at_ms: i64,
}

const INVITE_COLS: &str = "id,created_by,role,max_uses,used_count,expires_at,revoked_at,created_at";

fn invite_from_row(row: &SqliteRow) -> Result<InviteRow, ControlError> {
    let role_s: String = row.try_get("role").map_err(invariant("role"))?;
    Ok(InviteRow {
        id: UuidBytes::from_bytes(bytes16(row, "id")?),
        created_by: UserId::from_bytes(bytes16(row, "created_by")?),
        role: UserRole::parse(&role_s)?,
        max_uses: row.try_get("max_uses").map_err(invariant("max_uses"))?,
        used_count: row.try_get("used_count").map_err(invariant("used_count"))?,
        expires_at_ms: row.try_get("expires_at").map_err(invariant("expires_at"))?,
        revoked_at_ms: row.try_get("revoked_at").map_err(invariant("revoked_at"))?,
        created_at_ms: row.try_get("created_at").map_err(invariant("created_at"))?,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NewInvite<'a> {
    pub id: &'a UuidBytes,
    pub code_digest: &'a [u8; 32],
    pub created_by: &'a UserId,
    pub role: UserRole,
    pub max_uses: i32,
    pub expires_at_ms: i64,
    pub created_at_ms: i64,
}

pub(crate) async fn insert_invite(pool: &SqlitePool, v: NewInvite<'_>) -> Result<(), ControlError> {
    let NewInvite {
        id,
        code_digest,
        created_by,
        role,
        max_uses,
        expires_at_ms,
        created_at_ms,
    } = v;
    let sql = "INSERT INTO invites(
        id, code_hash, created_by, role, max_uses, used_count, expires_at, created_at
    ) VALUES(?, ?, ?, ?, ?, 0, ?, ?)";
    sqlx::query(sql)
        .bind(id.as_bytes().as_slice())
        .bind(code_digest.to_vec())
        .bind(created_by.as_bytes().as_slice())
        .bind(role.as_str())
        .bind(max_uses)
        .bind(expires_at_ms)
        .bind(created_at_ms)
        .execute(pool)
        .await
        .map_err(|e| map_db_error(e, "创建邀请码"))?;
    Ok(())
}

pub(crate) async fn find_invite_tx(
    tx: &mut Tx<'_>,
    digest: &[u8; 32],
) -> Result<Option<InviteRow>, ControlError> {
    let sql = format!("SELECT {INVITE_COLS} FROM invites WHERE code_hash=?");
    let row = sqlx::query(&sql)
        .bind(digest.to_vec())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| map_db_error(e, "按摘要查邀请码"))?;
    row.as_ref().map(invite_from_row).transpose()
}

pub(crate) async fn consume_invite_tx(
    tx: &mut Tx<'_>,
    id: &UuidBytes,
    accepted_by: &UserId,
    now_ms: i64,
) -> Result<(), ControlError> {
    let res = sqlx::query(
        "UPDATE invites SET used_count = used_count + 1,
         accepted_by = COALESCE(accepted_by, ?),
         accepted_at = COALESCE(accepted_at, ?)
         WHERE id = ?",
    )
    .bind(accepted_by.as_bytes().as_slice())
    .bind(now_ms)
    .bind(id.as_bytes().as_slice())
    .execute(&mut **tx)
    .await
    .map_err(|e| map_db_error(e, "记账邀请码使用"))?;
    if res.rows_affected() != 1 {
        return Err(ControlError::InvariantBroken {
            detail: "记账邀请码使用：受影响行数不是 1 —— 邀请码行在事务内消失了".to_string(),
        });
    }
    Ok(())
}

pub(crate) async fn list_invites(pool: &SqlitePool) -> Result<Vec<InviteRow>, ControlError> {
    let sql = format!("SELECT {INVITE_COLS} FROM invites ORDER BY created_at DESC");
    let rows = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| map_db_error(e, "列出邀请码"))?;
    rows.iter().map(invite_from_row).collect()
}

pub(crate) async fn revoke_invite_owned(
    pool: &SqlitePool,
    id: &UuidBytes,
    created_by: &UserId,
    now_ms: i64,
) -> Result<bool, ControlError> {
    let res = sqlx::query(
        "UPDATE invites SET revoked_at=? WHERE id=? AND created_by=? AND revoked_at IS NULL",
    )
    .bind(now_ms)
    .bind(id.as_bytes().as_slice())
    .bind(created_by.as_bytes().as_slice())
    .execute(pool)
    .await
    .map_err(|e| map_db_error(e, "撤销邀请码"))?;
    Ok(res.rows_affected() > 0)
}
