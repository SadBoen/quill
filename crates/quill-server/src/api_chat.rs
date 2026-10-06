use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use quill_provider::{Message, TokenUsage};

use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::db::now_ms;
use crate::error::ApiError;
use crate::state::AppState;

pub const MAX_CONTENT_LEN: usize = 32_000;
pub const HISTORY_LIMIT: i64 = 40;

fn new_id() -> Result<[u8; 16], ApiError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b)
        .map_err(|e| ApiError::internal(format!("生成会话标识失败（系统随机源不可用）：{e}")))?;
    if b == [0u8; 16] {
        b[0] = 1;
    }
    Ok(b)
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;
    let rows = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<Vec<Value>, quill_agent::AgentError> = async {
                    let out = sqlx::query(
                        "SELECT hex(id) AS id, kind, room_id, title, expert_id, provider_id, \
                         model, state, message_count, created_at, last_active_at \
                         FROM sessions WHERE user_id = ? AND deleted_at IS NULL \
                         ORDER BY last_active_at DESC LIMIT 100",
                    )
                    .bind(uid.as_bytes().to_vec())
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("列会话", e))?;
                    Ok(out
                        .into_iter()
                        .map(|row| {
                            json!({
                                "id": s(&row, "id"),
                                "kind": s(&row, "kind"),
                                "room_id": s(&row, "room_id"),
                                "title": s(&row, "title"),
                                "expert_id": s(&row, "expert_id"),
                                "provider_id": s(&row, "provider_id"),
                                "model": s(&row, "model"),
                                "state": s(&row, "state"),
                                "message_count": n(&row, "message_count"),
                                "created_at": n(&row, "created_at"),
                                "last_active_at": n(&row, "last_active_at"),
                            })
                        })
                        .collect())
                }
                .await;
                r
            })
        })
        .map_err(storage)?;

    Ok(Json(json!({ "sessions": rows })).into_response())
}

fn storage(e: quill_agent::AgentError) -> ApiError {
    ApiError::storage_unavailable(e.to_string())
}

/// 模型调用失败时怎么措辞。
///
/// **关键是把「连不上」与「连上了但被拒」分开。** `ProviderError` 本来就分得清
/// （`Unreachable` / `Timeout` / `ModelNotFound` / `NotConfigured` 是没连上；
/// `Status` / `MalformedResponse` / `UpstreamRejected` / `InvalidRequest`
/// 是回过话的），而这两种情况的「下一步」**完全相反**。
///
/// 早先这里不管哪种都套 `service_unavailable`，于是下面这句在第二种情况下
/// 变成了假建议：
/// > 「模型服务不可用：先执行 curl …/models 确认端点活着；本地模型请先启动 llama-server」
///
/// 实测踩到过（2026-10-06）：模型上下文 8192，装了 5 个 SKILL 之后请求变成
/// 8525 token，上游回 `exceed_context_size_error`（`Status { code: 400 }`）。
/// 界面当时让用户去检查一个**正在正常应答**的服务。见 ISSUE-018。
///
/// detail 走的是 `e.message()` 而不是 `{e}`：后者会把「→ 下一步：…」也拼进来，
/// 而下面每个分支都已经给了**结构化**的 `next_step`，界面把它渲染成独立段落 ——
/// 两条都自称「下一步」，且 detail 那条含糊。见 ISSUE-020。
fn provider_failure(e: quill_provider::ProviderError) -> ApiError {
    use quill_provider::ProviderError;
    let detail = format!("模型调用失败：{}", e.message());
    match e {
        // 没连上：那句「去确认端点活着 / 把 llama-server 起起来」在这里是对的。
        ProviderError::Unreachable { .. }
        | ProviderError::Timeout { .. }
        | ProviderError::ModelNotFound { .. }
        | ProviderError::NotConfigured { .. } => ApiError::service_unavailable(detail),
        // 回过话了。端点是活的，建议必须关于「它说了什么」而不是「怎么连上」。
        ProviderError::Status { .. } => ApiError::provider_rejected(detail, ADVICE_REJECTED_BY_STATUS),
        other => ApiError::provider_rejected(detail, advice_for_unusable_reply(&other)),
    }
}

/// 上游回了一个 HTTP 错误状态码时的建议。
///
/// 说「最常见的一种是…」而不是「就是上下文超了」：这里**没有**解析上游的 body，
/// 按关键字去猜就又变成替上游说话。点出最常见的那一种，是因为它在本机 4B 上
/// 最常发生、而且给出的处置真的对得上；不中也不耽误 —— detail 里原样带着
/// 上游自己的那句话。
/// `pub(crate)`：`error.rs` 的单测要引用它，钉住「模型回过话就不许建议重启它」
/// 那条断言（ISSUE-018 的回归）。
pub(crate) const ADVICE_REJECTED_BY_STATUS: &str = "模型服务**活着**并回了一个错误状态码，它自己的原话在上一段里。\
     下一步：照那句话改，**不要**去重启模型服务（它正在正常应答）。最常见的一种是请求超出了模型上下文：\
     调大 QUILL_LLM_MAX_CONTEXT_TOKENS，或减少这一轮挂着的技能/工具\
     （挂了多少可以在 GET /api/extensions/skills 的 model_can_see 里逐条数）。\
     改完用同一条消息重试；细节跑 `quill doctor`。";

