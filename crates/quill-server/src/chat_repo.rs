//! 会话（`sessions` 表）的存储口径。
//!
//! **为什么单独成块**：这些 SQL 原先内联在 `api_chat` 的每个处理器里 —— HTTP 层
//! 直接写 SQL，数据模型一改就得在几十个处理器之间找全所有语句。收到这里之后，
//! `api_chat` 只拿**类型化**的结果再拼响应（见 queue Q006）。
//!
//! SQL 写成 `pub const` 常量，便于单测直接断言「隔离谓词在 SQL 里」「软删过滤没漏」，
//! 与 `teams_repo` 同一套口径。

use sqlx::sqlite::SqliteRow;

use crate::db::{col_i64, col_str, storage_error, DbBridge};
use quill_adapters::UserId;
use quill_agent::AgentError;

/// 列表上限。侧栏只展示最近这些条。
pub const LIST_LIMIT: i64 = 100;

/// 会话列表的固定骨架：`hex(id)` 大写，与 SQLite 的 `hex()` 同形态（写路径也用它）。
/// 排除条件（`kind NOT IN`）在末尾前动态拼**占位符** —— 不拼字符串值，那是注入面。
pub const LIST_SQL_HEAD: &str = "SELECT hex(id) AS id, kind, room_id, title, expert_id, \
     provider_id, model, state, message_count, created_at, last_active_at \
     FROM sessions WHERE user_id = ? AND deleted_at IS NULL";

pub const LIST_SQL_TAIL: &str = " ORDER BY last_active_at DESC LIMIT 100";

/// 存在性：软删行算不存在（与「用户看不见」同一种结果）。
pub const EXISTS_SQL: &str = "SELECT count(*) FROM sessions \
     WHERE user_id = ? AND id = ? AND deleted_at IS NULL";

/// 单读软删时间戳。行不存在 → `Ok(None)`；存在且未删 → `Ok(Some(None))`。
pub const DELETED_AT_SQL: &str = "SELECT deleted_at FROM sessions WHERE user_id = ? AND id = ?";

/// 软删。`deleted_at IS NULL` 让重复删除幂等（第二次影响 0 行）。
pub const SOFT_DELETE_SQL: &str = "UPDATE sessions SET deleted_at = ?, updated_at = ? \
     WHERE user_id = ? AND id = ? AND deleted_at IS NULL";

/// 建一条 `solo` 会话。列清单与 `create` / 通道建会话**同一份**。
pub const INSERT_SQL: &str =
    "INSERT INTO sessions(user_id,id,kind,room_id,title,provider_id,model,state,\
     workspace_path,created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)";

/// 挂专家（建会话后单独一步，因为 `expert_id` 允许为空）。
pub const SET_EXPERT_SQL: &str = "UPDATE sessions SET expert_id = ? WHERE user_id = ? AND id = ?";

/// 列表用的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListRow {
    pub id: String,
    pub kind: String,
    pub room_id: String,
    pub title: String,
    pub expert_id: String,
    pub provider_id: String,
    pub model: String,
    pub state: String,
    pub message_count: i64,
    pub created_at: i64,
    pub last_active_at: i64,
}

/// 建会话要写进去的字段。`provider_id` 固定 `local`（与既有行为一致）。
#[derive(Debug, Clone)]
pub struct NewSoloSession {
    pub id: [u8; 16],
    pub room_id: String,
    pub title: String,
    pub model: String,
    pub workspace_path: String,
    pub now: i64,
}

fn row_to_list(r: &SqliteRow) -> SessionListRow {
    SessionListRow {
        id: col_str(r, "id"),
        kind: col_str(r, "kind"),
        room_id: col_str(r, "room_id"),
        title: col_str(r, "title"),
        expert_id: col_str(r, "expert_id"),
        provider_id: col_str(r, "provider_id"),
        model: col_str(r, "model"),
        state: col_str(r, "state"),
        message_count: col_i64(r, "message_count"),
        created_at: col_i64(r, "created_at"),
        last_active_at: col_i64(r, "last_active_at"),
    }
}

