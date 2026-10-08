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

// ---------------------------------------------------------------- 消息（messages）

/// 按 `seq` 升序列出某会话的全部消息。
pub const MESSAGES_SQL: &str = "SELECT hex(id) AS id, seq, role, status, content, reasoning, \
     input_tokens, output_tokens, turn_ms, error_code, created_at \
     FROM messages WHERE user_id = ? AND session_id = ? ORDER BY seq";

/// 最后一条 **assistant** 且 `input_tokens > 0` 的入参大小 —— 上下文占用用它。
/// 取不到就是 `None`（还没跟模型说过话），**不是 0**。
pub const LAST_INPUT_TOKENS_SQL: &str = "SELECT input_tokens FROM messages \
     WHERE user_id = ? AND session_id = ? AND role = 'assistant' \
     AND input_tokens > 0 ORDER BY seq DESC LIMIT 1";

/// 对话段的正文（只取 user/assistant，工具与系统消息不算「对话」）。
pub const DIALOG_CONTENTS_SQL: &str = "SELECT content FROM messages \
     WHERE user_id = ? AND session_id = ? AND role IN ('user','assistant')";

/// 一条消息。字段与 `MESSAGES_SQL` 的列一一对应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRow {
    pub id: String,
    pub seq: i64,
    pub role: String,
    pub status: String,
    pub content: String,
    pub reasoning: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub turn_ms: i64,
    pub error_code: String,
    pub created_at: i64,
}

/// 列某会话的全部消息。
pub fn list_messages(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
) -> Result<Vec<MessageRow>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let out = sqlx::query(MESSAGES_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("列消息", e))?;
            Ok(out
                .iter()
                .map(|r| MessageRow {
                    id: col_str(r, "id"),
                    seq: col_i64(r, "seq"),
                    role: col_str(r, "role"),
                    status: col_str(r, "status"),
                    content: col_str(r, "content"),
                    reasoning: col_str(r, "reasoning"),
                    input_tokens: col_i64(r, "input_tokens"),
                    output_tokens: col_i64(r, "output_tokens"),
                    turn_ms: col_i64(r, "turn_ms"),
                    error_code: col_str(r, "error_code"),
                    created_at: col_i64(r, "created_at"),
                })
                .collect())
        })
    })
}

/// 上下文占用的实测值：最后一条 assistant 的 `input_tokens`。
pub fn last_assistant_input_tokens(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
) -> Result<Option<i64>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query_scalar(LAST_INPUT_TOKENS_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error("读上下文占用", e))
        })
    })
}

/// 对话段（user/assistant）的字符总数。**是字符数不是 token 数** —— quill 没有
/// 分词器，把字符说成 token 就是凭空造数字（见 `session_metrics`）。
pub fn dialog_content_chars(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
) -> Result<usize, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let texts: Vec<String> = sqlx::query_scalar(DIALOG_CONTENTS_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("读对话字符数", e))?;
            Ok(texts.iter().map(|t| t.chars().count()).sum())
        })
    })
}

// ---------------------------------------------------------------- 用量（metrics / usage）

/// 会话级指标要的消息列（`GET /api/sessions/{id}/metrics`）。
pub const USAGE_ROWS_SQL: &str = "SELECT role, input_tokens, output_tokens, turn_ms, \
     cache_read_tokens FROM messages WHERE user_id = ? AND session_id = ? ORDER BY seq";

/// `GET /api/usage` 的会话清单（多取一条由调用方判断截断）。
pub const USAGE_SESSIONS_SQL: &str = "SELECT id, title, expert_id, last_active_at FROM sessions \
     WHERE user_id = ? AND deleted_at IS NULL ORDER BY last_active_at DESC LIMIT ?";

/// 一条消息的用量**原始列**。怎么解释（谁算 user、缺一列算不算）在
/// `session_metrics` —— 这里只搬数据，不带口径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageUsageRow {
    pub role: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// `None` = 没量到，**不是 0**。
    pub turn_ms: Option<i64>,
    /// `None` = 模型端没上报缓存读，**不是 0**。
    pub cache_read_tokens: Option<i64>,
}