/// 上游回的不是 HTTP 错误，但也没给出可用结果时的建议。
fn advice_for_unusable_reply(e: &quill_provider::ProviderError) -> &'static str {
    let _ = e;
    "模型服务**活着**，但没有按协议回出可用的结果，它自己的原话在上一段里。\
     下一步：先照那句话排查；这是 quill 与该模型端点之间的问题，跑 `quill doctor` \
     打印完整诊断后再用同一条消息重试。"
}

fn s(row: &sqlx::sqlite::SqliteRow, name: &str) -> String {
    sqlx::Row::try_get::<Option<String>, _>(row, name)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn n(row: &sqlx::sqlite::SqliteRow, name: &str) -> i64 {
    sqlx::Row::try_get::<Option<i64>, _>(row, name).ok().flatten().unwrap_or(0)
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;
    let requested_expert = body
        .get("expert_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    // 没指定角色就自动挂通用专家，不留 `expert_id = null`。
    // 留空的代价是实打实的：会话建出来 `persona_applied=false`，界面上
    // 归进「默认（未选角色）」分组，用户以为在跟某个角色说话，其实没有。
    let expert_id = crate::general_expert::resolve_for_session(&state, uid, requested_expert)?;
    let title = body
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(64)
        .collect::<String>();

    let id = new_id()?;
    let now = now_ms();
    let room = format!("room-{}", &hex_lower(&id)[..12]);
    let workspace = format!("ws/{}", &hex_lower(&id)[..12]);
    let model = state
        .llm_config
        .read()
        .map_err(|_| ApiError::internal("llm_config 锁被毒化"))?
        .model
        .clone();

    let room_out = room.clone();
    let title_out = title.clone();
    let model_out = model.clone();
    let hex_id = hex(&id);
    let expert_for_write = expert_id.clone();

    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r: Result<(), quill_agent::AgentError> = async {
                sqlx::query(
                    "INSERT INTO sessions(user_id,id,kind,room_id,title,provider_id,model,state,\
                     workspace_path,created_at,updated_at,last_active_at) \
                     VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                )
                .bind(uid.as_bytes().to_vec())
                .bind(id.to_vec())
                .bind("solo")
                .bind(room.clone())
                .bind(title.clone())
                .bind("local")
                .bind(model.clone())
                .bind("IDLE")
                .bind(workspace.clone())
                .bind(now)
                .bind(now)
                .bind(now)
                .execute(&pool)
                .await
                .map_err(|e| crate::db::storage_error("建会话", e))?;
                if !expert_for_write.is_empty() {
                    sqlx::query("UPDATE sessions SET expert_id = ? WHERE user_id = ? AND id = ?")
                        .bind(expert_for_write)
                        .bind(uid.as_bytes().to_vec())
                        .bind(id.to_vec())
                        .execute(&pool)
                        .await
                        .map_err(|e| crate::db::storage_error("挂专家", e))?;
                }
                Ok(())
            }
            .await;
            r
        })
    })
    .map_err(storage)?;

    // 建会话时就体检一次绑定：调用方指定了专家但那个专家不存在/已删的会话照样建
    // （用户可能只是想聊天），但必须在响应里明说「你选的专家没生效」，
    // 不能等发消息时才让用户自己发现。
    // 没指定的那些已经在 `resolve_for_session` 里换成通用专家了，所以这里恒有值。
    let persona = resolve_persona(&db, uid, id).await?;

    Ok(Json(json!({
        "id": hex_id,
        "room_id": room_out,
        "title": title_out,
        "model": model_out,
        "state": "IDLE",
        "expert_id": expert_id,
        "persona_applied": persona.instructions.is_some(),
        "expert_notice": persona.notice,
    }))
    .into_response())
}

// 读路径走 SQLite 的 hex(id)，输出大写；写路径必须一致，否则同一个 id 在
// POST 响应和 GET 列表里是两种字符串，前端按 id 去重/匹配会全部失配。
fn hex(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn hex_lower(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;
    let sid = parse_id(&id)?;
    let found = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<bool, quill_agent::AgentError> = async {
                    let n: i64 = sqlx::query_scalar(
                        "SELECT count(*) FROM sessions WHERE user_id = ? AND id = ? AND deleted_at IS NULL",
                    )
                    .bind(uid.as_bytes().to_vec())
                    .bind(sid.to_vec())
                    .fetch_one(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("查会话", e))?;
                    Ok(n > 0)
                }
                .await;
                r
            })
        })
        .map_err(storage)?;

    if found {
        Ok(Json(json!({ "id": id, "found": true })).into_response())
    } else {
        Err(ApiError::entity_not_found(format!(
            "会话 {id} 不存在，或不属于当前用户。下一步：用 GET /api/sessions 看自己的会话列表。"
        )))
    }
}

