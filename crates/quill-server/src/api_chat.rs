use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use quill_provider::{
    ChatRequest, ChatResponse, Message, SharedProvider, StreamDelta, TokenUsage, ToolCall, ToolSpec,
};

use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::db::now_ms;
use crate::error::ApiError;
use crate::state::AppState;
use quill_adapters::ids::{to_hex_lower, to_hex_upper};

pub const MAX_CONTENT_LEN: usize = 32_000;
pub const HISTORY_LIMIT: i64 = 40;

/// 会话标题的字符上限。`create` 与 `rename`（Q051）**共用这一个数** ——
/// 两处各写一个 64 的话，改了一处另一处就漂了，而漂的方式是「建会话时截到 64、
/// 改名时截到 80」这种没人会发现的不一致。
pub const TITLE_MAX_CHARS: usize = 64;

fn new_id() -> Result<[u8; 16], ApiError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b)
        .map_err(|e| ApiError::internal(format!("生成会话标识失败（系统随机源不可用）：{e}")))?;
    if b == [0u8; 16] {
        b[0] = 1;
    }
    Ok(b)
}

/// `GET /api/sessions` 的查询参数。
///
/// 只有一个字段，但**不能省**：建团队时 `POST /api/teams` 会顺带插一条
/// `kind='team_leader'` 的会话（`teams.leader_session_id` 是 NOT NULL 且外键
/// 指向 sessions，而派工时 `POST /api/dispatch` 强制要 `leader_session_id`）。
/// 那条会话是**派工记账的落点**，删不掉也不该删。
///
/// 它出现在用户侧栏里却是另一回事：前端没有任何 `team_leader` 字样，
/// 不知道这个 kind，于是一律当普通对话渲染 —— 用户看到的是「建个团队
/// 凭空多出一会话」。
///
/// 所以过滤放在**后端**：那条会话跑起来之后真的有消息，前端藏起来会让
/// 侧栏计数和实际数量对不上，而「显示的东西必须有真来源」是这条仓库的纪律。
///
/// 格式是**逗号分隔**（`?exclude_kind=team_leader,member`）而不是重复键：
/// `serde_urlencoded` 把重复键解成 `Vec` 需要写成 `?exclude_kind[]=a`，
/// 方括号在 URL 里既不干净也不好手拼。逗号是这类过滤参数更省事的写法。
///
/// 「这整套 team_leader 机制是自创的」—— goose 完全没有多智能体，
/// octop 的 team 只是专家名册、没有「给团队开会话」这层。详见 BACKLOG B2-3。
#[derive(Debug, Default, Clone, Deserialize)]
pub struct SessionListQuery {
    /// 要排除的 kind，逗号分隔。侧栏传 `team_leader` 即可。
    #[serde(default)]
    pub exclude_kind: Option<String>,
}

impl SessionListQuery {
    /// 拆成可绑定的列表。空串与全空白都当「不过滤」——
    /// `?exclude_kind=` 不该退化成 `NOT IN ('')` 而把整张表清空。
    fn excluded(&self) -> Vec<String> {
        self.exclude_kind
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    }
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<SessionListQuery>,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;
    let excluded = q.excluded();
    let rows = crate::chat_repo::list_sessions(db, uid, excluded).map_err(storage)?;
    let rows: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "kind": r.kind,
                "room_id": r.room_id,
                "title": r.title,
                "expert_id": r.expert_id,
                "provider_id": r.provider_id,
                "model": r.model,
                "state": r.state,
                "message_count": r.message_count,
                "created_at": r.created_at,
                "last_active_at": r.last_active_at,
            })
        })
        .collect();

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
        ProviderError::Status { .. } => {
            ApiError::provider_rejected(detail, ADVICE_REJECTED_BY_STATUS)
        }
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

// 取列助手 `s` / `n` 与可空列助手 `nullable_n` 已随 Q006c 删除：
// 本文件里最后几个用它们的读路径（metrics / usage / load_history）都搬进了
// `chat_repo`，取列在那边的 `db::{col_str, col_i64}` 里做。

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
        .take(TITLE_MAX_CHARS)
        .collect::<String>();

    let id = new_id()?;
    let now = now_ms();
    let room = format!("room-{}", &to_hex_lower(&id)[..12]);
    let workspace = format!("ws/{}", &to_hex_lower(&id)[..12]);
    let model = state
        .llm_config
        .read()
        .map_err(|_| ApiError::internal("llm_config 锁被毒化"))?
        .model
        .clone();

    let room_out = room.clone();
    let title_out = title.clone();
    let model_out = model.clone();
    let hex_id = to_hex_upper(&id);
    let expert_for_write = expert_id.clone();

    crate::chat_repo::insert_solo_session(
        db,
        uid,
        crate::chat_repo::NewSoloSession {
            id,
            room_id: room.clone(),
            title: title.clone(),
            model: model.clone(),
            workspace_path: workspace.clone(),
            now,
        },
        "建会话",
    )
    .map_err(storage)?;
    // 没指定专家时 `expert_for_write` 是空串，等价于不挂。
    if !expert_for_write.is_empty() {
        crate::chat_repo::set_expert(db, uid, id, expert_for_write).map_err(storage)?;
    }

    // 建会话时就体检一次绑定：调用方指定了专家但那个专家不存在/已删的会话照样建
    // （用户可能只是想聊天），但必须在响应里明说「你选的专家没生效」，
    // 不能等发消息时才让用户自己发现。
    // 没指定的那些已经在 `resolve_for_session` 里换成通用专家了，所以这里恒有值。
    let persona = resolve_persona(db, uid, id).await?;

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

// id → hex 的转换统一在 `quill_adapters::ids::{to_hex_upper, to_hex_lower}`：
// 大写对应 SQLite 的 `hex()`（读路径走它，写路径必须一致）；小写用于
// room / workspace 短标识。**不再各写一份** —— 两处口径漂了，同一个 id 会在
// POST 响应与 GET 列表里变成两种字符串，前端按 id 去重/匹配会全部失配。