fn row_to_usage(r: &SqliteRow) -> MessageUsageRow {
    MessageUsageRow {
        role: col_str(r, "role"),
        input_tokens: col_i64(r, "input_tokens"),
        output_tokens: col_i64(r, "output_tokens"),
        // 可空列：取不到与 NULL 是同一件事（没上报），不是 0。
        turn_ms: sqlx::Row::try_get::<Option<i64>, _>(r, "turn_ms")
            .ok()
            .flatten(),
        cache_read_tokens: sqlx::Row::try_get::<Option<i64>, _>(r, "cache_read_tokens")
            .ok()
            .flatten(),
    }
}

/// `GET /api/usage` 的会话行。`id` 是**裸 blob**（不是 hex）：这一列要拿去
/// 喂给下面的 `IN (?, ?, …)`，转成 hex 反而要多转回来一次。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSessionRow {
    pub id: Vec<u8>,
    pub title: String,
    pub expert_id: Option<String>,
    pub last_active_at: i64,
}

/// 单会话的用量行，按 `seq` 升序。
pub fn usage_rows(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
) -> Result<Vec<MessageUsageRow>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let out = sqlx::query(USAGE_ROWS_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("读会话统计", e))?;
            Ok(out.iter().map(row_to_usage).collect())
        })
    })
}

/// 用量页的会话清单（`limit` 由调用方给：多取一条用来判断有没有被截断）。
pub fn usage_sessions(
    db: &DbBridge,
    uid: UserId,
    limit: i64,
) -> Result<Vec<UsageSessionRow>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let rows = sqlx::query(USAGE_SESSIONS_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(limit)
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("列会话", e))?;
            Ok(rows
                .iter()
                .map(|r| {
                    let expert_id = col_str(r, "expert_id");
                    UsageSessionRow {
                        id: sqlx::Row::try_get::<Vec<u8>, _>(r, "id").unwrap_or_default(),
                        title: col_str(r, "title"),
                        // 空串与 NULL 在这里是同一件事：没挂专家。
                        expert_id: if expert_id.is_empty() {
                            None
                        } else {
                            Some(expert_id)
                        },
                        last_active_at: col_i64(r, "last_active_at"),
                    }
                })
                .collect())
        })
    })
}

/// 一批会话的用量行（`(会话 id, 行)`）。`ids` 为空时返回空 ——
/// `IN ()` 是语法错，不能把空集拼进 SQL。
pub fn usage_rows_for_sessions(
    db: &DbBridge,
    uid: UserId,
    ids: Vec<Vec<u8>>,
) -> Result<Vec<(Vec<u8>, MessageUsageRow)>, AgentError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let placeholders = vec!["?"; ids.len()].join(",");
            let sql = format!(
                "SELECT session_id, role, input_tokens, output_tokens, turn_ms, \
                 cache_read_tokens FROM messages WHERE user_id = ? \
                 AND session_id IN ({placeholders}) ORDER BY seq"
            );
            let mut q = sqlx::query(&sql).bind(uid.as_bytes().to_vec());
            for id in &ids {
                q = q.bind(id.clone());
            }
            let rows = q
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("读用量", e))?;
            Ok(rows
                .iter()
                .map(|r| {
                    (
                        sqlx::Row::try_get::<Vec<u8>, _>(r, "session_id").unwrap_or_default(),
                        row_to_usage(r),
                    )
                })
                .collect())
        })
    })
}

// ---------------------------------------------------------------- 发送路径（prepare_turn）

/// 会话绑定的专家标识（未软删才看得到；`expert_id` 本身允许为 NULL）。
pub const SESSION_EXPERT_ID_SQL: &str = "SELECT expert_id FROM sessions \
     WHERE user_id = ? AND id = ? AND deleted_at IS NULL";

/// 读会话绑定的专家标识。行不存在 / 列是 NULL → `Ok(None)`。
pub fn session_expert_id(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
) -> Result<Option<String>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let v: Option<Option<String>> = sqlx::query_scalar(SESSION_EXPERT_ID_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error("读会话绑定的专家", e))?;
            Ok(v.flatten())
        })
    })
}

/// 对话历史（只取 `complete` 的 user/assistant）。
///
/// SQL 用 `seq DESC LIMIT ?` 取**最近**若干条，返回前反转成升序 ——
/// 调用方拿到的是「按时间从旧到新」，与喂给模型的顺序一致。
pub const HISTORY_SQL: &str = "SELECT role, content FROM messages \
     WHERE user_id = ? AND session_id = ? AND status = 'complete' \
     AND role IN ('user','assistant') ORDER BY seq DESC LIMIT ?";