/// 软删会话：`SET deleted_at = ?`，与专家删除同一口径。
///
/// 硬删会连带丢掉 `messages`（外键 ON DELETE CASCADE）且不可逆，而 `sessions`
/// 表从 0001 起就预留了 `deleted_at`，列表查询也一直在按它过滤。
///
/// 幂等：对已软删的会话再删一次返回 200 + `deleted: false`，不报 404 ——
/// 客户端重试删除不该被当成「资源不存在」。
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;
    let sid = parse_id(&id)?;

    // 先看这一行在不在：不存在 / 不属于当前用户 → 404；已软删 → 幂等 false。
    let prior: Option<Option<i64>> = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<Option<Option<i64>>, quill_agent::AgentError> = async {
                    Ok(sqlx::query_scalar(
                        "SELECT deleted_at FROM sessions WHERE user_id = ? AND id = ?",
                    )
                    .bind(uid.as_bytes().to_vec())
                    .bind(sid.to_vec())
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("查会话", e))?)
                }
                .await;
                r
            })
        })
        .map_err(storage)?;

    let Some(deleted_at) = prior else {
        return Err(ApiError::entity_not_found(format!(
            "会话 {id} 不存在，或不属于当前用户。\
             下一步：用 GET /api/sessions 看自己的会话列表，确认 id 没写错。"
        )));
    };
    if deleted_at.is_some() {
        return Ok(Json(json!({
            "id": id,
            "deleted": false,
            "note": "该会话此前已被删除，本次为幂等重试（未重复删除）。"
        }))
        .into_response());
    }

    let now = now_ms();
    let affected = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<u64, quill_agent::AgentError> = async {
                    let n = sqlx::query(
                        "UPDATE sessions SET deleted_at = ?, updated_at = ? \
                         WHERE user_id = ? AND id = ? AND deleted_at IS NULL",
                    )
                    .bind(now)
                    .bind(now)
                    .bind(uid.as_bytes().to_vec())
                    .bind(sid.to_vec())
                    .execute(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("软删会话", e))?;
                    Ok(n.rows_affected())
                }
                .await;
                r
            })
        })
        .map_err(storage)?;

    // 会话本身没了即可用性；派工记录与团队关系保留，便于回溯这段会话做过什么。
    // `sessions_auth` 不在这里处理：它只存登录令牌（token_hash / family_id /
    // revoked_at），没有指向聊天会话的外键，把用户的登录令牌随一条聊天会话
    // 一起吊销会直接把用户踢出整个实例。
    Ok(Json(json!({
        "id": id,
        "deleted": affected > 0,
        "note": if affected > 0 {
            "已软删除：会话不再出现在列表里，消息记录保留（未被物理删除）。"
        } else {
            "该会话此前已被删除，本次为幂等重试（未重复删除）。"
        }
    }))
    .into_response())
}

pub async fn list_messages(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;
    let sid = parse_id(&id)?;
    ensure_session(db, uid, sid).await?;
    let rows = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<Vec<Value>, quill_agent::AgentError> = async {
                    let out = sqlx::query(
                        "SELECT hex(id) AS id, seq, role, status, content, reasoning, \
                         input_tokens, output_tokens, turn_ms, error_code, created_at \
                         FROM messages WHERE user_id = ? AND session_id = ? ORDER BY seq",
                    )
                    .bind(uid.as_bytes().to_vec())
                    .bind(sid.to_vec())
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("列消息", e))?;
                    Ok(out
                        .into_iter()
                        .map(|row| {
                            json!({
                                "id": s(&row, "id"),
                                "seq": n(&row, "seq"),
                                "role": s(&row, "role"),
                                "status": s(&row, "status"),
                                "content": s(&row, "content"),
                                "reasoning": s(&row, "reasoning"),
                                "input_tokens": n(&row, "input_tokens"),
                                "output_tokens": n(&row, "output_tokens"),
                                "turn_ms": n(&row, "turn_ms"),
                                "error_code": s(&row, "error_code"),
                                "created_at": n(&row, "created_at"),
                            })
                        })
                        .collect())
                }
                .await;
                r
            })
        })
        .map_err(storage)?;

    Ok(Json(json!({ "messages": rows })).into_response())
}

/// 会话级 Token 统计。界面那排指标 chip 的数据源。
///
/// 聚合口径全在 [`crate::session_metrics`] 里，是纯函数、测得到。
/// 这里只负责把消息行取出来喂给它。
///
/// **注意 `null` 是有意义的返回值**：算不出来的指标就是 `null`，前端按
/// 「跳过该 chip」处理。把它们填成 0 会让界面显示出一个从没被测量过的数字。
pub async fn metrics(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;
    let sid = parse_id(&id)?;
    ensure_session(db, uid, sid).await?;

    let rows = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<Vec<crate::session_metrics::MessageUsage>, quill_agent::AgentError> =
                    async {
                        let out = sqlx::query(
                            "SELECT role, input_tokens, output_tokens, turn_ms, cache_read_tokens \
                             FROM messages WHERE user_id = ? AND session_id = ? ORDER BY seq",
                        )
                        .bind(uid.as_bytes().to_vec())
                        .bind(sid.to_vec())
                        .fetch_all(&pool)
                        .await
                        .map_err(|e| crate::db::storage_error("读会话统计", e))?;

                        Ok(out
                            .into_iter()
                            .map(|row| {
                                let role = s(&row, "role");
                                crate::session_metrics::MessageUsage {
                                    is_user: role == "user",
                                    is_assistant: role == "assistant",
                                    input_tokens: n(&row, "input_tokens"),
                                    output_tokens: n(&row, "output_tokens"),
                                    turn_ms: nullable_n(&row, "turn_ms"),
                                    // 可空列必须区分「没上报」和「上报了 0」。
                                    cache_read_tokens: nullable_n(&row, "cache_read_tokens"),
                                }
                            })
                            .collect())
                    }
                    .await;
                r
            })
        })
        .map_err(storage)?;

    Ok(Json(crate::session_metrics::aggregate(&rows).to_json()).into_response())
}

