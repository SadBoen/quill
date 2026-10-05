//! 数据访问层：`users` / `sessions_auth` / `invites` 三张表的 SQL。
//!
//! # 为什么仓储放在本 crate 而不放 quill-store
//!
//! `crates/quill-store/src/lib.rs`（实测 283 行）**只提供连接层**：
//! `configure_pool` / `run_migration` / `in_memory` / `user_dir_name`。
//! 它**没有任何仓储方法** —— 它的文档自己写着「不做业务逻辑、不做 SQL 拼装」。
//! 所以控制面必须自己发 SQL，这也是本 crate 直接依赖 `sqlx` 的原因。
//!
//! # 三条必须守住的口径
//!
//! 1. **时间一律 `INTEGER` 毫秒**，与本 crate [`Clock`](crate::Clock) 的单位一致。
//! 2. **BLOB(16) 与 `UserId` / `SessionId` / `UuidBytes` 直连**：走
//!    `as_bytes()` / `from_bytes()`，不经过字符串（经字符串会引入 hex 解析差异）。
//! 3. **错误映射按语义类别**：唯一约束冲突 → 业务错误，
//!    其余 → `Storage`（附中文 + `quill doctor`）。绝不把 `sqlx::Error` 原样抛给调用方。
//!
//! # 判定用语义码而不是错误文本
//!
//! 唯一约束冲突的识别用 `sqlx::error::ErrorKind::UniqueViolation`
//! （SQLite 扩展码 → 语义枚举），**不**匹配错误字符串 ——
//! 后者会随 sqlx 版本漂移，而漂移的表现是「用户名重复时不再报重复」，
//! 变成一条能写进库的重复账号。
//!
//! # 事务边界
//!
//! 需要原子性的写路径（邀请码兑换：查邀请码 → 建用户 → 记账）走
//! [`insert_user_tx`] / [`find_invite_tx`] / [`consume_invite_tx`]，
//! 它们接收 `&mut Transaction`。
//!
//! ⚠️ **不要在事务里用 `&SqlitePool`**：本项目的池 `max_connections=1`
//! （`quill_store::in_memory` 与 `configure_pool` 的用法），
//! 事务已占住唯一连接，事务内再向池要连接会**死锁**。
//! 这不是理论风险 —— 集成测试用的正是单连接内存池。

use sqlx::error::ErrorKind;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use quill_domain::{SessionId, UserId, UuidBytes};

use crate::error::ControlError;
use crate::password::PasswordDigest;
use crate::user::{UserProfile, UserRole, UserStatus};

/// 本 crate 内部用的事务类型别名。
pub(crate) type Tx<'a> = Transaction<'a, Sqlite>;

/// 数据库错误 → 领域错误。
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
            // ⚠️ 这里**必须**带上 sqlx 的原始消息：`ErrorKind::Other` 是个空壳
            //    （实测踩过：只打 kind 会得到毫无信息量的「（Some(Other)）」，
            //    让人以为是随机故障）。sqlx 的解码/列缺失类消息只含**列名与类型**，
            //    不含行数据，因此不构成凭据泄漏。
            //    反过来，约束类错误走上面的分支，它们不携带任何值。
            detail: format!("{what}：{e}"),
        },
    }
}

/// 判定是否「唯一索引冲突」。
///
/// 抽成函数是为了让判定只有一份 —— 复制两份判定就是将来两份判定漂移的起点。
pub(crate) fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().map(|d| d.kind()) == Some(ErrorKind::UniqueViolation)
}

/// 「读列失败」的错误构造器。
fn invariant(col: &str) -> impl Fn(sqlx::Error) -> ControlError + '_ {
    move |e| ControlError::InvariantBroken {
        detail: format!("读列 {col} 失败：{e}"),
    }
}

// ═══════════════════════ 行 -> 领域类型 ═══════════════════════

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

/// 公开档案的列清单。
///
/// ⚠️ **刻意用真实换行而不是 `\` 续行**：Rust 的 `\` 续行会吃掉换行**和**下一行的
/// 全部前导空白，于是 `"...u.pwd_changed_at\` + `     FROM ..."` 会被拼成
/// `u.pwd_changed_atFROM` —— 一个语法错误，且**只在这条 SQL 被执行时**才暴露。
/// 本文件实测踩过这个坑（症状是 `(code: 1) near "s": syntax error`）。
/// 因此本 crate 的多行 SQL 一律用真实换行：SQL 不在乎换行，但在乎 token 之间有分隔。
const PROFILE_COLS: &str = "id, username, username_norm, display_name, role, status,
     token_epoch, created_at, last_login_at";