/// 最近 `limit` 条对话历史，**升序**返回。
pub fn history_rows(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
    limit: i64,
) -> Result<Vec<(String, String)>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let out = sqlx::query(HISTORY_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .bind(limit)
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("读历史", e))?;
            let mut rows: Vec<(String, String)> = out
                .iter()
                .map(|r| (col_str(r, "role"), col_str(r, "content")))
                .collect();
            rows.reverse();
            Ok(rows)
        })
    })
}

/// 下一个可用 `seq`（`MAX(seq) + 1`；空会话从 1 开始）。
pub const NEXT_SEQ_SQL: &str =
    "SELECT COALESCE(MAX(seq),0) FROM messages WHERE user_id = ? AND session_id = ?";

pub fn next_seq(db: &DbBridge, uid: UserId, sid: [u8; 16]) -> Result<i64, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let m: i64 = sqlx::query_scalar(NEXT_SEQ_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .fetch_one(&pool)
                .await
                .map_err(|e| storage_error("取序号", e))?;
            Ok(m + 1)
        })
    })
}

/// 要写进 `messages` 的一行。列顺序与 [`INSERT_MESSAGE_SQL`] 一一对应。
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub id: [u8; 16],
    pub seq: i64,
    pub role: String,
    pub status: String,
    pub content: String,
    pub reasoning: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// `None` = 模型端没上报，不是 0（见 migration 0008）。
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub turn_ms: Option<i64>,
    pub created_at: i64,
}

pub const INSERT_MESSAGE_SQL: &str =
    "INSERT INTO messages(user_id,id,session_id,seq,role,status,content,reasoning,\
     input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,turn_ms,created_at) \
     VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)";

/// 写一条消息，返回 `created_at`（调用方要用它拼响应）。
pub fn insert_message(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
    m: NewMessage,
) -> Result<i64, AgentError> {
    let created_at = m.created_at;
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(INSERT_MESSAGE_SQL)
                .bind(uid.as_bytes().to_vec())
                .bind(m.id.to_vec())
                .bind(sid.to_vec())
                .bind(m.seq)
                .bind(&m.role)
                .bind(&m.status)
                .bind(&m.content)
                .bind(&m.reasoning)
                .bind(m.input_tokens)
                .bind(m.output_tokens)
                .bind(m.cache_read_tokens)
                .bind(m.cache_write_tokens)
                .bind(m.turn_ms)
                .bind(m.created_at)
                .execute(&pool)
                .await
                .map_err(|e| storage_error("存消息", e))?;
            Ok(created_at)
        })
    })
}

/// 一轮结束后推进会话：`next_seq`、`message_count += 2`（一问一答）、
/// 累计 token、刷新活跃时间。
pub const TOUCH_SESSION_SQL: &str =
    "UPDATE sessions SET next_seq = ?, message_count = message_count + 2, \
     input_tokens = input_tokens + ?, output_tokens = output_tokens + ?, \
     last_active_at = ?, updated_at = ? WHERE user_id = ? AND id = ?";

