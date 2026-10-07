//! MBTI 测评记录的落库层。
//!
//! 一行 = 一次测完的结果（见迁移 `0010_mbti.sql` 的理由）。跟通道那边一样，
//! 这里只有「写进去 / 按用户读出来 / 标已应用」三件事，没有更新与删除接口
//! —— 用户重测就是新增一行。
//!
//! 保留期固定 20 条（[`KEEP_RECORDS`]）：够看出「我上次是什么」，
//! 又不会让这张表无界长大。清理在写入时顺手做，不单独开定时任务。

use crate::db::DbBridge;
use crate::error::ApiError;
use serde_json::{json, Value};
use sqlx::Row;

/// 每人保留的记录条数。
pub const KEEP_RECORDS: i64 = 20;

const COLS: &str =
    "rowid, id, owner_user_id, code, dimensions_json, answers_json, applied_expert_id, created_at";

#[derive(Debug, Clone)]
pub struct ResultRow {
    pub row_id: i64,
    pub code: String,
    pub dimensions: Value,
    pub answers: Value,
    pub applied_expert_id: Option<String>,
    pub created_at: i64,
}

fn s(row: &sqlx::sqlite::SqliteRow, name: &str) -> String {
    row.try_get::<Option<String>, _>(name)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn j(row: &sqlx::sqlite::SqliteRow, name: &str) -> Value {
    serde_json::from_str(&s(row, name)).unwrap_or_else(|_| json!({}))
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> ResultRow {
    ResultRow {
        row_id: row.try_get::<i64, _>("rowid").unwrap_or(0),
        code: s(row, "code"),
        dimensions: j(row, "dimensions_json"),
        answers: j(row, "answers_json"),
        applied_expert_id: row
            .try_get::<Option<String>, _>("applied_expert_id")
            .ok()
            .flatten(),
        created_at: row.try_get::<i64, _>("created_at").unwrap_or(0),
    }
}

fn storage(detail: impl std::fmt::Display) -> ApiError {
    ApiError::storage_unavailable(format!("MBTI 记录存储操作失败：{detail}"))
}

/// 对外形状。**唯一出口**。
pub fn to_public(row: &ResultRow, profile: Option<Value>) -> Value {
    json!({
        "row_id": row.row_id,
        "code": row.code,
        "dimensions": row.dimensions,
        "applied_expert_id": row.applied_expert_id,
        "created_at": row.created_at,
        // 档案在编译期常量里取；认不出的 code 就给 null，别拿空壳糊弄界面。
        "profile": profile,
    })
}

/// 记一次测评。返回新行的 `row_id`。
///
/// 顺手砍掉超出保留期的旧记录 —— 放在同一段事务里，中途失败不会留下
/// 「写进去但没清理」的半截状态。
pub async fn insert(
    db: &DbBridge,
    owner: quill_domain::UserId,
    code: &str,
    dimensions: &Value,
    answers: &Value,
    now: i64,
) -> Result<i64, ApiError> {
    let uid = owner.as_bytes().to_vec();
    let code = code.to_string();
    let dims = serde_json::to_string(dimensions).unwrap_or_else(|_| "{}".to_string());
    let ans = serde_json::to_string(answers).unwrap_or_else(|_| "{}".to_string());

    db.call(move |pool, _rt| {
        Box::pin(async move {
            let mut id = [0u8; 16];
            getrandom::fill(&mut id).map_err(|e| quill_agent::AgentError::Storage {
                detail: format!("mbti result id: {e}"),
            })?;

            let mut tx = pool
                .begin()
                .await
                .map_err(|e| quill_agent::AgentError::Storage {
                    detail: format!("mbti begin: {e}"),
                })?;
            let inserted = sqlx::query(
                "INSERT INTO mbti_results (id, owner_user_id, code, dimensions_json, \
                   answers_json, applied_expert_id, created_at) \
                 VALUES (?,?,?,?,?,NULL,?)",
            )
            .bind(id.to_vec())
            .bind(&uid)
            .bind(&code)
            .bind(&dims)
            .bind(&ans)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| quill_agent::AgentError::Storage {
                detail: format!("insert mbti result: {e}"),
            })?;
            let row_id = inserted.last_insert_rowid();

            sqlx::query(
                "DELETE FROM mbti_results WHERE owner_user_id = ? AND rowid NOT IN ( \
                   SELECT rowid FROM mbti_results WHERE owner_user_id = ? \
                   ORDER BY created_at DESC, rowid DESC LIMIT ?)",
            )
            .bind(&uid)
            .bind(&uid)
            .bind(KEEP_RECORDS)
            .execute(&mut *tx)
            .await
            .map_err(|e| quill_agent::AgentError::Storage {
                detail: format!("prune mbti results: {e}"),
            })?;

            tx.commit()
                .await
                .map_err(|e| quill_agent::AgentError::Storage {
                    detail: format!("mbti commit: {e}"),
                })?;
            Ok::<i64, quill_agent::AgentError>(row_id)
        })
    })
    .map_err(storage)
}

/// 历史，倒序（最新在前）。
pub async fn list(
    db: &DbBridge,
    owner: quill_domain::UserId,
    limit: i64,
) -> Result<Vec<ResultRow>, ApiError> {
    let uid = owner.as_bytes().to_vec();
    let sql = format!(
        "SELECT {COLS} FROM mbti_results WHERE owner_user_id = ? \
         ORDER BY created_at DESC, rowid DESC LIMIT ?"
    );
    let rows = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(&sql)
                    .bind(&uid)
                    .bind(limit)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| quill_agent::AgentError::Storage {
                        detail: format!("list mbti results: {e}"),
                    })
            })
        })
        .map_err(storage)?;
    Ok(rows.iter().map(from_row).collect())
}