/// 从 `users` 行装配公开档案（**不含**任何密码字段）。
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

// ═══════════════════════ users ═══════════════════════

/// 登录路径需要的**全部**凭据字段。
///
/// crate 内私有：它带密码材料，绝不能出现在返回给 HTTP 层的类型里。
#[derive(Debug, Clone)]
pub(crate) struct UserCredentials {
    /// 用户标识。
    pub id: UserId,
    /// 密码摘要。
    pub digest: PasswordDigest,
    /// 账号状态。
    pub status: UserStatus,
    /// 角色（直接取自被验证的那一行，不再二次查询 —— 避免「两次读到的角色不一致」）。
    pub role: UserRole,
    /// 连续失败次数。
    pub login_fail_count: i32,
    /// 锁定截止时刻；`None` = 未锁定。
    pub locked_until_ms: Option<i64>,
}

/// 插入用户的列与值（事务版与非事务版共用同一段 SQL）。
///
/// 见 [`PROFILE_COLS`] 关于「不用 `\` 续行」的说明。
const INSERT_USER_SQL: &str = "INSERT INTO users(
    id, username, username_norm, display_name,
    password_hash, password_salt, password_algo, role,
    status, locale, token_epoch, settings_json,
    pwd_changed_at, login_fail_count, created_at, updated_at
) VALUES(?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, 1, '{}', ?, 0, ?, ?)";

/// 一次用户插入的全部字段。
///
/// 打包成结构体的理由与 `service::NewSession` 相同：
/// `username` 与 `username_norm` 是两个**相邻的 `&str`** 且语义只差规范化 ——
/// 位置传参时传反是类型合法的，且会静默写错一列（登录从此查不到这个用户）。
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

/// 插入用户（**在事务内**执行）。
///
/// 唯一索引冲突时**不直接**断定「用户名重复」——冲突也可能来自主键。
/// 这里在同一事务里回查 `username_norm` 来区分，让报错指向真正的原因。
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

/// 用户名是否已被占用（**在事务内**查询）。
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

/// 插入用户（自带事务：开事务 → 插入 → 提交）。
///
/// 之所以包一层事务而不是直接用池：让 `insert_user_tx` 成为**唯一**的插入实现，
/// 避免「事务版与非事务版两份 SQL」将来漂移。
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
        // ⚠️ 事务随 tx 析构回滚；这里**不**显式 rollback ——
        //    显式 rollback 后再析构是重复动作，且在某些 sqlx 版本上会二次报错。
        Err(e) => Err(e),
    }
}

/// 按规范化用户名读登录所需的凭据。
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
        digest: PasswordDigest {
            algo_tag: algo,
            salt: salt16(&row)?,
            hash: bytes32(&row, "password_hash")?,
        },
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

fn salt16(row: &SqliteRow) -> Result<[u8; 16], ControlError> {
    // 盐长度不对必须判失败：否则 PBKDF2 会拿着错误长度的盐
    // 算出「一个能算出来但永远对不上」的摘要，表现为「所有人都登不上」且无报错。
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

/// 读取一个用户的公开档案。
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

/// 列出全部未删除用户。
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

/// 未删除用户数（`create_first_owner` 的前置条件）。
pub(crate) async fn count_users(pool: &SqlitePool) -> Result<i64, ControlError> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL")
        .fetch_one(pool)
        .await
        .map_err(|e| map_db_error(e, "统计用户数"))
}

/// 记录一次登录成功：清零失败计数、写 `last_login_at`、解锁。
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

/// 记录一次登录失败并更新锁定时刻。
///
/// `locked_until` 为 `None` 表示不加锁。
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

/// 改密码：写新摘要、推进 `pwd_changed_at`、代次 +1、清零失败计数。
///
/// ⚠️ `token_epoch` **不参与**会话校验（`sessions_auth` 没有存它的列）。
/// 它是「凭据代次」计数器，v1 里由 [`UserProfile::token_epoch`] 暴露，
/// 供将来「把 epoch 快照写进 sessions_auth」的 expand-only 迁移使用。
/// 今天真正强制「改密即失效」的是 `pwd_changed_at` + 显式撤销全部会话（两处都有断言）。
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