/// 读一个可空的整数列：`NULL` 保持 `None`，绝不塌成 `0`。
///
/// 塌成 0 是这套统计里最容易犯、后果也最直接的错误：`cache_read_tokens`
/// 从 NULL 变成 0，界面就会理直气壮地显示「缓存命中 0.0%」—— 而真实情况
/// 只是模型端没报这个数。
fn nullable_n(row: &sqlx::sqlite::SqliteRow, key: &str) -> Option<i64> {
    use sqlx::Row;
    row.try_get::<Option<i64>, _>(key).unwrap_or(None)
}

fn parse_id(raw: &str) -> Result<[u8; 16], ApiError> {
    quill_domain::SessionId::parse(raw)
        .map(|s| *s.as_bytes())
        .map_err(|e| {
            ApiError::bad_request(format!(
                "会话 ID {raw:?} 非法（{e}）。下一步：用 GET /api/sessions 拿到的 32 位 hex 再来调。"
            ))
        })
}

/// 发一句话：存用户消息 → 调模型 → 存助手消息 → 返回。
pub async fn post_message(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<axum::response::Response, ApiError> {
    let content = body
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Err(ApiError::bad_request(
            "content 不能为空。下一步：{\"content\": \"你好\"}".to_string(),
        ));
    }
    if content.chars().count() > MAX_CONTENT_LEN {
        return Err(ApiError::bad_request(format!(
            "content 太长（{} 字符，上限 {MAX_CONTENT_LEN}）。下一步：分段发送。",
            content.chars().count()
        )));
    }

    let provider = state.llm()?;
    let llm_config = state
        .llm_config
        .read()
        .map_err(|_| ApiError::internal("llm_config 锁被毒化"))?
        .clone();
    let db = state.db()?.clone();
    let uid = user.0.user_id;
    let sid = parse_id(&id)?;

    ensure_session(&db, uid, sid).await?;

    let history = load_history(&db, uid, sid).await?;
    let persona = resolve_persona(&db, uid, sid).await?;
    let seq_user = next_seq(&db, uid, sid).await?;

    let user_id16 = new_id()?;
    let user_created_at = append_message(
        &db,
        uid,
        sid,
        &user_id16,
        seq_user,
        "user",
        "complete",
        &content,
        None,
        // 用户这一轮没调用模型，没有 token 概念。
        TokenUsage::default(),
        None,
    )
    .await?;

    let mut msgs: Vec<Message> = Vec::new();
    // 人格 system 消息必须排在历史之前：插在中间会让模型把「你刚说过的话」
    // 之后又收到一次身份重定义，长会话里表现得不稳定。
    if let Some(text) = persona.instructions.as_deref() {
        msgs.push(Message::system(text));
    }
    for m in &history {
        if m.0 == "user" {
            msgs.push(Message::user(m.1.clone()));
        } else if m.0 == "assistant" && !m.1.trim().is_empty() {
            msgs.push(Message::assistant(m.1.clone()));
        }
    }
    msgs.push(Message::user(content.clone()));

    // 工具往返：从这里开始是一段循环，而不是单次调用。
    //
    // 模型发出 tool_calls 时本轮**没有正文**。此前这里直接取 `answer()`，
    // 而 `has_answer()` 在「只有 tool_calls」时也返回 true —— 于是内容
    // 凭空消失、HTTP 照样 200，没有任何迹象。现在改为：执行工具 → 把结果
    // 以 role=tool 回灌 → 再问一次，直到模型给出真正的正文。
    //
    // 技能目录要在 `state` 被 move 进 Arc 之前取出来。
    let skill_root = crate::api_extensions::skill_dir(&state.config);
    // 角色已经在界面上定下来的会话，不再挂 list_experts / get_expert_detail：
    // 模型不需要（也不该）替用户挑角色，那两个工具只是噪音。实测它们会吃掉
    // 4 轮工具预算里的 3 轮，直接把本来能答的任务压成 tool_loop_exhausted。
    // 见 ISSUE-041。
    let role_already_picked = persona.expert_id.is_some();
    let registry = crate::tools::ToolRegistry::builtin_with_expert_tools(
        Arc::new(state),
        uid,
        !role_already_picked,
    );
    // SKILL 与内置工具在模型看来没有区别：都是 `tools` 字段里的一条。
    let registry = registry
        .with_skills(db.as_ref(), uid, &skill_root)
        .await
        .map_err(|e| ApiError::internal(format!("对话无法开始：{e}")))?;
    // MCP 同理：`tools/list` 报出来的条目走同一个 registry，所以「内置的」与
    // 「MCP 来的」在模型看来没有区别。一台服务器连不上只跳过它，不让整条对话失败。
    let registry = registry
        .with_mcp_tools(db.as_ref(), uid, &crate::tools::user_key(uid))
        .await
        .map_err(|e| ApiError::internal(format!("对话无法开始：{e}")))?;
    let tools = registry.specs();
    let request = crate::llm::build_request(&llm_config, msgs.clone());
    let request = if tools.is_empty() {
        request
    } else {
        request.with_tools(tools.clone())
    };

    let started = std::time::Instant::now();
    let mut reply = provider.chat(&request).await.map_err(provider_failure)?;

    let mut tool_trace: Vec<Value> = Vec::new();
    let mut rounds = 0usize;
    while !reply.tool_calls.is_empty() {
        if rounds >= crate::tools::MAX_TOOL_ROUNDS {
            eprintln!(
                "[chat] 工具往返达到上限 {} 轮，停止；未执行的调用：{:?}",
                crate::tools::MAX_TOOL_ROUNDS,
                reply.tool_calls.iter().map(|c| &c.name).collect::<Vec<_>>()
            );
            break;
        }
        rounds += 1;

        let calls = reply.tool_calls.clone();
        msgs.push(Message::assistant_tool_calls(calls.clone()));
        for call in &calls {
            let result = registry.call(call);
            let ok = result.is_ok();
            match &result {
                Ok(text) => eprintln!("[chat] 工具 {} 执行成功（{} 字符）", call.name, text.len()),
                Err(detail) => eprintln!("[chat] 工具 {} 失败：{detail}", call.name),
            }
            // 渲染一次、存两处：回灌给模型的文本与写进响应的轨迹必须是同一份，
            // 否则界面上显示的与模型实际看到的会不一致。
            let rendered = registry.render_result(call, result);
            tool_trace.push(json!({
                "id": call.id,
                "name": call.name,
                "arguments": call.arguments,
                "ok": ok,
                "result": rendered,
            }));
            msgs.push(Message::tool_result(&call.id, &call.name, rendered));
        }

        let follow_up = crate::llm::build_request(&llm_config, msgs.clone());
        let follow_up = if tools.is_empty() {
            follow_up
        } else {
            follow_up.with_tools(tools.clone())
        };
        reply = provider.chat(&follow_up).await.map_err(provider_failure)?;
    }

    // 轮次用尽、模型仍然只给 tool_calls 没有正文时，**再做一次收尾调用**：
    // 把工具**摘掉**，并明确要求「用手上的信息作答」。
    //
    // 为什么非要这一步：模型其实完全有能力说清「我查到了什么、缺什么」——
    // 同一批 4B 在第 1、2、3、6、9、12 条任务里都主动这么做了。
    // 不做这一步，用户拿到的只是一条 `tool_loop_exhausted` 错误，
    // **一句有用的正文都没有**，而那些信息本来就在上下文里。
    let mut forced_final_answer = false;
    if !reply.tool_calls.is_empty() && reply.answer().trim().is_empty() {
        eprintln!(
            "[chat] 工具往返用尽仍无正文，去掉工具再问一次，强制它作答"
        );
        msgs.push(Message::assistant_tool_calls(reply.tool_calls.clone()));
        msgs.push(Message::user(crate::tools::FINAL_ANSWER_PROMPT.to_string()));
        // **不带 tools**：模型此刻已经证明会一直要工具，再给一次只是再要一轮。
        let final_request = crate::llm::build_request(&llm_config, msgs.clone());
        match provider.chat(&final_request).await {
            Ok(last) => {
                tool_trace.push(json!({
                    "id": "final",
                    "name": crate::tools::FINAL_ANSWER_MARKER,
                    "arguments": json!({ "rounds_exhausted": crate::tools::MAX_TOOL_ROUNDS }),
                    "ok": true,
                    "result": last.answer().to_string(),
                }));
                if !last.answer().trim().is_empty() {
                    reply = last;
                    forced_final_answer = true;
                }
            }
            Err(e) => {
                eprintln!("[chat] 收尾调用失败，保留原来的 tool_loop_exhausted 结论：{e}");
            }
        }
    }
    let turn_ms = started.elapsed().as_millis() as i64;

    // 工具往返用尽后仍只有 tool_calls、没有正文：这是**没有回答**，
    // 不能当成功返回。`has_answer()` 在这种情况下会返回 true（它只判断
    // 「有内容」），若照它走就会存一条空消息并返回 200 —— 又是内容凭空消失。
    //
    // 用 `tool_loop_exhausted` 而不是 `service_unavailable`（ISSUE-027）：
    // 模型这 `rounds` 轮里**每一轮都回过话**，只是没收敛。用
    // `service_unavailable` 会附上「确认端点活着 / 启动 llama-server」——
    // 让用户去查一台正在正常应答的服务，与 detail 里那句
    // 「换个更直接的问法」当场打架。
    if reply.answer().is_empty() && !reply.tool_calls.is_empty() {
        return Err(ApiError::tool_loop_exhausted(format!(
            "模型连续 {} 轮都在请求调用工具，没有给出正文。\
             已执行的工具：{}。\
             下一步：换个更直接的问法，或检查该工具是否满足不了模型的需求。",
            rounds,
            if tool_trace.is_empty() {
                "（无）".to_string()
            } else {
                tool_trace
                    .iter()
                    .map(|t| t["name"].as_str().unwrap_or("?").to_string())
                    .collect::<Vec<_>>()
                    .join("、")
            }
        )));
    }

    let (text, reasoning) = if reply.has_answer() {
        (reply.answer().to_string(), reply.reasoning.clone())
    } else if reply.truncated_by_reasoning() {
        return Err(ApiError::service_unavailable(format!(
            "模型只输出了思考过程，没有正文 —— token 预算被思考吃光了。\
             下一步：调大 QUILL_LLM_MAX_TOKENS（当前 {}）后重启服务，或换一个非推理模型。",
            llm_config.max_tokens
        )));
    } else {
        (String::new(), reply.reasoning.clone())
    };

    let seq_assistant = seq_user + 1;
    let assistant_id = new_id()?;
    let assistant_created_at = append_message(
        &db,
        uid,
        sid,
        &assistant_id,
        seq_assistant,
        "assistant",
        "complete",
        &text,
        Some(&reasoning),
        reply.usage,
        Some(turn_ms),
    )
    .await?;

    touch_session(&db, uid, sid, seq_assistant + 1, reply.usage.input, reply.usage.output).await?;

    Ok(Json(json!({
        "session_id": id,
        "user_message": {
            "id": hex(&user_id16),
            "seq": seq_user,
            "content": content,
            "created_at": user_created_at,
        },
        "reply": text,
        "reasoning": reasoning,
        "message": {
            "id": hex(&assistant_id),
            "seq": seq_assistant,
            "content": text,
            "created_at": assistant_created_at,
        },
        "finish_reason": reply.finish_reason.map(|f| format!("{f:?}")),
        // 缓存两项是 nullable：上游没报就是 null，前端据此决定「命中率」这一格
        // 到底显示数字还是干脆不显示。别把它们 default 成 0。
        "usage": {
            "input": reply.usage.input,
            "output": reply.usage.output,
            "cache_read": reply.usage.cache_read,
            "cache_write": reply.usage.cache_write,
        },
        "turn_ms": turn_ms,
        "persona_applied": persona.instructions.is_some(),
        "expert_notice": persona.notice,
        // 工具执行轨迹。没有工具时是空数组 —— 前端按「长度 0」判定不显示，
        // 不需要另设一个布尔开关（两个字段可能不同步的那种设计最难维护）。
        "tool_calls": tool_trace,
        "tool_rounds": rounds,
        // 这一段正文是**工具用尽之后、把工具摘掉逼出来的**，没有工具支撑。
        // 实测 4B 在这种时候会开始编（ISSUE-043），所以必须让界面知道：
        // 用户看到的必须是一条警告，而不是一段看起来和平时一样可信的回答。
        "final_answer_forced": forced_final_answer,
    }))
    .into_response())
}