/// 最新一条。
pub async fn latest(
    db: &DbBridge,
    owner: quill_domain::UserId,
) -> Result<Option<ResultRow>, ApiError> {
    Ok(list(db, owner, 1).await?.pop())
}

/// 按 `row_id` 取一条。**必须带 owner** —— `row_id` 是全局自增的，
/// 只按它查等于让任何用户读别人的测评记录。
pub async fn get_by_row_id(
    db: &DbBridge,
    owner: quill_domain::UserId,
    row_id: i64,
) -> Result<Option<ResultRow>, ApiError> {
    let uid = owner.as_bytes().to_vec();
    let sql = format!("SELECT {COLS} FROM mbti_results WHERE rowid = ? AND owner_user_id = ?");
    let found = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(&sql)
                    .bind(row_id)
                    .bind(&uid)
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e| quill_agent::AgentError::Storage {
                        detail: format!("get mbti result: {e}"),
                    })
            })
        })
        .map_err(storage)?;
    Ok(found.as_ref().map(from_row))
}

/// 标记「这条已经应用到某个专家」。
///
/// 只看 `row_id`，**不校验 owner** —— 调用方拿到的 `row_id` 必然来自本用户
/// 的列表，但这里再挡一道就得多带一个条件、还得防越权读到别人的行。
/// 所以对外的 `to_public` 里回的是 `applied_expert_id`，不回 `id`，
/// 界面拿不到裸 rowid 去试别人。
pub async fn mark_applied(
    db: &DbBridge,
    owner: quill_domain::UserId,
    row_id: i64,
    expert_id: &str,
) -> Result<(), ApiError> {
    let uid = owner.as_bytes().to_vec();
    let eid = expert_id.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "UPDATE mbti_results SET applied_expert_id = ? \
                 WHERE rowid = ? AND owner_user_id = ?",
            )
            .bind(&eid)
            .bind(row_id)
            .bind(&uid)
            .execute(&pool)
            .await
            .map_err(|e| quill_agent::AgentError::Storage {
                detail: format!("mark mbti applied: {e}"),
            })?;
            Ok::<(), quill_agent::AgentError>(())
        })
    })
    .map_err(storage)
}
