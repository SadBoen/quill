//! 定时任务的存储层（queue Q042）。
//!
//! 一张表 + 一个「到期」查询。**调度器只依赖这里的两个函数**：`due`（谁到点了）与
//! `mark_fired` / `mark_failed`（投递完怎么记账）—— 其余都是给 HTTP 面用的增删改查。
//!
//! 时间一律是 **epoch 毫秒**（与全仓其他表一致：`created_at`/`updated_at` 都是它）。
//! `next_fire_at` 由写入方算好（见 `api_cron::plan`）：`every` 是纯加法，`at` 直接就是
//! 那一刻 —— 这个模块不做任何日历/时区运算，那是 `api_cron` 的事。

use quill_domain::{SessionId, UserId};

use crate::db::{storage_error, DbBridge};

/// 与迁移里的 CHECK 同口径（改这里就要改那里，否则是「代码说得通、库里存不进」）。
pub const NAME_MAX_CHARS: usize = 120;
pub const MESSAGE_MAX_CHARS: usize = 32_000;
pub const EVERY_MIN_SECONDS: i64 = 60;
pub const EVERY_MAX_SECONDS: i64 = 31_536_000;

/// 一次 tick 最多处理几个到期任务。上限是**防雪崩**：同一条 `every` 任务配错了间隔
/// （比如 60 秒 × 几百条）时，别让一次 tick 把所有会话同时打起来。
pub const DUE_BATCH: i64 = 16;

/// 投递失败后的重排间隔：**不推进到正常的下一次**，而是 5 分钟后再试。
///
/// 为什么不直接推进：投递失败（比如模型端点挂了）时，用户最需要看到的是「它还在等」，
/// 而不是「它跳过了这一次」。为什么要有间隔：不设的话每个 tick 都会重试同一条，
/// 把日志和模型端点都打爆。
pub const FAILURE_RETRY_MS: i64 = 5 * 60 * 1000;

const COLUMNS: &str = "user_id, id, name, message, schedule_kind, every_seconds, run_at, tz, \
                       session_id, last_fired_at, next_fire_at, fired_count, last_error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    /// 固定间隔（秒）。
    Every { every_seconds: i64 },
    /// 指定时刻（epoch 毫秒）。**一次性**：投递后软删。
    At { run_at: i64 },
}

impl Schedule {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Every { .. } => "every",
            Self::At { .. } => "at",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronJobRow {
    pub id: String,
    pub name: String,
    pub message: String,
    pub schedule: Schedule,
    pub tz: String,
    /// 投递落点的会话。**首次投递时才建**（没投过的任务在界面上显示「暂无记录」）。
    pub session_id: Option<SessionId>,
    pub last_fired_at: Option<i64>,
    pub next_fire_at: i64,
    pub fired_count: i64,
    pub last_error: Option<String>,
}

fn blob16(v: &[u8], what: &str) -> Result<[u8; 16], AgentError> {
    v.try_into().map_err(|_| AgentError::Storage {
        detail: format!("{what} 不是 16 字节的标识（实际 {} 字节）", v.len()),
    })
}

use quill_agent::AgentError;

fn row_into_job(r: &sqlx::sqlite::SqliteRow) -> Result<CronJobRow, AgentError> {
    use sqlx::Row;
    let kind: String = r.get("schedule_kind");
    let every: Option<i64> = r.get("every_seconds");
    let at: Option<i64> = r.get("run_at");
    let schedule = match (kind.as_str(), every, at) {
        ("every", Some(secs), _) => Schedule::Every {
            every_seconds: secs,
        },
        ("at", _, Some(ms)) => Schedule::At { run_at: ms },
        (other, _, _) => {
            return Err(AgentError::Storage {
                detail: format!(
                    "定时任务的排期列与 kind 对不上（kind={other:?}, every={every:?}, at={at:?}）。\
                     下一步：这是迁移的 CHECK 该挡住的情况，请把它当缺陷报出来。"
                ),
            })
        }
    };
    let session: Option<Vec<u8>> = r.get("session_id");
    let session_id = match session {
        Some(b) => Some(SessionId::from_bytes(blob16(&b, "定时任务的 session_id")?)),
        None => None,
    };
    Ok(CronJobRow {
        id: r.get("id"),
        name: r.get("name"),
        message: r.get("message"),
        schedule,
        tz: r.get("tz"),
        session_id,
        last_fired_at: r.get("last_fired_at"),
        next_fire_at: r.get("next_fire_at"),
        fired_count: r.get("fired_count"),
        last_error: r.get("last_error"),
    })
}

/// 列出某个用户的任务（按下次触发时间升序 —— 界面上最该先看到的就是「马上要跑的」）。
pub fn list(
    db: &DbBridge,
    owner: UserId,
    limit: i64,
    offset: i64,
) -> Result<Vec<CronJobRow>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let sql = format!(
                "SELECT {COLUMNS} FROM cron_jobs \
                 WHERE user_id = ? AND deleted_at IS NULL \
                 ORDER BY next_fire_at ASC, id ASC LIMIT ? OFFSET ?"
            );
            let rows = sqlx::query(&sql)
                .bind(owner.as_bytes().to_vec())
                .bind(limit)
                .bind(offset)
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("列出定时任务", e))?;
            rows.iter().map(row_into_job).collect()
        })
    })
}

pub fn get(db: &DbBridge, owner: UserId, id: String) -> Result<Option<CronJobRow>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let sql = format!(
                "SELECT {COLUMNS} FROM cron_jobs WHERE user_id = ? AND id = ? AND deleted_at IS NULL"
            );
            let row = sqlx::query(&sql)
                .bind(owner.as_bytes().to_vec())
                .bind(&id)
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error("读定时任务", e))?;
            row.as_ref().map(row_into_job).transpose()
        })
    })
}

