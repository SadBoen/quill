//! `/api/cron` —— 定时任务的 HTTP 面（queue Q042）。
//!
//! 路由形状对齐 Octop（`.octop-ref/octop/src/octop/api/routers/cron.py:109-231`：
//! list / create / get / patch / delete，**只读对齐**），字段名与语义则照
//! **前端已经写死的那份契约**（`ui/web/src/automations/api.ts` 的 `CRON_ROUTE` /
//! `CRON_JOB_ROUTE` 与三个类型）—— 与 Q058 的资料库写入面同一情形：
//! 契约先在前端定下，后端这次接上。
//!
//! ## 排期只支持两种（与迁移里的 CHECK 同口径）
//!
//! - `{"type":"every","every_seconds":N}` —— 固定间隔，纯加法；
//! - `{"type":"at","at":"<RFC3339>","tz":"<IANA 名>"}` —— 一次性，投递后软删。
//!
//! **刻意不支持 `{"type":"cron","cron_expr":…}`**：cron 表达式 + IANA 时区要 cron 解析
//! 与时区库，而 DST 感知的时区算错一小时是用户可见的错。表单里那个选项**同批去掉了**
//! —— 不画做不到的入口（Q041 删假路由 / Q037 保留诚实 501 是同一条纪律）。
//! 传进来就回 400 并说清下一步，不是静默当成别的排期。
//!
//! ## 时间在线上是 **ISO 字符串**
//!
//! 前端用 `new Date(value)` 渲染（`Automations.tsx:444`），所以 `last_fired_at` /
//! `next_fire_at` 必须是它能解析的 ISO 串，**不是** epoch 数字（库里存的是毫秒）。
//! 这一层就是两种表示的转换点。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, SecondsFormat};
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::cron_repo::{self, CronJobRow, Schedule};
use crate::error::ApiError;
use crate::jsonx::{need_str, opt_str};
use crate::state::AppState;

/// 不带 `limit` 时的条数（与其它列表端点同口径）。
pub const DEFAULT_LIMIT: i64 = 50;

/// 一次最多取多少条。
pub const MAX_LIMIT: i64 = 200;

const WHERE_LIST: &str = "GET /api/cron";
const WHERE_WRITE: &str = "POST/PATCH /api/cron";