/// 读库拿回来的 id 是 `Vec<u8>`（长度未必 16）。长度不对就返回空串 ——
/// 长度不对的 id 本来就拼不出合法会话路由，与其编一个 id 不如空着。
fn hex16(b: &[u8]) -> String {
    match <[u8; 16]>::try_from(b) {
        Ok(arr) => to_hex_upper(&arr),
        Err(_) => String::new(),
    }
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;
    let sid = parse_id(&id)?;
    let found = crate::chat_repo::session_exists(db, uid, sid).map_err(storage)?;

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
    let prior = crate::chat_repo::session_deleted_at(db, uid, sid).map_err(storage)?;

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
    let affected = crate::chat_repo::soft_delete_session(db, uid, sid, now).map_err(storage)?;

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

/// `PATCH /api/sessions/{id}` —— 改会话名（queue Q051）。
///
/// **为什么要有它**：这条路由此前只挂了 GET/DELETE，PATCH 落进 405 —— 界面上
/// 「重命名」没有任何后端出口，会话建完名字就再也改不了（前端按实情提示
/// 「未接通」，但那是缺口的说明，不是功能）。
///
/// 口径：
/// - 只改 `title`，长度与 `create` 同一上限（[`TITLE_MAX_CHARS`]，按**字符**截断
///   而不是字节：按字节截会把中文截成半个字）；
/// - `trim` 后为空一律拒（与 Q007 的取值助手规范语义一致）——侧栏里一行没有
///   名字的会话等于让人认不出来；
/// - 不存在 / 别人的 / 已软删 → 404，与 `get_one` / `delete` 同一个口径；
/// - `last_active_at` 不动（见 `chat_repo::RENAME_SQL` 的注释）。
pub async fn rename(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<axum::response::Response, ApiError> {
    crate::api_experts::only_keys(&body, &["title"], "PATCH /api/sessions/{id}")?;
    let title: String = crate::jsonx::need_str(&body, "title", "改会话名请求")?
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect();

    let db = state.db()?;
    let uid = user.0.user_id;
    let sid = parse_id(&id)?;
    ensure_session(db, uid, sid).await?;

    let affected =
        crate::chat_repo::rename_session(db, uid, sid, title.clone(), now_ms()).map_err(storage)?;
    if affected == 0 {
        // 存在性检查与 UPDATE 之间被并发软删：如实报 404，不假装改成功。
        return Err(ApiError::entity_not_found(format!(
            "会话 {id} 在改名的瞬间被删除了，本次未改。\
             下一步：刷新会话列表，确认它是否还在；不在就说明已删除。"
        )));
    }

    Ok(Json(json!({
        "id": id,
        "title": title,
        "renamed": true,
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
    let rows = crate::chat_repo::list_messages(db, uid, sid).map_err(storage)?;
    let rows: Vec<Value> = rows
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "seq": m.seq,
                "role": m.role,
                "status": m.status,
                "content": m.content,
                "reasoning": m.reasoning,
                "input_tokens": m.input_tokens,
                "output_tokens": m.output_tokens,
                "turn_ms": m.turn_ms,
                "error_code": m.error_code,
                "created_at": m.created_at,
            })
        })
        .collect();

    Ok(Json(json!({ "messages": rows })).into_response())
}

/// 上下文窗口占用。抄 octop 的 `ContextWindowRing` 的数据源。
///
/// **环的总占用是实测的**：`used_tokens` 取该会话最后一条 assistant 消息的
/// `input_tokens` —— 那正是模型端真实收到的入参大小。`max_tokens` 取配置值。
///
/// **分段是字符数，不是 token 数。** quill 没有分词器，把字符说成 token 就是
/// 凭空造数字（见 [`crate::session_metrics`] 里的说明）。所以分段只表达
/// 「谁占得多」，单位在响应里明写 `segment_unit: "chars"`。
pub async fn context(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;
    let sid = parse_id(&id)?;
    ensure_session(db, uid, sid).await?;

    // 实测入参：最后一条 assistant 消息的 input_tokens。
    let used_tokens =
        crate::chat_repo::last_assistant_input_tokens(db, uid, sid).map_err(storage)?;

    // 构成：按与聊天路径**完全相同**的方式重建一遍工具表，然后按增量切片。
    // 复用真实的 builder 而不是另写一份统计 —— 另写一份必然与实际发送的请求漂移。
    //
    // `state` 会在下面被 move 进 registry，所以先把它要用的 llm_config 读出来。
    // 上限取运行时配置（与 /healthz 的 max_context_tokens 同一个来源），
    // 不重新读库 —— 读出来一份可能和运行时热替换后的值不一致。
    let max_tokens: i64 = state
        .llm_config
        .read()
        .map_err(|_| ApiError::internal("llm_config 锁被毒化"))?
        .max_context_tokens
        .into();
    let persona = resolve_persona(db, uid, sid).await?;
    let skill_root = crate::api_extensions::skill_dir(&state.config);
    let role_already_picked = persona.expert_id.is_some();
    // 工具素材走端口：专家 / SKILL / MCP 三份清单由 `DbToolSources` 转发到
    // 既有查询函数（逐个见该模块的表格）。
    let sources = crate::tool_sources::DbToolSources::shared(&state, skill_root);
    let registry = crate::tools::ToolRegistry::builtin_with_expert_tools(
        // `Arc::clone` 而不是把 `state` move 进来：这之后还要用 `db` 读历史
        // （`load_history_chars`），而且 `sources` 后面还要交给 `with_skills` /
        // `with_mcp_tools`。AppState 内部全是 Arc，这里那次克隆是廉价的。
        Arc::clone(&sources),
        uid,
        !role_already_picked,
    );
    let builtin_specs = registry.specs();
    let n_builtin = builtin_specs.len();
    let registry = registry
        .with_skills(sources.as_ref(), uid)
        .await
        .map_err(|e| ApiError::internal(format!("统计上下文时加载 SKILL 失败：{e}")))?;
    let skill_specs = registry.specs();
    let n_skills = skill_specs.len();
    let all_specs = registry
        .with_mcp_tools(sources.as_ref(), uid, &crate::tools::user_key(uid))
        .await
        .map_err(|e| ApiError::internal(format!("统计上下文时加载 MCP 失败：{e}")))?
        .specs();

    // `with_skills` / `with_mcp_tools` 是往同一个 Vec 尾部追加的，所以按长度切片
    // 就是它们的来源归属 —— 不需要在工具表上另挂一个 origin 字段。
    let sum_chars = |slice: &[quill_provider::ToolSpec]| {
        slice
            .iter()
            .map(crate::session_metrics::tool_spec_chars)
            .sum()
    };
    let breakdown = crate::session_metrics::ContextBreakdown {
        system_prompt: persona
            .instructions
            .as_deref()
            .map(|s| s.chars().count())
            .unwrap_or(0),
        tool_definitions: sum_chars(&builtin_specs),
        skills: sum_chars(&skill_specs[n_builtin..]),
        mcp: sum_chars(&all_specs[n_skills..]),
        conversation: load_history_chars(db, uid, sid).await?,
    };

    let segments: Vec<Value> = crate::session_metrics::context_segments(&breakdown)
        .into_iter()
        .map(|s| json!({ "key": s.key, "chars": s.chars }))
        .collect();

    Ok(Json(json!({
        "max_tokens": max_tokens,
        // None = 还没跟模型说过话，没有实测值。**不是 0。**
        "used_tokens": used_tokens,
        "used_percent": used_tokens
            .map(|u| crate::session_metrics::context_used_percent(u, max_tokens)),
        "segments": segments,
        // 明写单位。界面上必须显示这个「字符」二字，否则用户会当成 token。
        "segment_unit": "chars",
    }))
    .into_response())
}

/// 历史消息的字符总数（对话段）。
async fn load_history_chars(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
) -> Result<usize, ApiError> {
    crate::chat_repo::dialog_content_chars(db, uid, sid).map_err(storage)
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

    let rows = crate::chat_repo::usage_rows(db, uid, sid).map_err(storage)?;
    let rows: Vec<crate::session_metrics::MessageUsage> = rows.iter().map(usage_of).collect();

    Ok(Json(crate::session_metrics::aggregate(&rows).to_json()).into_response())
}

/// 一次最多统计多少个会话。
///
/// 多取一个（`LIMIT 201`）只为判断「有没有被截断」——被截断时接口会明说，
/// 而不是让用户以为这就是全部。
pub const USAGE_SESSIONS_LIMIT: i64 = 200;

/// 全部会话的用量统计。左侧导航「用量统计」页的数据源。
///
/// **口径全部复用 [`crate::session_metrics::aggregate`]**，没有在 SQL 里另写一套
/// 汇总规则 —— 那些规则（缺一条耗时就不给、没上报缓存就不报比率…）写两遍必然漂移。
/// 这里只把行取出来、按会话分组喂给它。合并总计也是同一个函数跑一遍全量行，
/// 所以「总计」和「每行加起来」必然一致。
/// `GET /api/usage` 的行集合：会话行 + 每会话的 `(消息 id, 用量)`。
/// 两块都来自 `chat_repo`（queue Q006c）：HTTP 层不再内联 SQL。
pub async fn usage(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<axum::response::Response, ApiError> {
    let db = state.db()?;
    let uid = user.0.user_id;

    let sessions =
        crate::chat_repo::usage_sessions(db, uid, USAGE_SESSIONS_LIMIT + 1).map_err(storage)?;
    let ids: Vec<Vec<u8>> = sessions.iter().map(|s| s.id.clone()).collect();
    let rows = crate::chat_repo::usage_rows_for_sessions(db, uid, ids).map_err(storage)?;

    // 多取的那一条在这里被丢掉 —— 丢掉之前先记住「确实被截断了」。
    let truncated = sessions.len() > USAGE_SESSIONS_LIMIT as usize;
    let mut sessions = sessions;
    if truncated {
        sessions.truncate(USAGE_SESSIONS_LIMIT as usize);
    }

    // 按会话分组。同一个纯函数，各组各调一次；总计跑一遍全量。
    let mut per_session: std::collections::HashMap<
        Vec<u8>,
        Vec<crate::session_metrics::MessageUsage>,
    > = std::collections::HashMap::new();
    let mut all_rows: Vec<crate::session_metrics::MessageUsage> = Vec::new();
    for (sid, usage) in &rows {
        per_session
            .entry(sid.clone())
            .or_default()
            .push(usage_of(usage));
        all_rows.push(usage_of(usage));
    }

    let totals = crate::session_metrics::aggregate(&all_rows);
    let list: Vec<Value> = sessions
        .iter()
        .map(|row| {
            let empty = Vec::new();
            let bucket = per_session.get(&row.id).unwrap_or(&empty);
            json!({
                "id": hex16(&row.id),
                "title": row.title,
                "expert_id": row.expert_id,
                "last_active_at": row.last_active_at,
                "metrics": crate::session_metrics::aggregate(bucket).to_json(),
            })
        })
        .collect();

    Ok(Json(json!({
        "totals": totals.to_json(),
        "sessions": list,
        "session_count": list.len(),
        "limit": USAGE_SESSIONS_LIMIT,
        // 被截断时前端必须说出来：把「只统计了前 200 个」当成「一共就这些」
        // 是一句凭空而来的话。
        "truncated": truncated,
    }))
    .into_response())
}

/// 库里的用量原始列 → 指标模块的口径。只做这一件事，所以留在 HTTP 层：
/// 聚合规则（缺一列怎么办、哪些角色算数）在 `session_metrics`，那边是纯函数、测得到。
fn usage_of(r: &crate::chat_repo::MessageUsageRow) -> crate::session_metrics::MessageUsage {
    crate::session_metrics::MessageUsage {
        is_user: r.role == "user",
        is_assistant: r.role == "assistant",
        input_tokens: r.input_tokens,
        output_tokens: r.output_tokens,
        turn_ms: r.turn_ms,
        // 可空列必须区分「没上报」和「上报了 0」。
        cache_read_tokens: r.cache_read_tokens,
    }
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

/// 会话 hex id。通道侧要拿着它给 `prepare_turn`，所以出口必须是**大写** ——
/// 与 GET 列表读路径一致（见本文件 `hex` 的注释：两边口径不同会让前端失配）。
pub(crate) fn session_hex(id: &[u8; 16]) -> String {
    to_hex_upper(id)
}

/// `parse_id` 的通道侧出口。**报错文案不同**：通道侧拿到非法 id 时的处置是
/// 「丢掉重建」，不是「叫用户去建会话」，所以不该复用那个带 HTTP 引导的
/// bad_request —— 那会让服务端日志里出现一条误导性的用户指引。
pub(crate) fn parse_public_id(raw: &str) -> Result<[u8; 16], ()> {
    quill_domain::SessionId::parse(raw)
        .map(|s| *s.as_bytes())
        .map_err(|_| ())
}

/// 通道用：这条会话还在不在。
///
/// 与 `ensure_session` 刻意分成两个函数：`ensure_session` 缺失时报错并叫用户
/// 去建会话，而通道那边会话可能被用户删了，得当成「要新建」而不是故障。
pub(crate) async fn session_exists(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
) -> Result<bool, ApiError> {
    let found = crate::chat_repo::session_exists(db, uid, sid).map_err(storage)?;
    Ok(found)
}

/// 通道用：为一个外部通道来件建一条 `solo` 会话。
///
/// **不复用 `create`**：那个处理器是 HTTP 端点，要走 `JsonBody` 与响应封装，
/// 而通道是在后台任务里调的。
///
/// **不挂角色**：`create` 之所以会挂通用专家（`resolve_for_session`），是因为
/// 网页侧栏按角色分组、不挂就归进「默认（未选角色）」，用户会以为自己在跟
/// 某个角色说话其实没有。通道来件没有那个界面，也不该替用户选角色 ——
/// 真要指定，由用户在通道设置里选，存进 config。
pub(crate) async fn create_session_for_channel(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    title: &str,
) -> Result<[u8; 16], ApiError> {
    let id = new_id()?;
    let now = now_ms();
    let short = &to_hex_lower(&id)[..12];
    let room = format!("room-{short}");
    let workspace = format!("ws/{short}");
    let title = title.chars().take(64).collect::<String>();

    crate::chat_repo::insert_solo_session(
        db,
        uid,
        crate::chat_repo::NewSoloSession {
            id,
            room_id: room,
            title,
            model: String::new(),
            workspace_path: workspace,
            now,
        },
        "为通道建会话",
    )
    .map_err(storage)?;

    Ok(id)
}

/// 一轮对话（可能含多次模型调用）的 token 用量累加器。
///
/// ## 为什么需要它
///
/// 工具往返那段循环每轮都 `reply = provider.chat(&follow_up)`，**只把最后一轮的
/// `reply.usage` 存进 messages**。于是「这一轮」在统计条上只剩最后一次调用的数：
/// 实测 50 条真实任务里有 34 条以工具轮收场，而工具轮恰恰是入参最大的一类 ——
/// 用户拿这个数字判断「技能是不是把上下文撑爆了」，少报一半正好报在要命的地方。
///
/// ## 累加规则
///
/// 1. **只加真值**（`session_metrics::sum_reported` 的同一条规矩：「token 可加，
///    有一条真值即可」）。一条都没报就是 `None` —— 不能因为求和就凭空造出 0，
///    那会让界面显示「这次聊天一点没花 token」。
/// 2. **缓存两项不加进入参**。goose 口径里 `cache_read` / `cache_write` 是
///    `input` 的**子集**（见 migration 0008），把它们并进 `input` 会算出 >100% 的
///    命中率。求和是**逐项**求和，语义不变。
/// 3. `cache_read` 求和后**不允许超过 `input`**。越界只可能来自「某几轮报了
///    `input=0`/`None`、另一轮报了 `cache_read`」这种半真值组合，夹到 `input`
///    上，命中率就永远落在 100% 以内。诚实的上游每轮都满足子集关系，求和后
///    必然也满足，所以这条夹取**只**动得了异常上报。
/// 4. **单轮逐字不变**（`rounds <= 1` 时不夹）。不带工具的那一轮仍然原样存上游
///    报来的数：那是上游的口径，不在这一层替它改写。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TurnUsage {
    total: TokenUsage,
    /// 这一轮里一共调了几次模型。第 3 条的夹取只对「真的求和过」的情形生效，
    /// 靠它把单轮和多轮区分开。
    rounds: u32,
}

impl TurnUsage {
    /// 记入一次模型调用报上来的 usage。
    pub fn push(&mut self, usage: TokenUsage) {
        add_reported(&mut self.total.input, usage.input);
        add_reported(&mut self.total.output, usage.output);
        add_reported(&mut self.total.cache_read, usage.cache_read);
        add_reported(&mut self.total.cache_write, usage.cache_write);
        self.rounds = self.rounds.saturating_add(1);
    }

    /// 这一轮的总量。存档与响应 JSON 都用它，保证两边是同一份数。
    pub fn finish(self) -> TokenUsage {
        let mut total = self.total;
        if self.rounds > 1 {
            if let (Some(input), Some(read)) = (total.input, total.cache_read) {
                total.cache_read = Some(read.min(input));
            }
        }
        total
    }
}

/// 只在有真值时累加；`None` 保持 `None`（「没报」不等于「报了个 0」）。
fn add_reported(acc: &mut Option<u32>, value: Option<u32>) {
    if let Some(v) = value {
        *acc = Some(acc.unwrap_or(0).saturating_add(v));
    }
}

/// 校验并取出发送内容。两个入口共用，否则流式那条路很容易漏掉长度上限。
pub(crate) fn take_content(body: &Value) -> Result<String, ApiError> {
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
    Ok(content)
}

/// 存用户消息 → 调模型 → 存助手消息 → 返回。
///
/// 这条路由**不改语义**：它仍然等模型把整段回完再一次性返回 JSON。
/// 流式是另一条路由 `POST /api/sessions/{id}/messages/stream`，两者共用
/// `prepare_turn` / `run_turn` / `finish_turn` 这三段，不复制第二份循环。
pub async fn post_message(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<axum::response::Response, ApiError> {
    let content = take_content(&body)?;
    let mut prep = prepare_turn(state, user, id, content).await?;

    let mut sink = NullSink;
    let outcome = run_turn(&mut prep, ReplyMode::Once, &mut sink).await?;

    Ok(Json(finish_turn(&prep, outcome).await?).into_response())
}

/// 一次「发消息」里**与模型怎么答无关**的那部分准备结果。
///
/// 拆它的理由：一次性路由与 SSE 路由必须跑**同一份**工具循环。两份循环
/// 迟早只改一边，而「一次性那条还能用、流式那条坏了」恰好是最难发现的错法
/// —— 前端全切到流式之后，老路由根本没人碰了。
pub(crate) struct TurnPrep {
    pub(crate) provider: SharedProvider,
    pub(crate) llm_config: crate::llm::LlmConfig,
    pub(crate) registry: crate::tools::ToolRegistry,
    pub(crate) tools: Vec<ToolSpec>,
    msgs: Vec<Message>,
    /// `state.db()` 交出来的是 `Arc`；这里保持 Arc，SSE 那条路要把
    /// 整份准备结果 move 进后台任务。
    pub(crate) db: Arc<crate::db::DbBridge>,
    pub(crate) uid: quill_domain::UserId,
    pub(crate) sid: [u8; 16],
    pub(crate) session_id: String,
    pub(crate) content: String,
    pub(crate) user_id: [u8; 16],
    pub(crate) seq_user: i64,
    pub(crate) user_created_at: i64,
    pub(crate) persona: SessionPersona,
}

/// 校验内容 → 装 provider → 取历史与人格 → 落用户消息 → 拼 messages → 装工具表。
pub(crate) async fn prepare_turn(
    state: AppState,
    user: AuthUser,
    id: String,
    content: String,
) -> Result<TurnPrep, ApiError> {
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

    let user_id = new_id()?;
    let user_created_at = append_message(
        &db,
        uid,
        sid,
        &user_id,
        MessageBody {
            seq: seq_user,
            role: "user",
            status: "complete",
            content: &content,
            reasoning: None,
            // 用户这一轮没调用模型，没有 token 概念。
            usage: TokenUsage::default(),
            turn_ms: None,
        },
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
    // 技能目录要在 `state` 被 move 进端口之前取出来。
    let skill_root = crate::api_extensions::skill_dir(&state.config);
    // 角色已经在界面上定下来的会话，不再挂 list_experts / get_expert_detail：
    // 模型不需要（也不该）替用户挑角色，那两个工具只是噪音。实测它们会吃掉
    // 4 轮工具预算里的 3 轮，直接把本来能答的任务压成 tool_loop_exhausted。
    // 见 ISSUE-041。
    let role_already_picked = persona.expert_id.is_some();
    // 工具素材走端口：专家 / SKILL / MCP 三份清单由 `DbToolSources` 转发到
    // 既有查询函数（逐个见该模块的表格）。
    let sources = crate::tool_sources::DbToolSources::shared(&state, skill_root);
    let registry = crate::tools::ToolRegistry::builtin_with_expert_tools(
        Arc::clone(&sources),
        uid,
        !role_already_picked,
    );
    // SKILL 与内置工具在模型看来没有区别：都是 `tools` 字段里的一条。
    let registry = registry
        .with_skills(sources.as_ref(), uid)
        .await
        .map_err(|e| ApiError::internal(format!("对话无法开始：{e}")))?;
    // MCP 同理：`tools/list` 报出来的条目走同一个 registry，所以「内置的」与
    // 「MCP 来的」在模型看来没有区别。一台服务器连不上只跳过它，不让整条对话失败。
    let registry = registry
        .with_mcp_tools(sources.as_ref(), uid, &crate::tools::user_key(uid))
        .await
        .map_err(|e| ApiError::internal(format!("对话无法开始：{e}")))?;

    Ok(TurnPrep {
        tools: registry.specs(),
        registry,
        provider,
        llm_config,
        msgs,
        db,
        uid,
        sid,
        session_id: id,
        content,
        user_id,
        seq_user,
        user_created_at,
        persona,
    })
}

/// 循环跑完、还没落库也还没拼响应的东西。
pub(crate) struct TurnOutcome {
    reply: ChatResponse,
    usage: TokenUsage,
    tool_trace: Vec<Value>,
    rounds: usize,
    forced_final_answer: bool,
    turn_ms: i64,
}

/// 一轮模型调用里「发生了什么」的出口。
///
/// 一次性那条路用 `NullSink`（什么都不做，行为与接入流式之前逐字节一致），
/// SSE 那条路把它换成往外发事件的实现。**循环本身只有一份。**
///
/// `Send` 是 supertrait 而非可选：SSE 那条路要把整个 future 交给后台任务，
/// 没有它编译不过。
pub(crate) trait RoundSink: Send {
    /// 新一轮模型调用开始，`round` 从 0 起。
    fn round_start(&mut self, _round: usize) {}
    /// 正文增量。
    fn text(&mut self, _delta: &str) {}
    /// 思考增量。它不是答案，展示时必须与正文分开。
    fn reasoning(&mut self, _delta: &str) {}
    /// 模型请求调用工具。
    fn tool_call(&mut self, _call: &ToolCall) {}
    /// 工具执行完毕。**只带成败，不带结果正文**：全文在 `done` 事件的
    /// `tool_calls` 里，每个增量都抄一遍会把一帧撑到几千字符。
    fn tool_result(&mut self, _call: &ToolCall, _ok: bool) {}
    /// 这一轮的正文/思考会被后面的调用覆盖掉，现在丢弃。
    fn discard(&mut self, _round: usize) {}
}

/// 一次性路由用的空出口。
pub(crate) struct NullSink;
impl RoundSink for NullSink {}

/// 一轮模型调用怎么发出去。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReplyMode {
    /// 等模型把整段回包给完再返回（老路由，语义不变）。
    Once,
    /// 增量一到就往外发；一个字都没吐出来就失败时退回 `Once`。
    Streamed,
}

fn build_request(prep: &TurnPrep, msgs: &[Message]) -> ChatRequest {
    let request = crate::llm::build_request(&prep.llm_config, msgs.to_vec());
    if prep.tools.is_empty() {
        request
    } else {
        request.with_tools(prep.tools.clone())
    }
}

async fn one_round(
    prep: &TurnPrep,
    request: &ChatRequest,
    mode: ReplyMode,
    sink: &mut dyn RoundSink,
) -> Result<ChatResponse, ApiError> {
    match mode {
        ReplyMode::Once => prep.provider.chat(request).await.map_err(provider_failure),
        ReplyMode::Streamed => streamed_round(prep, request, sink).await,
    }
}

/// 一次流式调用。**吐不出任何增量时的失败一律退回一次性调用。**
///
/// 退路是必需的：不少 OpenAI 兼容端点对 `stream: true` 的支持并不完整
/// （老版本 llama.cpp、部分网关直接回 400 或干脆回一整段 JSON）。
/// 流式是锦上添花，不能让它把「聊天」本身变成不可用。
async fn streamed_round(
    prep: &TurnPrep,
    request: &ChatRequest,
    sink: &mut dyn RoundSink,
) -> Result<ChatResponse, ApiError> {
    let mut emitted = 0usize;
    let result = match prep.provider.stream(request).await {
        Ok(stream) => {
            let mut forward = |delta: StreamDelta| {
                emitted += 1;
                match &delta {
                    StreamDelta::Text(t) => sink.text(t),
                    StreamDelta::Reasoning(r) => sink.reasoning(r),
                    // 工具调用**不在这里让出去**：循环拿到整条回复之后自己发
                    // 一次。在这儿也发一遍就会每个工具调用出现两帧。
                    StreamDelta::ToolCall(_) => {}
                    // `Done` 只是收尾摘要，没有新内容可显示。
                    StreamDelta::Done(_) => {}
                }
            };
            quill_provider::pump_stream(stream, &request.model, &mut forward).await
        }
        Err(e) => Err(e),
    };
    match result {
        Ok(reply) => Ok(reply),
        Err(e) if emitted == 0 => {
            eprintln!("[chat] 流式一个字都没吐就失败（{e}），退回一次性调用");
            prep.provider.chat(request).await.map_err(provider_failure)
        }
        // 已经吐过字了：再补一次一次性调用会让同一段话在界面上出现两遍，
        // 那比报错更糟。老实报错。
        Err(e) => Err(provider_failure(e)),
    }
}

/// 这一轮的可见内容会被后面的调用覆盖掉 —— 现在就告诉上层丢弃。
///
/// 不发这个信号，用户会看着一段已经显示出来的文字中途消失，以为是界面坏了。
fn discard_if_visible(reply: &ChatResponse, sink: &mut dyn RoundSink, round: usize) {
    if !reply.answer().is_empty() || !reply.reasoning.trim().is_empty() {
        sink.discard(round);
    }
}

/// 工具往返循环。一次性与 SSE 两条路由跑的都是这一段。
pub(crate) async fn run_turn(
    prep: &mut TurnPrep,
    mode: ReplyMode,
    sink: &mut dyn RoundSink,
) -> Result<TurnOutcome, ApiError> {
    let started = std::time::Instant::now();
    // 这一轮**所有**模型调用的 usage 都记在这里，而不是只留最后一轮 ——
    // 工具往返的每一轮都真花了入参，漏掉它们统计条就只会报最后那次。
    let mut turn_usage = TurnUsage::default();
    // msgs 由本函数取走跑循环，之后没有别的读者。
    let mut msgs = std::mem::take(&mut prep.msgs);

    sink.round_start(0);
    let request = build_request(prep, &msgs);
    let mut reply = one_round(prep, &request, mode, sink).await?;
    turn_usage.push(reply.usage);

    let mut tool_trace: Vec<Value> = Vec::new();
    let mut rounds = 0usize;
    let mut round_no = 0usize;
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
        discard_if_visible(&reply, sink, round_no);

        let calls = reply.tool_calls.clone();
        msgs.push(Message::assistant_tool_calls(calls.clone()));
        for call in &calls {
            sink.tool_call(call);
            let result = prep.registry.call(call);
            let ok = result.is_ok();
            match &result {
                Ok(text) => eprintln!("[chat] 工具 {} 执行成功（{} 字符）", call.name, text.len()),
                Err(detail) => eprintln!("[chat] 工具 {} 失败：{detail}", call.name),
            }
            // 渲染一次、存两处：回灌给模型的文本与写进响应的轨迹必须是同一份，
            // 否则界面上显示的与模型实际看到的会不一致。
            let rendered = prep.registry.render_result(call, result);
            tool_trace.push(json!({
                "id": call.id,
                "name": call.name,
                "arguments": call.arguments,
                "ok": ok,
                "result": rendered,
            }));
            msgs.push(Message::tool_result(&call.id, &call.name, rendered));
            sink.tool_result(call, ok);
        }

        let follow_up = build_request(prep, &msgs);
        round_no += 1;
        sink.round_start(round_no);
        reply = one_round(prep, &follow_up, mode, sink).await?;
        turn_usage.push(reply.usage);
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
        eprintln!("[chat] 工具往返用尽仍无正文，去掉工具再问一次，强制它作答");
        discard_if_visible(&reply, sink, round_no);
        msgs.push(Message::assistant_tool_calls(reply.tool_calls.clone()));
        msgs.push(Message::user(crate::tools::FINAL_ANSWER_PROMPT.to_string()));
        // **不带 tools**：模型此刻已经证明会一直要工具，再给一次只是再要一轮。
        let final_request = crate::llm::build_request(&prep.llm_config, msgs.clone());
        round_no += 1;
        sink.round_start(round_no);
        match one_round(prep, &final_request, mode, sink).await {
            Ok(last) => {
                // 收尾这一次也真花了 token：不管它最后有没有被采纳，都记上。
                turn_usage.push(last.usage);
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
                } else {
                    discard_if_visible(&last, sink, round_no);
                }
            }
            Err(e) => {
                // 失败不改变结论：下面照旧报 `tool_loop_exhausted`。流式那边
                // 由调用方把这个 Err 变成 `error` 事件，不会无声断连接。
                eprintln!("[chat] 收尾调用失败，保留原来的 tool_loop_exhausted 结论：{e}");
            }
        }
    }
    let turn_ms = started.elapsed().as_millis() as i64;
    // 存档、汇总列、响应 JSON 三处用**同一份**总量。
    let usage = turn_usage.finish();

    Ok(TurnOutcome {
        reply,
        usage,
        tool_trace,
        rounds,
        forced_final_answer,
        turn_ms,
    })
}

/// 用户消息那一段。SSE 的 `user_message` 事件与 `done` 里的 `user_message`
/// 都调它 —— 前端在流式那条路上先拿它把气泡画出来，最后又拿 `done` 里的
/// 覆盖一次，两份要是各拼各的，字段早晚会对不上。
pub(crate) fn user_message_json(prep: &TurnPrep) -> Value {
    json!({
        "id": to_hex_upper(&prep.user_id),
        "seq": prep.seq_user,
        "content": prep.content,
        "created_at": prep.user_created_at,
    })
}

/// 校验结果 → 落助手消息 → 更新会话 → 拼响应体。
///
/// 一次性路由把它包成 JSON，SSE 路由**原样**放进 `done` 事件 —— 两处看到的
/// 内容必然是同一份，不会出现「流式那条少几个字段」。
pub(crate) async fn finish_turn(prep: &TurnPrep, outcome: TurnOutcome) -> Result<Value, ApiError> {
    let TurnOutcome {
        reply,
        usage,
        tool_trace,
        rounds,
        forced_final_answer,
        turn_ms,
    } = outcome;

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
            prep.llm_config.max_tokens
        )));
    } else {
        (String::new(), reply.reasoning.clone())
    };

    let seq_assistant = prep.seq_user + 1;
    let assistant_id = new_id()?;
    let assistant_created_at = append_message(
        &prep.db,
        prep.uid,
        prep.sid,
        &assistant_id,
        MessageBody {
            seq: seq_assistant,
            role: "assistant",
            status: "complete",
            content: &text,
            reasoning: Some(&reasoning),
            usage,
            turn_ms: Some(turn_ms),
        },
    )
    .await?;

    touch_session(
        &prep.db,
        prep.uid,
        prep.sid,
        seq_assistant + 1,
        usage.input,
        usage.output,
    )
    .await?;

    Ok(json!({
        "session_id": prep.session_id,
        "user_message": user_message_json(prep),
        "reply": text,
        "reasoning": reasoning,
        "message": {
            "id": to_hex_upper(&assistant_id),
            "seq": seq_assistant,
            "content": text,
            "created_at": assistant_created_at,
        },
        "finish_reason": reply.finish_reason.map(|f| format!("{f:?}")),
        // 缓存两项是 nullable：上游没报就是 null，前端据此决定「命中率」这一格
        // 到底显示数字还是干脆不显示。别把它们 default 成 0。
        // 这里给的是**整轮总量**（含工具往返的每一轮），与存档里那一行同源。
        "usage": {
            "input": usage.input,
            "output": usage.output,
            "cache_read": usage.cache_read,
            "cache_write": usage.cache_write,
        },
        "turn_ms": turn_ms,
        "persona_applied": prep.persona.instructions.is_some(),
        "expert_notice": prep.persona.notice,
        // 工具执行轨迹。没有工具时是空数组 —— 前端按「长度 0」判定不显示，
        // 不需要另设一个布尔开关（两个字段可能不同步的那种设计最难维护）。
        "tool_calls": tool_trace,
        "tool_rounds": rounds,
        // 这一段正文是**工具用尽之后、把工具摘掉逼出来的**，没有工具支撑。
        // 实测 4B 在这种时候会开始编（ISSUE-043），所以必须让界面知道：
        // 用户看到的必须是一条警告，而不是一段看起来和平时一样可信的回答。
        "final_answer_forced": forced_final_answer,
    }))
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
    // 两段读都在 repo 层（queue Q006c）：会话绑的谁在 `chat_repo`，
    // 那个人格是什么在 `experts_repo`（口径与 ExpertRegistry 的去重一致）。
    let expert_id = crate::chat_repo::session_expert_id(db, uid, sid).map_err(storage)?;

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

    let row = crate::experts_repo::persona_instructions_model(db, uid, expert_id.clone())
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
    crate::chat_repo::history_rows(db, uid, sid, HISTORY_LIMIT).map_err(storage)
}