/// 新建或整体覆盖一条任务。`session_id` 为空串的语义由调用方决定（新建时给 None）。
pub fn put(db: &DbBridge, owner: UserId, row: CronJobRow, now: i64) -> Result<(), AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let (every, at) = match row.schedule {
                Schedule::Every { every_seconds } => (Some(every_seconds), None),
                Schedule::At { run_at } => (None, Some(run_at)),
            };
            sqlx::query(
                "INSERT INTO cron_jobs (user_id, id, name, message, schedule_kind, every_seconds, \
                   run_at, tz, session_id, last_fired_at, next_fire_at, fired_count, last_error, \
                   created_at, updated_at, deleted_at) \
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,NULL) \
                 ON CONFLICT (user_id, id) DO UPDATE SET \
                   name = excluded.name, message = excluded.message, \
                   schedule_kind = excluded.schedule_kind, every_seconds = excluded.every_seconds, \
                   run_at = excluded.run_at, tz = excluded.tz, \
                   next_fire_at = excluded.next_fire_at, last_error = NULL, \
                   updated_at = excluded.updated_at, deleted_at = NULL",
            )
            .bind(owner.as_bytes().to_vec())
            .bind(&row.id)
            .bind(&row.name)
            .bind(&row.message)
            .bind(row.schedule.kind())
            .bind(every)
            .bind(at)
            .bind(&row.tz)
            .bind(row.session_id.map(|s| s.as_bytes().to_vec()))
            .bind(row.last_fired_at)
            .bind(row.next_fire_at)
            .bind(row.fired_count)
            .bind(&row.last_error)
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|e| storage_error("写定时任务", e))?;
            Ok(())
        })
    })
}

/// 软删。返回 `false` = 本来就没有（**不是错误**：调用方要拿它如实回 404）。
pub fn soft_delete(db: &DbBridge, owner: UserId, id: String, now: i64) -> Result<bool, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let out = sqlx::query(
                "UPDATE cron_jobs SET deleted_at = ?, updated_at = ? \
                 WHERE user_id = ? AND id = ? AND deleted_at IS NULL",
            )
            .bind(now)
            .bind(now)
            .bind(owner.as_bytes().to_vec())
            .bind(&id)
            .execute(&pool)
            .await
            .map_err(|e| storage_error("删除定时任务", e))?;
            Ok(out.rows_affected() > 0)
        })
    })
}

/// 到点该跑的任务（**跨用户** —— 调度器是全局的，一条 tick 服务所有用户）。
pub fn due(db: &DbBridge, now: i64, limit: i64) -> Result<Vec<(UserId, CronJobRow)>, AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let sql = format!(
                "SELECT {COLUMNS} FROM cron_jobs \
                 WHERE deleted_at IS NULL AND next_fire_at <= ? \
                 ORDER BY next_fire_at ASC LIMIT ?"
            );
            let rows = sqlx::query(&sql)
                .bind(now)
                .bind(limit)
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error("取到期定时任务", e))?;
            let mut out = Vec::with_capacity(rows.len());
            for r in &rows {
                let owner = UserId::from_bytes(blob16(
                    &sqlx::Row::get::<Vec<u8>, _>(r, "user_id"),
                    "定时任务的 user_id",
                )?);
                out.push((owner, row_into_job(r)?));
            }
            Ok(out)
        })
    })
}

/// 投递成功后的记账：推进下一次、记 fired 次数、清掉上次的错误。
pub fn mark_fired(
    db: &DbBridge,
    owner: UserId,
    id: String,
    fired_at: i64,
    next_fire_at: i64,
) -> Result<(), AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "UPDATE cron_jobs SET last_fired_at = ?, next_fire_at = ?, \
                   fired_count = fired_count + 1, last_error = NULL, updated_at = ? \
                 WHERE user_id = ? AND id = ? AND deleted_at IS NULL",
            )
            .bind(fired_at)
            .bind(next_fire_at)
            .bind(fired_at)
            .bind(owner.as_bytes().to_vec())
            .bind(&id)
            .execute(&pool)
            .await
            .map_err(|e| storage_error("记定时任务投递", e))?;
            Ok(())
        })
    })
}

/// 投递失败后的记账：**不推进到正常的下一次**，而是 5 分钟后再试，并把原因存下来。
pub fn mark_failed(
    db: &DbBridge,
    owner: UserId,
    id: String,
    now: i64,
    error: String,
) -> Result<(), AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "UPDATE cron_jobs SET next_fire_at = ?, last_error = ?, updated_at = ? \
                 WHERE user_id = ? AND id = ? AND deleted_at IS NULL",
            )
            .bind(now + FAILURE_RETRY_MS)
            .bind(&error)
            .bind(now)
            .bind(owner.as_bytes().to_vec())
            .bind(&id)
            .execute(&pool)
            .await
            .map_err(|e| storage_error("记定时任务失败", e))?;
            Ok(())
        })
    })
}

/// 把任务绑到一条会话上（首次投递时建完会话就调它）。
pub fn set_session(
    db: &DbBridge,
    owner: UserId,
    id: String,
    session_id: SessionId,
    now: i64,
) -> Result<(), AgentError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "UPDATE cron_jobs SET session_id = ?, updated_at = ? \
                 WHERE user_id = ? AND id = ? AND deleted_at IS NULL",
            )
            .bind(session_id.as_bytes().to_vec())
            .bind(now)
            .bind(owner.as_bytes().to_vec())
            .bind(&id)
            .execute(&pool)
            .await
            .map_err(|e| storage_error("绑定时任务会话", e))?;
            Ok(())
        })
    })
}