/// 改账号状态（启用 / 禁用）。
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

// ═══════════════════════ sessions_auth ═══════════════════════

/// 一个会话行 + 它所属用户的必要字段。
#[derive(Debug, Clone)]
pub(crate) struct SessionRow {
    /// 会话标识。
    pub id: SessionId,
    /// 所属用户。
    pub user_id: UserId,
    /// 令牌家族（轮换链的根）。
    pub family_id: SessionId,
    /// 签发时刻。
    pub issued_at_ms: i64,
    /// 过期时刻。
    pub expires_at_ms: i64,
    /// 撤销时刻；`None` = 未撤销。
    pub revoked_at_ms: Option<i64>,
    /// 撤销原因。
    pub revoked_reason: Option<String>,
    /// 所属用户的状态（禁用要立即让会话失效）。
    pub user_status: UserStatus,
    /// 所属用户改密时刻（会话必须不早于它）。
    pub pwd_changed_at_ms: i64,
}

const SESSION_SELECT: &str = "SELECT s.id, s.user_id, s.family_id, s.issued_at, s.expires_at,
     s.revoked_at, s.revoked_reason, u.status, u.pwd_changed_at
     FROM sessions_auth s JOIN users u ON u.id = s.user_id
     WHERE s.token_hash = ? AND u.deleted_at IS NULL";

/// 按令牌摘要查会话。
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

/// 一次会话插入的全部字段。
///
/// 打包理由同 [`NewUser`]：`user_agent` 与 `peer_addr` 是两个相邻的
/// `Option<&str>`，位置传参时传反在类型上完全合法。
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

/// 插入会话。
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

/// 撤销单个会话，返回受影响的行数（0 = 已经处于撤销态）。
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

/// 撤销整个令牌家族（检出重放时连坐）。
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

/// 撤销某用户的全部未撤销会话，返回受影响行数。
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

// ═══════════════════════ invites ═══════════════════════

/// 一个邀请码行。
#[derive(Debug, Clone)]
pub(crate) struct InviteRow {
    /// 邀请码标识（128 位，`invites.id`）。
    pub id: UuidBytes,
    /// 创建者。
    pub created_by: UserId,
    /// 被邀请人将获得的角色。
    pub role: UserRole,
    /// 最大使用次数。
    pub max_uses: i32,
    /// 已使用次数。
    pub used_count: i32,
    /// 过期时刻。
    pub expires_at_ms: i64,
    /// 撤销时刻。
    pub revoked_at_ms: Option<i64>,
    /// 创建时刻。
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

/// 一次邀请码插入的全部字段。打包理由同 [`NewUser`]。
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

/// 插入邀请码。
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

/// 按邀请码摘要查邀请码（**在事务内**执行）。
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

/// 记一次邀请码使用（**在事务内**执行）。
///
/// ⚠️ 调用方必须**先**在同一事务里校验有效性。
/// 本函数刻意**不再带任何条件**（如 `used_count < max_uses`）：
/// 那样会把「校验」与「记账」分裂成两条语句，中间一旦插队就会超出 `max_uses` 发放。
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

/// 列出邀请码（全部，未删除的）。
///
/// ⚠️ **刻意不带 `WHERE created_by = ?`**：归属检查由调用方在**同一事务**里
/// 用 `revoke_invite_owned` 的 `WHERE created_by=?` 完成。
/// 若这里先按创建者过滤、别处再按 id 撤销，就会出现
/// 「查的时候是我的，改的时候归属已变」的窗口。
pub(crate) async fn list_invites(pool: &SqlitePool) -> Result<Vec<InviteRow>, ControlError> {
    let sql = format!("SELECT {INVITE_COLS} FROM invites ORDER BY created_at DESC");
    let rows = sqlx::query(&sql)
        .fetch_all(pool)
        .await
        .map_err(|e| map_db_error(e, "列出邀请码"))?;
    rows.iter().map(invite_from_row).collect()
}

/// 撤销**属于 `created_by`** 的邀请码。
///
/// 归属条件写在 SQL 的 `WHERE` 里而不是先查后改：
/// 后者存在「查的时候还是我的，改的时候归属已变」的窗口。
/// 返回 `false` 表示「不存在 / 不属于你 / 已撤销」三种情况的合并 —— 对外都表现为
/// [`ControlError::InviteUnknown`]，不泄露「这张码存在但不是你签的」。
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