async fn next_seq(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
) -> Result<i64, ApiError> {
    crate::chat_repo::next_seq(db, uid, sid).map_err(storage)
}

/// 一条要写进 `messages` 的消息体。
///
/// 把这七列拼成一个结构体，是因为 `append_message` 原本有 11 个参数
/// （`clippy::too_many_arguments`）：前四个（db/uid/sid/mid）是不变的定位参数，
/// 后面这七个才随每一行而变。分组而不是整条 `#[allow]`，改的人一眼能看出
/// 「变化的东西」与「定位的东西」是两回事。
struct MessageBody<'a> {
    seq: i64,
    role: &'a str,
    status: &'a str,
    content: &'a str,
    reasoning: Option<&'a str>,
    usage: TokenUsage,
    turn_ms: Option<i64>,
}

async fn append_message(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
    mid: &[u8; 16],
    body: MessageBody<'_>,
) -> Result<i64, ApiError> {
    let MessageBody {
        seq,
        role,
        status,
        content,
        reasoning,
        usage,
        turn_ms,
    } = body;
    let created_at = now_ms();
    crate::chat_repo::insert_message(
        db,
        uid,
        sid,
        crate::chat_repo::NewMessage {
            id: *mid,
            seq,
            role: role.to_string(),
            status: status.to_string(),
            content: content.to_string(),
            reasoning: reasoning.map(String::from),
            input_tokens: i64::from(usage.input.unwrap_or(0)),
            output_tokens: i64::from(usage.output.unwrap_or(0)),
            // None = 模型端没上报，不是 0。见 migration 0008 的说明。
            cache_read_tokens: usage.cache_read.map(i64::from),
            cache_write_tokens: usage.cache_write.map(i64::from),
            turn_ms,
            created_at,
        },
    )
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
    crate::chat_repo::touch_session(db, uid, sid, next_seq, input, output, now_ms())
        .map_err(storage)
}