pub fn touch_session(
    db: &DbBridge,
    uid: UserId,
    sid: [u8; 16],
    next_seq: i64,
    input: Option<u32>,
    output: Option<u32>,
    now: i64,
) -> Result<(), AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(TOUCH_SESSION_SQL)
                .bind(next_seq)
                .bind(i64::from(input.unwrap_or(0)))
                .bind(i64::from(output.unwrap_or(0)))
                .bind(now)
                .bind(now)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .execute(&pool)
                .await
                .map_err(|e| storage_error("更新会话", e))?;
            Ok(())
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

    #[test]
    fn messages_are_scoped_by_user_and_session_and_ordered() {
        assert!(MESSAGES_SQL.contains("user_id = ?"), "必须按用户隔离");
        assert!(MESSAGES_SQL.contains("session_id = ?"), "必须按会话隔离");
        assert!(
            MESSAGES_SQL.contains("ORDER BY seq"),
            "顺序由 seq 决定，不能靠插入顺序"
        );
    }

    #[test]
    fn context_usage_takes_the_last_reported_assistant_input() {
        // 三条一起构成「实测值」的定义，少一条就会算错上下文占用：
        // 只看 assistant（用户那条没有入参）、只要 > 0（0 是没报，不是真的 0）、
        // 取最后一条（seq DESC LIMIT 1）。
        assert!(LAST_INPUT_TOKENS_SQL.contains("role = 'assistant'"));
        assert!(LAST_INPUT_TOKENS_SQL.contains("input_tokens > 0"));
        assert!(LAST_INPUT_TOKENS_SQL.contains("ORDER BY seq DESC LIMIT 1"));
    }

    #[test]
    fn dialog_chars_only_count_user_and_assistant() {
        assert!(
            DIALOG_CONTENTS_SQL.contains("role IN ('user','assistant')"),
            "工具/系统消息不算「对话段」，算进去会让上下文分段虚高"
        );
    }

    #[test]
    fn usage_rows_are_scoped_and_ordered_like_metrics_expects() {
        // 少任何一条都会让「指标」变成别人的或乱序的：
        // 隔离谓词两条、顺序一条。
        assert!(USAGE_ROWS_SQL.contains("user_id = ?"));
        assert!(USAGE_ROWS_SQL.contains("session_id = ?"));
        assert!(USAGE_ROWS_SQL.contains("ORDER BY seq"));
    }

    #[test]
    fn usage_sessions_hide_soft_deleted_and_bound_the_window() {
        assert!(USAGE_SESSIONS_SQL.contains("user_id = ?"));
        assert!(USAGE_SESSIONS_SQL.contains("deleted_at IS NULL"));
        assert!(
            USAGE_SESSIONS_SQL.contains("LIMIT ?"),
            "用量页的会话窗口必须由调用方给；写死或没有上限都会让接口随数据增长"
        );
    }

    #[test]
    fn history_takes_only_complete_dialog_turns() {
        // 三条一起构成「喂给模型的历史」的定义：
        // 只取 complete（半截消息会把模型带进坏状态）、只取 user/assistant
        // （工具往返是内部细节）、DESC LIMIT 取最近的（不是最旧的）。
        assert!(HISTORY_SQL.contains("status = 'complete'"));
        assert!(HISTORY_SQL.contains("role IN ('user','assistant')"));
        assert!(HISTORY_SQL.contains("ORDER BY seq DESC LIMIT ?"));
        assert!(HISTORY_SQL.contains("user_id = ?") && HISTORY_SQL.contains("session_id = ?"));
    }

    #[test]
    fn next_seq_is_scoped_to_the_session() {
        assert!(NEXT_SEQ_SQL.contains("user_id = ?"));
        assert!(NEXT_SEQ_SQL.contains("session_id = ?"));
        assert!(
            NEXT_SEQ_SQL.contains("COALESCE(MAX(seq),0)"),
            "空会话必须从 1 开始（COALESCE 缺了会算出 NULL）"
        );
    }

    #[test]
    fn insert_message_binds_exactly_the_columns_it_declares() {
        // 占位符个数与列数必须一致：少一个会在运行时报
        // 「column index out of range」，而那是写入路径，测试里最不容易撞上。
        let cols = INSERT_MESSAGE_SQL
            .split_once('(')
            .and_then(|(_, rest)| rest.split_once(')'))
            .map(|(c, _)| c.split(',').count())
            .unwrap_or(0);
        let qs = INSERT_MESSAGE_SQL.matches('?').count();
        assert_eq!(cols, 14, "列清单与 NewMessage 字段一一对应");
        assert_eq!(qs, cols, "占位符数必须等于列数：{INSERT_MESSAGE_SQL}");
    }

    #[test]
    fn touch_session_advances_count_by_a_question_and_an_answer() {
        // `+ 2` 是一问一答：改成 +1 会让侧栏条数少于真实消息数，
        // 而界面上没有人能看出来「少算了」。
        assert!(TOUCH_SESSION_SQL.contains("message_count + 2"));
        assert!(TOUCH_SESSION_SQL.contains("next_seq = ?"));
        assert!(TOUCH_SESSION_SQL.contains("user_id = ?") && TOUCH_SESSION_SQL.contains("id = ?"));
    }

    #[test]
    fn the_bound_expert_read_hides_soft_deleted_sessions() {
        // 绑定的专家只对「还看得见的会话」生效；软删的会话不该被它救活。
        assert!(SESSION_EXPERT_ID_SQL.contains("deleted_at IS NULL"));
        assert!(SESSION_EXPERT_ID_SQL.contains("user_id = ?"));
    }
}