/// 列会话。`excluded` 里的 `kind` 不返回；空列表退化成「不过滤」
/// （而不是 `NOT IN ()` 那种语法错）。
pub fn list_sessions(
    db: &DbBridge,
    uid: UserId,
    excluded: Vec<String>,
) -> Result<Vec<SessionListRow>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let placeholders = vec!["?"; excluded.len()].join(", ");
            let sql = format!(
                "{LIST_SQL_HEAD}{}{LIST_SQL_TAIL}",
                if excluded.is_empty() {
                    String::new()
                } else {
                    format!(" AND kind NOT IN ({placeholders})")
                }
            );
            let mut q = sqlx::query(&sql).bind(uid.as_bytes().to_vec());
            for kind in &excluded {
                q = q.bind(kind.as_str());
            }
            let out = q
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("列会话", e))?;
            Ok(out.iter().map(row_to_list).collect())
        })
    })
}

/// 建一条 `solo` 会话。不挂专家（要挂另调 [`set_expert`]）。
///
/// `op` 由调用方给：HTTP 建会话与通道来件建会话失败时的文案不同
/// （「建会话失败」vs「为通道建会话失败」），保留它便于排障。
pub fn insert_solo_session(
    db: &DbBridge,
    uid: UserId,
    s: NewSoloSession,
    op: &'static str,
) -> Result<(), AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(INSERT_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(s.id.to_vec())
                .bind("solo")
                .bind(&s.room_id)
                .bind(&s.title)
                .bind("local")
                .bind(&s.model)
                .bind("IDLE")
                .bind(&s.workspace_path)
                .bind(s.now)
                .bind(s.now)
                .bind(s.now)
                .execute(&pool)
                .await
                .map_err(|e| storage_error(op, e))?;
            Ok(())
        })
    })
}

/// 把会话挂到某个专家上。`expert` 为空串时调用方**不应**调它（等价于不挂）。
pub fn set_expert(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
    expert: String,
) -> Result<(), AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(SET_EXPERT_SQL)
                .bind(expert)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .execute(&pool)
                .await
                .map_err(|e| storage_error("挂专家", e))?;
            Ok(())
        })
    })
}

/// 这条会话在不在（未软删）。
pub fn session_exists(db: &DbBridge, uid: UserId, sid: [u8; 16]) -> Result<bool, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let n: i64 = sqlx::query_scalar(EXISTS_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .fetch_one(&pool)
                .await
                .map_err(|e| storage_error("查会话", e))?;
            Ok(n > 0)
        })
    })
}

/// 读会话的软删时间戳。
pub fn session_deleted_at(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
) -> Result<Option<Option<i64>>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query_scalar(DELETED_AT_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error("查会话", e))
        })
    })
}

/// 软删会话，返回受影响行数（0 = 本来已删或不存在，供幂等判断）。
pub fn soft_delete_session(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
    now: i64,
) -> Result<u64, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let n = sqlx::query(SOFT_DELETE_SQL)
                .bind(now)
                .bind(now)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .execute(&pool)
                .await
                .map_err(|e| storage_error("软删会话", e))?;
            Ok(n.rows_affected())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_sql_filters_soft_deleted_and_scopes_to_the_user() {
        // 这两条是**隔离谓词**：漏掉任一条都会把别人的会话或已删的会话列出来。
        assert!(LIST_SQL_HEAD.contains("user_id = ?"), "必须按用户隔离");
        assert!(LIST_SQL_HEAD.contains("deleted_at IS NULL"), "必须过滤软删");
        assert!(EXISTS_SQL.contains("user_id = ?") && EXISTS_SQL.contains("deleted_at IS NULL"));
    }

    #[test]
    fn soft_delete_is_idempotent_by_predicate() {
        // 幂等靠 SQL 里的 `deleted_at IS NULL`：重复删除影响 0 行，而不是把
        // 删除时间戳刷新成新值。
        assert!(
            SOFT_DELETE_SQL.contains("deleted_at IS NULL"),
            "少了这个谓词，重复删除会覆盖原来的删除时间"
        );
    }

    #[test]
    fn insert_and_set_expert_are_two_statements() {
        assert!(INSERT_SQL.starts_with("INSERT INTO sessions"));
        assert!(SET_EXPERT_SQL.starts_with("UPDATE sessions SET expert_id"));
    }
}