async fn ensure_session(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    sid: [u8; 16],
) -> Result<(), ApiError> {
    // 存在性判定的 SQL 早就在 `chat_repo::session_exists` 里（Q006 那批收的口径），
    // 这里原先又内联了一份 `count(*)` —— 同一件事两种写法，改一处必漂。
    if crate::chat_repo::session_exists(db, uid, sid).map_err(storage)? {
        return Ok(());
    }
    Err(ApiError::entity_not_found(
        "会话不存在，或不属于当前用户。下一步：先 POST /api/sessions 建一个，再发消息。"
            .to_string(),
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
            body: r#"{"error":{"type":"exceed_context_size_error","n_prompt_tokens":8525}}"#.into(),
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

    // -----------------------------------------------------------------------
    // 一轮的 token 用量：工具往返的**每一轮**都要算进去
    //
    // 原先只存 `reply.usage`（最后一次调用），于是「先查工具、再回答」这种
    // 最常见的形状会把入参少报一大半。下面这几条把这个口径钉死。
    // -----------------------------------------------------------------------

    fn usage(input: Option<u32>, output: Option<u32>, cache: Option<u32>) -> TokenUsage {
        TokenUsage::new(input, output).with_cache(cache, None)
    }

    #[test]
    fn a_single_round_is_stored_exactly_as_the_provider_reported_it() {
        // 不带工具的那一轮（绝大多数请求）必须**逐字不变**。
        for u in [
            TokenUsage::default(),
            usage(Some(1000), Some(20), Some(900)),
            // 上游报的数即使离谱也不在这一层改写：那是上游的口径。
            usage(Some(10), Some(1), Some(9999)),
            usage(None, Some(7), None),
        ] {
            let mut acc = TurnUsage::default();
            acc.push(u);
            assert_eq!(acc.finish(), u, "单轮不能被求和逻辑改写：{u:?}");
        }
    }

    #[test]
    fn a_turn_with_tool_rounds_reports_every_round_not_only_the_last_one() {
        let mut acc = TurnUsage::default();
        acc.push(usage(Some(1000), Some(50), Some(400)));
        // 第二轮才是给出正文的那一轮，入参更大（上下文里多了工具结果）。
        acc.push(usage(Some(1500), Some(80), Some(1200)));
        let t = acc.finish();
        assert_eq!(t.input, Some(2500), "两轮都要算，不能只报最后一轮");
        assert_eq!(t.output, Some(130));
        assert_eq!(t.cache_read, Some(1600));
    }

    #[test]
    fn a_turn_where_nobody_reported_usage_reports_nothing_rather_than_zero() {
        // 全部 None 时求和必须是 None：报 0 会让界面显示「这次聊天一点没花 token」。
        let mut acc = TurnUsage::default();
        acc.push(TokenUsage::default());
        acc.push(TokenUsage::default());
        let t = acc.finish();
        assert_eq!(t.input, None);
        assert_eq!(t.output, None);
        assert_eq!(t.cache_read, None);
    }

    #[test]
    fn a_turn_sums_the_reported_rounds_and_leaves_the_unreported_ones_out() {
        // 与 `session_metrics::sum_reported` 同一条规矩：token 可加，
        // 有一条真值即可；没报的那几轮不参与，也**不**因此把整体变成 None。
        let mut acc = TurnUsage::default();
        acc.push(usage(None, None, None));
        acc.push(usage(Some(700), Some(30), Some(100)));
        acc.push(usage(None, Some(5), None));
        let t = acc.finish();
        assert_eq!(t.input, Some(700));
        assert_eq!(t.output, Some(35));
        assert_eq!(t.cache_read, Some(100));
    }

    #[test]
    fn summed_cache_tokens_never_escape_the_summed_input() {
        // 半真值组合：某几轮报了入参 0，另一轮报了缓存读。逐项相加后
        // cache_read > input，命中率会算出 >100%。夹到 input 上。
        let mut acc = TurnUsage::default();
        acc.push(usage(Some(0), Some(0), Some(500)));
        acc.push(usage(Some(100), Some(0), None));
        let t = acc.finish();
        assert_eq!(t.input, Some(100));
        assert_eq!(t.cache_read, Some(100), "缓存读是入参的子集，不能越界");
    }

    #[test]
    fn the_sum_saturates_instead_of_wrapping_around() {
        let mut acc = TurnUsage::default();
        acc.push(usage(Some(u32::MAX), Some(0), None));
        acc.push(usage(Some(u32::MAX), Some(0), None));
        assert_eq!(
            acc.finish().input,
            Some(u32::MAX),
            "回绕成 0 比不显示更糟：用户会以为这次几乎没花 token"
        );
    }
}