type HistoryRow = (String, String);

/// 一条会话解析出来的「人格」结果。
///
/// `sessions.expert_id` 一直被前端选专家时写进去了，但服务端此前从没读过它
/// （选专家等于摆设）。这里补上：instructions 非空才作为 system 消息注入。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionPersona {
    /// 非空才注入；空串与空白串按「没有额外人格」处理（塞一条空 system
    /// 消息纯属给模型发噪声）。
    pub instructions: Option<String>,

    /// 会话上**确实绑定了**的角色 id（空白串按未绑定算）。
    /// 用来判断「角色是不是已经在界面上定过了」—— 定过了就不该再把
    /// `list_experts` / `get_expert_detail` 挂给模型。见 ISSUE-041。
    pub expert_id: Option<String>,

    /// 专家不可用时的中文提示，会原样带进 HTTP 响应的 `expert_notice`。
    /// **不允许静默降级**：用户点了专家却没生效，必须让他知道。
    pub notice: Option<String>,
}

/// 解析会话绑定的专家人格。查不到 / 已软删除时返回空人格 + 中文提示，
/// 并在服务端日志留一条 warning。
async fn resolve_persona(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
) -> Result<SessionPersona, ApiError> {
    let expert_id: Option<String> = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<Option<String>, quill_agent::AgentError> = async {
                    Ok(
                        sqlx::query_scalar(
                            "SELECT expert_id FROM sessions \
                             WHERE user_id = ? AND id = ? AND deleted_at IS NULL",
                        )
                        .bind(uid.as_bytes().to_vec())
                        .bind(sid.to_vec())
                        .fetch_optional(&pool)
                        .await
                        .map_err(|e| crate::db::storage_error("读会话绑定的专家", e))?
                        .flatten(),
                    )
                }
                .await;
                r
            })
        })
        .map_err(storage)?;

    let Some(expert_id) = expert_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    else {
        // 没绑定角色：`expert_id` 为空，人格为空，但**要记住「没绑定」**，
        // 因为这种情况下才需要把 list_experts / get_expert_detail 挂给模型
        // 让它自己挑。见 ISSUE-041。
        return Ok(SessionPersona::default());
    };

    let row: Option<(String, Option<String>)> = db
        .call({
            let expert_id = expert_id.clone();
            move |pool, _rt| {
                Box::pin(async move {
                    let r: Result<Option<(String, Option<String>)>, quill_agent::AgentError> =
                        async {
                            // 用户自建的那份优先于同名内置专家，与 ExpertRegistry
                            // 的去重口径一致（见 list_visible 的 dedup_by）。
                            let row = sqlx::query(
                                "SELECT instructions, model FROM experts \
                                 WHERE id = ? AND deleted_at IS NULL \
                                   AND owner_user_id IN (?, x'00000000000000000000000000000000') \
                                 ORDER BY CASE WHEN owner_user_id = ? THEN 0 ELSE 1 END \
                                 LIMIT 1",
                            )
                            .bind(expert_id.clone())
                            .bind(uid.as_bytes().to_vec())
                            .bind(uid.as_bytes().to_vec())
                            .fetch_optional(&pool)
                            .await
                            .map_err(|e| crate::db::storage_error("读专家人格", e))?;
                            Ok(row.map(|r| {
                                let instructions: String =
                                    sqlx::Row::get(&r, "instructions");
                                let model: Option<String> = sqlx::Row::get(&r, "model");
                                (instructions, model)
                            }))
                        }
                    .await;
                    r
                })
            }
        })
        .map_err(storage)?;

    let Some((instructions, model)) = row else {
        eprintln!(
            "[chat] 会话 {sid:?} 绑定的专家「{expert_id}」不存在或已被软删除：\
             本轮按默认人格回答（不注入任何 system 消息）"
        );
        return Ok(SessionPersona {
            instructions: None,
            expert_id: Some(expert_id.clone()),
            notice: Some(format!(
                "会话绑定的专家「{expert_id}」已不可用（不存在或已被删除），已按默认人格回答。\
                 下一步：在专家页重新选择一个可用专家，或新建会话。"
            )),
        });
    };

    if let Some(m) = model.as_deref() {
        // 专家级 model 覆盖留到接 goose 核心（把专家映射成 goose custom agent、
        // 由 provider 层按 agent 偏好选模型）时做；本轮只落地人格注入，
        // 所以这里明确记录「不切模型」，而不是假装已经支持。
        eprintln!(
            "[chat] 专家「{expert_id}」声明了偏好模型「{m}」，本轮**未**切换模型（仍用实例默认模型）：\
             专家级 model 覆盖留到接 goose 核心时落地"
        );
    }

    Ok(SessionPersona {
        instructions: Some(instructions).filter(|s| !s.trim().is_empty()),
        expert_id: Some(expert_id.clone()),
        notice: None,
    })
}