/// epoch 毫秒 → ISO 串（`new Date()` 能解析的形状）。
fn iso(ms: i64) -> Option<String> {
    DateTime::from_timestamp_millis(ms).map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn iso_or_empty(ms: i64) -> String {
    // 极端时钟下 `from_timestamp_millis` 会返回 None（超出可表示范围）。
    // 那时给空串 —— 前端 `new Date("")` 得到 Invalid Date，界面上显示不出时间，
    // 但**不会假装一个时间**。比编一个更诚实。
    iso(ms).unwrap_or_default()
}

/// 库里的一行 → 列表用的 JSON（`CronJobSummary`）。
fn summary_json(row: &CronJobRow) -> Value {
    json!({
        "id": row.id,
        "name": row.name,
        "schedule": schedule_json(&row.schedule, &row.tz),
        "last_fired_at": row.last_fired_at.and_then(iso),
        "next_fire_at": iso_or_empty(row.next_fire_at),
        "session_id": row.session_id.map(|s| s.to_compact_hex()),
        // 下面三个字段前端目前不显示，但排障要看 —— 多给不破坏契约（前端按名取）。
        "fired_count": row.fired_count,
        "last_error": row.last_error,
    })
}

/// 库里的一行 → 单条用的 JSON（`CronJob`：带 message）。
fn job_json(row: &CronJobRow) -> Value {
    let mut v = summary_json(row);
    v["message"] = json!(row.message);
    v
}

fn schedule_json(schedule: &Schedule, tz: &str) -> Value {
    match schedule {
        Schedule::Every { every_seconds } => json!({
            "type": "every",
            "every_seconds": every_seconds,
        }),
        Schedule::At { run_at } => json!({
            "type": "at",
            "at": iso_or_empty(*run_at),
            "tz": tz,
        }),
    }
}

/// 解析请求体里的排期。返回 `(排期, tz, 首次触发时间)`。
fn parse_schedule(body: &Value, now: i64) -> Result<(Schedule, String, i64), ApiError> {
    let schedule = body.get("schedule").ok_or_else(|| {
        ApiError::bad_request(format!(
            "缺少必填字段 {WHERE_WRITE}.schedule。\
             下一步：{{\"schedule\":{{\"type\":\"every\",\"every_seconds\":3600}}}} \
             或 {{\"schedule\":{{\"type\":\"at\",\"at\":\"2026-09-03T10:00:00+08:00\",\"tz\":\"Asia/Shanghai\"}}}}"
        ))
    })?;
    let kind = need_str(schedule, "type", &format!("{WHERE_WRITE}.schedule"))?;
    match kind.as_str() {
        "every" => {
            let secs = schedule
                .get("every_seconds")
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "{WHERE_WRITE}.schedule.every_seconds 必须是整数（秒）。\
                         下一步：60 到 {} 之间。",
                        cron_repo::EVERY_MAX_SECONDS
                    ))
                })?;
            if !(cron_repo::EVERY_MIN_SECONDS..=cron_repo::EVERY_MAX_SECONDS).contains(&secs) {
                return Err(ApiError::bad_request(format!(
                    "间隔 {secs} 秒越界：只接受 {} 到 {} 秒（60 秒 ~ 365 天）。\
                     下一步：改到区间内 —— 比 1 分钟更密的排期不该用定时任务做。",
                    cron_repo::EVERY_MIN_SECONDS,
                    cron_repo::EVERY_MAX_SECONDS
                )));
            }
            Ok((
                Schedule::Every {
                    every_seconds: secs,
                },
                String::new(),
                now + secs * 1000,
            ))
        }
        "at" => {
            let raw = need_str(schedule, "at", &format!("{WHERE_WRITE}.schedule"))?;
            let when = DateTime::parse_from_rfc3339(&raw)
                .map_err(|e| {
                    ApiError::bad_request(format!(
                        "时刻 {raw:?} 不是带时区的 RFC3339（{e}）。\
                         下一步：写成 `2026-09-03T10:00:00+08:00` 这样（带偏移）。"
                    ))
                })?
                .timestamp_millis();
            if when <= now {
                return Err(ApiError::bad_request(format!(
                    "指定的时刻 {raw:?} 已经过去（现在是 {}）。\
                     下一步：给一个将来的时刻；要「马上跑一次」请直接在会话里发消息。",
                    iso_or_empty(now)
                )));
            }
            let tz =
                opt_str(schedule, "tz", &format!("{WHERE_WRITE}.schedule"))?.unwrap_or_default();
            Ok((Schedule::At { run_at: when }, tz, when))
        }
        // 明确拒绝，而不是猜成别的排期。
        "cron" => Err(ApiError::bad_request(
            "cron 表达式（`cron_expr`）暂不支持：它要 cron 解析 + IANA 时区库，\
             而时区算错一小时是用户可见的错，所以宁可不做也不做错。\
             下一步：用 `{\"type\":\"every\",\"every_seconds\":N}` 或 \
             `{\"type\":\"at\",\"at\":\"…\"}`；界面上的选项已同步去掉。"
                .to_string(),
        )),
        other => Err(ApiError::bad_request(format!(
            "未知的排期类型 {other:?}：只支持 `every` 与 `at`。"
        ))),
    }
}

/// 解析写入体（POST 与 PATCH 共用）。PATCH 收的也是**完整**的 `CronWrite` ——
/// 前端编辑表单就是整份提交；不按字段做部分更新，是因为「改个名字却把排期清空」
/// 这类事故正是部分更新的经典坑。
fn parse_write(
    body: &Value,
    now: i64,
) -> Result<(String, String, Schedule, String, i64), ApiError> {
    crate::api_experts::only_keys(body, &["name", "message", "schedule"], WHERE_WRITE)?;
    let name = need_str(body, "name", WHERE_WRITE)?;
    if name.chars().count() > cron_repo::NAME_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "名称超过 {} 个字符。下一步：写短一点。",
            cron_repo::NAME_MAX_CHARS
        )));
    }
    let message = need_str(body, "message", WHERE_WRITE)?;
    if message.chars().count() > cron_repo::MESSAGE_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "要投递的内容超过 {} 个字符。下一步：写短一点，或把长材料放进资料库再引用它。",
            cron_repo::MESSAGE_MAX_CHARS
        )));
    }
    let (schedule, tz, next) = parse_schedule(body, now)?;
    Ok((name, message, schedule, tz, next))
}

fn new_job_id() -> Result<String, ApiError> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b)
        .map_err(|e| ApiError::internal(format!("生成任务标识失败（随机源不可用）：{e}")))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