async fn load_history(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
) -> Result<Vec<HistoryRow>, ApiError> {
    let rows = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<Vec<HistoryRow>, quill_agent::AgentError> = async {
                    let out = sqlx::query(
                        "SELECT role, content FROM messages \
                         WHERE user_id = ? AND session_id = ? AND status = 'complete' \
                         AND role IN ('user','assistant') \
                         ORDER BY seq DESC LIMIT ?",
                    )
                    .bind(uid.as_bytes().to_vec())
                    .bind(sid.to_vec())
                    .bind(HISTORY_LIMIT)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("读历史", e))?;
                    Ok(out
                        .into_iter()
                        .map(|row| (s(&row, "role"), s(&row, "content")))
                        .collect())
                }
                .await;
                r
            })
        })
        .map_err(storage)?;
    let mut rows = rows;
    rows.reverse();
    Ok(rows)
}

async fn next_seq(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
) -> Result<i64, ApiError> {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r: Result<i64, quill_agent::AgentError> = async {
                let m: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(seq),0) FROM messages WHERE user_id = ? AND session_id = ?",
                )
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .fetch_one(&pool)
                .await
                .map_err(|e| crate::db::storage_error("取序号", e))?;
                Ok(m + 1)
            }
            .await;
            r
        })
    })
    .map_err(storage)
}

async fn append_message(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
    mid: &[u8; 16],
    seq: i64,
    role: &str,
    status: &str,
    content: &str,
    reasoning: Option<&str>,
    usage: TokenUsage,
    turn_ms: Option<i64>,
) -> Result<i64, ApiError> {
    let content = content.to_string();
    let reasoning = reasoning.map(String::from);
    let role = role.to_string();
    let status = status.to_string();
    let mid = mid.to_vec();
    let input = i64::from(usage.input.unwrap_or(0));
    let output = i64::from(usage.output.unwrap_or(0));
    // None = 模型端没上报，不是 0。见 migration 0008 的说明。
    let cache_read = usage.cache_read.map(i64::from);
    let cache_write = usage.cache_write.map(i64::from);
    let created_at = now_ms();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r: Result<i64, quill_agent::AgentError> = async {
                sqlx::query(
                    "INSERT INTO messages(user_id,id,session_id,seq,role,status,content,reasoning,\
                     input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,turn_ms,created_at) \
                     VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                )
                .bind(uid.as_bytes().to_vec())
                .bind(mid.clone())
                .bind(sid.to_vec())
                .bind(seq)
                .bind(&role)
                .bind(&status)
                .bind(&content)
                .bind(&reasoning)
                .bind(input)
                .bind(output)
                .bind(cache_read)
                .bind(cache_write)
                .bind(turn_ms)
                .bind(created_at)
                .execute(&pool)
                .await
                .map_err(|e| crate::db::storage_error("存消息", e))?;
                Ok(created_at)
            }
            .await;
            r
        })
    })
    .map_err(storage)
}