#[derive(Debug, serde::Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// `GET /api/cron?limit=&offset=` —— 列出当前用户的任务。
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(ApiError::bad_request(format!(
            "{WHERE_LIST} 的 limit 越界（收到 {limit}）：只接受 1~{MAX_LIMIT}。\
             下一步：填 1~{MAX_LIMIT}，或去掉 limit 用默认的 {DEFAULT_LIMIT}。"
        )));
    }
    let offset = q.offset.unwrap_or(0).max(0);
    let db = state.db()?;
    // 多取一条来判断「还有没有下一页」——比 `COUNT(*)` 省一次扫描，
    // 而且不会在两次查询之间因为新增而算错。
    let rows = cron_repo::list(db, user.0.user_id, limit + 1, offset).map_err(map_agent)?;
    let more = rows.len() as i64 > limit;
    let items: Vec<Value> = rows.iter().take(limit as usize).map(summary_json).collect();
    Ok(Json(json!({
        "items": items,
        "next_offset": if more { json!(offset + items.len() as i64) } else { Value::Null },
    })))
}

/// `GET /api/cron/{id}` —— 单条（带 message）。
pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let db = state.db()?;
    let row = cron_repo::get(db, user.0.user_id, id.clone()).map_err(map_agent)?;
    match row {
        Some(row) => Ok(Json(job_json(&row))),
        None => Err(ApiError::entity_not_found(format!(
            "没有 id 为 {id:?} 的定时任务（或它不属于当前账号）。\
             下一步：用 GET /api/cron 拿现有任务的 id。"
        ))),
    }
}

/// `POST /api/cron` —— 新建一条。
pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    let now = crate::db::now_ms();
    let (name, message, schedule, tz, next) = parse_write(&body, now)?;
    let id = new_job_id()?;
    let row = CronJobRow {
        id: id.clone(),
        name,
        message,
        schedule,
        tz,
        // 会话**首次投递时才建**（还没跑过的任务在界面上显示「暂无记录」）。
        session_id: None,
        last_fired_at: None,
        next_fire_at: next,
        fired_count: 0,
        last_error: None,
    };
    let db = state.db()?;
    cron_repo::put(db, user.0.user_id, row, now).map_err(map_agent)?;
    let saved = cron_repo::get(db, user.0.user_id, id)
        .map_err(map_agent)?
        .ok_or_else(|| ApiError::internal("刚写入的任务读不回来".to_string()))?;
    Ok((StatusCode::CREATED, Json(job_json(&saved))).into_response())
}

/// `PATCH /api/cron/{id}` —— 覆盖一条（收完整的 `CronWrite`，见 `parse_write` 的注释）。
pub async fn patch(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    let now = crate::db::now_ms();
    let db = state.db()?;
    let existing = cron_repo::get(db, user.0.user_id, id.clone())
        .map_err(map_agent)?
        .ok_or_else(|| {
            ApiError::entity_not_found(format!(
                "没有 id 为 {id:?} 的定时任务，改不了。下一步：用 GET /api/cron 拿现有任务的 id。"
            ))
        })?;
    let (name, message, schedule, tz, next) = parse_write(&body, now)?;
    let row = CronJobRow {
        id: id.clone(),
        name,
        message,
        schedule,
        tz,
        // 会话沿用（改排期不该换一条会话 —— 那会把历史记录劈成两半）。
        session_id: existing.session_id,
        last_fired_at: existing.last_fired_at,
        next_fire_at: next,
        fired_count: existing.fired_count,
        // 排期与内容都重写了，上次的错误不再代表现状。
        last_error: None,
    };
    cron_repo::put(db, user.0.user_id, row, now).map_err(map_agent)?;
    let saved = cron_repo::get(db, user.0.user_id, id)
        .map_err(map_agent)?
        .ok_or_else(|| ApiError::internal("刚改完的任务读不回来".to_string()))?;
    Ok(Json(job_json(&saved)))
}

/// `DELETE /api/cron/{id}` —— 软删。
pub async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let db = state.db()?;
    let removed = cron_repo::soft_delete(db, user.0.user_id, id.clone(), crate::db::now_ms())
        .map_err(map_agent)?;
    if !removed {
        return Err(ApiError::entity_not_found(format!(
            "没有 id 为 {id:?} 的定时任务，删不了（可能已经被删过）。\
             下一步：用 GET /api/cron 看现在的清单。"
        )));
    }
    Ok(Json(json!({ "id": id, "deleted": true })))
}

/// 存储层的错误 → 统一的错误信封（带「下一步」）。
fn map_agent(e: quill_agent::AgentError) -> ApiError {
    ApiError::storage_unavailable(format!(
        "定时任务存储失败：{e}。下一步：用 `quill doctor --section=db` 看存储诊断。"
    ))
}