async fn touch_session(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
    next_seq: i64,
    input: Option<u32>,
    output: Option<u32>,
) -> Result<(), ApiError> {
    let now = now_ms();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r: Result<(), quill_agent::AgentError> = async {
                sqlx::query(
                    "UPDATE sessions SET next_seq = ?, message_count = message_count + 2, \
                     input_tokens = input_tokens + ?, output_tokens = output_tokens + ?, \
                     last_active_at = ?, updated_at = ? WHERE user_id = ? AND id = ?",
                )
                .bind(next_seq)
                .bind(i64::from(input.unwrap_or(0)))
                .bind(i64::from(output.unwrap_or(0)))
                .bind(now)
                .bind(now)
                .bind(uid.as_bytes().to_vec())
                .bind(sid.to_vec())
                .execute(&pool)
                .await
                .map_err(|e| crate::db::storage_error("更新会话", e))?;
                Ok(())
            }
            .await;
            r
        })
    })
    .map_err(storage)
}

async fn ensure_session(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
) -> Result<(), ApiError> {
    let found = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<bool, quill_agent::AgentError> = async {
                    let n: i64 = sqlx::query_scalar(
                        "SELECT count(*) FROM sessions WHERE user_id = ? AND id = ? AND deleted_at IS NULL",
                    )
                    .bind(uid.as_bytes().to_vec())
                    .bind(sid.to_vec())
                    .fetch_one(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("查会话", e))?;
                    Ok(n > 0)
                }
                .await;
                r
            })
        })
        .map_err(storage)?;
    if found {
        return Ok(());
    }
    Err(ApiError::entity_not_found(
        "会话不存在，或不属于当前用户。下一步：先 POST /api/sessions 建一个，再发消息。".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ISSUE-020 的回归：**信封里「下一步」只能有一个出口。**
    ///
    /// `Page.tsx:65-70` 把 `detail` 与 `next_step` 渲染成两个独立段落。
    /// detail 曾经用 `format!("{e}")` 拼出 `ProviderError` 的 `Display`，
    /// 而 `Display` 会追加「→ 下一步：…」，于是界面上出现两条自称「下一步」的段落。
    fn status_error() -> ApiError {
        provider_failure(quill_provider::ProviderError::Status {
            code: 400,
            body: r#"{"error":{"type":"exceed_context_size_error","n_prompt_tokens":8525}}"#
                .into(),
        })
    }

    #[test]
    fn detail_carries_no_next_step_so_the_envelope_shows_only_one() {
        let e = status_error();
        assert!(
            !e.detail().contains("下一步"),
            "detail 里不该再出现「下一步」，它会和结构化 next_step 渲染成两个段落：{}",
            e.detail()
        );
        // 上游的原话必须原样留着 —— 那是用户唯一能照着改的权威依据。
        assert!(
            e.detail().contains("n_prompt_tokens"),
            "detail 必须带上游原话：{}",
            e.detail()
        );
    }

    #[test]
    fn the_structured_next_step_is_the_one_that_survives() {
        let e = status_error();
        assert_eq!(e.code(), "provider_rejected");
        let n = e.next_step();
        assert!(n.contains("下一步"), "结构化 next_step 要自带标签：{n}");
        assert!(
            !n.contains("llama-server"),
            "模型回过话了，不该再劝用户去重启它：{n}"
        );
    }

    /// 别把 ISSUE-020 修成另一个 bug：把 detail 里的下一步去掉之后，
    /// **结构化那句必须还在，而且内容不能退化。**
    #[test]
    fn unreachable_still_tells_the_user_how_to_check_the_endpoint() {
        let e = provider_failure(quill_provider::ProviderError::Unreachable {
            url: "http://127.0.0.1:18080/v1/chat/completions".into(),
            detail: "connection refused".into(),
        });
        assert!(!e.detail().contains("下一步"), "detail：{}", e.detail());
        assert!(
            e.next_step().contains("确认端点活着"),
            "真连不上时这句必须还在：{}",
            e.next_step()
        );
    }
}
