//! 多模型供应商的 admin 接口：CRUD / 切默认 / 真实探测模型池。
//!
//! 写路径的语义差别是刻意的：
//! - `POST` 要**全字段**（缺一个就 400）—— 新建时没有「沿用旧值」可循。
//! - `PUT /{id}` 是**部分更新**（PATCH 风格）：body 里没出现的字段一律沿用
//!   数据库现值。模型池里点 ☆ 只发一个 `{"model": "..."}`，全量替换会把
//!   name / base_url / 三个 token 全清空。
//! - `api_key` 缺省或空串 = 沿用旧密钥；显式 `null` = 清除。
//!
//! 所有响应都不含 api_key 明文，只给 `has_api_key`。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::RequireAdmin;
use crate::body::JsonBody;
use crate::db::now_ms;
use crate::error::ApiError;
use crate::llm_providers::{self, LlmProvider, ModelCard, ProviderKind};
use crate::state::AppState;

const DEFAULT_CTX_TOKENS: u32 = 32_768;
const DEFAULT_COMPACTION_TOKENS: u32 = 8_000;
const DEFAULT_OUTPUT_TOKENS: u32 = 4_096;

fn storage(e: quill_agent::AgentError) -> ApiError {
    ApiError::storage_unavailable(e.to_string())
}

fn not_found(id: &str) -> ApiError {
    ApiError::entity_not_found(format!(
        "模型供应商 {id} 不存在。下一步：先 GET /api/admin/providers 看现有清单，确认 id 是否写错（id 是 32 位大写 hex）。"
    ))
}

/// `GET /api/admin/providers`
pub async fn list(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Value>, ApiError> {
    let rows = llm_providers::list(&state)?;
    Ok(Json(json!({
        "providers": rows.iter().map(LlmProvider::to_json).collect::<Vec<_>>(),
    })))
}

/// `POST /api/admin/providers` —— 全字段必填。
pub async fn create(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    JsonBody(body): JsonBody,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let now = now_ms();
    let existing_default = llm_providers::default_provider(&state)?;

    let name = required_str(&body, "name")?.trim().to_string();
    let base_url = required_str(&body, "base_url")?.trim().to_string();
    let protocol_raw = required_str(&body, "protocol")?;
    let protocol =
        LlmProvider::parse_protocol(protocol_raw.trim()).map_err(ApiError::bad_request)?;

    let kind = parse_kind(&body, "kind")?;
    let preset_id = match body.get("preset_id").and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ if kind == ProviderKind::Custom => llm_providers::CUSTOM_PRESET_ID.to_string(),
        _ => {
            return Err(ApiError::bad_request(
                "kind=preset 时必须给 preset_id（预设卡片上的标识）。下一步：\
                 补上 preset_id，或把 kind 改成 custom。"
                    .to_string(),
            ))
        }
    };

    // model 允许先留空：模型管理页允许「先建端点，再从模型池里挑模型」。
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let api_key = body
        .get("api_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    let is_default = body
        .get("is_default")
        .and_then(Value::as_bool)
        .unwrap_or(existing_default.is_none());

    let p = LlmProvider {
        id: LlmProvider::new_id()?,
        name,
        preset_id,
        kind,
        protocol,
        base_url,
        api_key,
        model,
        max_context_tokens: pick_u32(&body, "max_context_tokens", DEFAULT_CTX_TOKENS)?,
        compaction_threshold_tokens: pick_u32(
            &body,
            "compaction_threshold_tokens",
            DEFAULT_COMPACTION_TOKENS,
        )?,
        max_output_tokens: pick_u32(&body, "max_output_tokens", DEFAULT_OUTPUT_TOKENS)?,
        enabled: body.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        is_default,
        created_at: now,
        updated_at: now,
    };
    p.validate().map_err(ApiError::bad_request)?;
    ensure_default_buildable(&p, is_default)?;
    if is_default {
        clear_default_flag(&state)?;
    }
    llm_providers::insert(&state, &p)?;
    state.reload_providers()?;

    Ok((StatusCode::CREATED, Json(p.to_json())))
}

/// `PUT /api/admin/providers/{id}` —— 部分更新。
///
/// body 里没出现的字段沿用现值；`id` / `kind` / `is_default` / `created_at`
/// 不接受来自 body 的值（`is_default` 只能走 `PUT /{id}/default`）。
pub async fn update(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    let mut p = llm_providers::get(&state, &id)?.ok_or_else(|| not_found(&id))?;

    for reserved in ["id", "is_default", "created_at"] {
        if body.get(reserved).is_some() {
            return Err(ApiError::bad_request(format!(
                "字段 {reserved:?} 不接受来自请求体（id 由服务端分配，is_default 只能走 \
                 PUT /api/admin/providers/{{id}}/default，created_at 是创建时刻）。下一步：把它从请求体里去掉。"
            )));
        }
    }
    if body.get("kind").is_some() {
        return Err(ApiError::bad_request(
            "字段 \"kind\" 不接受修改（preset/custom 是这条记录诞生时就定下的）。\
             下一步：删掉这一行重建，或从请求体里去掉它。"
                .to_string(),
        ));
    }

    if let Some(v) = body.get("name") {
        p.name = want_str(v, "name")?.trim().to_string();
    }
    if let Some(v) = body.get("base_url") {
        p.base_url = want_str(v, "base_url")?.trim().to_string();
    }
    if let Some(v) = body.get("preset_id") {
        p.preset_id = want_str(v, "preset_id")?.trim().to_string();
    }
    if let Some(v) = body.get("protocol") {
        p.protocol = LlmProvider::parse_protocol(want_str(v, "protocol")?.trim())
            .map_err(ApiError::bad_request)?;
    }
    if let Some(v) = body.get("model") {
        p.model = want_str(v, "model")?.trim().to_string();
    }
    if let Some(v) = body.get("enabled") {
        p.enabled = v.as_bool().ok_or_else(|| {
            ApiError::bad_request(
                "字段 \"enabled\" 必须是布尔值。下一步：传 true / false，不要传字符串。"
                    .to_string(),
            )
        })?;
    }
    // 缺省 / 空串 = 沿用旧密钥；显式 null = 清除。
    if let Some(v) = body.get("api_key") {
        p.api_key = match v {
            Value::Null => String::new(),
            Value::String(s) if s.trim().is_empty() => p.api_key,
            Value::String(s) => s.trim().to_string(),
            _ => {
                return Err(ApiError::bad_request(
                    "字段 \"api_key\" 必须是字符串或 null。下一步：\
                     传新密钥、传 \"\" 表示沿用旧密钥，或传 null 表示清除。"
                        .to_string(),
                ))
            }
        };
    }
    for key in [
        "max_context_tokens",
        "compaction_threshold_tokens",
        "max_output_tokens",
    ] {
        if let Some(v) = body.get(key) {
            let n = want_positive_u32(v, key)?;
            match key {
                "max_context_tokens" => p.max_context_tokens = n,
                "compaction_threshold_tokens" => p.compaction_threshold_tokens = n,
                _ => p.max_output_tokens = n,
            }
        }
    }

    p.validate().map_err(ApiError::bad_request)?;
    p.updated_at = now_ms();
    ensure_default_buildable(&p, p.is_default)?;
    llm_providers::update(&state, &p)?;
    state.reload_providers()?;

    Ok(Json(p.to_json()))
}

/// `DELETE /api/admin/providers/{id}` —— 拒绝删「唯一」或「当前默认」。
pub async fn delete(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let p = llm_providers::get(&state, &id)?.ok_or_else(|| not_found(&id))?;
    let total = llm_providers::count(&state)?;

    if total <= 1 {
        return Err(ApiError::conflict(
            format!(
                "不能删掉唯一的一个模型供应商「{}」（当前 {total} 条）：删了实例就没有任何模型端点了。",
                p.name
            ),
            "先 POST /api/admin/providers 加一个新端点，再回来删这条。",
        ));
    }
    if p.is_default {
        return Err(ApiError::conflict(
            format!(
                "不能删掉当前默认供应商「{}」（id={}）：删掉之后实例不知道该用哪个模型。",
                p.name, p.id
            ),
            "先把这个供应商的 id（见 detail）套进 `PUT /api/admin/providers/<id>/default` 切到另一条，再删这条。",
        ));
    }
    llm_providers::delete(&state, &p.id)?;
    state.reload_providers()?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PUT /api/admin/providers/{id}/default` —— 切成唯一默认项并热重载。
pub async fn set_default(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let p = llm_providers::get(&state, &id)?.ok_or_else(|| not_found(&id))?;
    ensure_default_buildable(&p, true)?;
    llm_providers::set_default(&state, &p.id)?;
    state.reload_providers()?;

    let refreshed = llm_providers::get(&state, &p.id)?.ok_or_else(|| not_found(&id))?;
    Ok(Json(refreshed.to_json()))
}

/// `GET /api/admin/providers/{id}/models` —— 对该端点做一次真实探测。
///
/// 探测失败仍返回 200，但 `error` 字段是中文原因、`models` 为空 —— 契约
/// 里这一路是 200，UI 靠 `error` 区分「探测失败」与「确实没有模型」。
pub async fn models(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let p = llm_providers::get(&state, &id)?.ok_or_else(|| not_found(&id))?;
    let probed_at = now_ms();
    let (models, error) = match llm_providers::probe_models(&p.base_url, &p.api_key).await {
        Ok(ms) => (ms, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    Ok(Json(json!({
        "provider_id": p.id,
        "probed_at": probed_at,
        "models": models.iter().map(ModelCard::to_json).collect::<Vec<_>>(),
        "error": error,
    })))
}

/// `GET /api/admin/models` —— 全部**启用**的 provider 的真实探测结果。
///
/// 探测失败的 provider 进 `unavailable` 并带中文原因，绝不用空列表冒充
/// 「这个端点没有模型」。被 `enabled=false` 停用的 provider 两个数组都不进
/// （停用是用户主动的选择，不是探测失败），改由 `disabled` 单列一份，让前端
/// 能把「关了」与「挂了」区分开。
pub async fn pool(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Value>, ApiError> {
    let providers = llm_providers::list(&state)?;
    let default_provider_id = llm_providers::ProviderCache {
        providers: providers.clone(),
    }
    .default_provider_id()
    .map(str::to_string);

    let mut pool = Vec::new();
    let mut unavailable = Vec::new();
    let mut disabled = Vec::new();
    for p in providers.iter() {
        if !p.enabled {
            disabled.push(json!({
                "provider_id": p.id,
                "provider_name": p.name,
                "reason": "已被管理员停用（enabled=false），不参与模型池探测。",
            }));
            continue;
        }
        match llm_providers::probe_models(&p.base_url, &p.api_key).await {
            Ok(models) => {
                for m in &models {
                    pool.push(json!({
                        "provider_id": p.id,
                        "provider_name": p.name,
                        "model": m.to_json(),
                        "starred": m.id == p.model,
                    }));
                }
            }
            Err(e) => unavailable.push(json!({
                "provider_id": p.id,
                "provider_name": p.name,
                "error": e,
            })),
        }
    }

    Ok(Json(json!({
        "generated_at": now_ms(),
        "default_provider_id": default_provider_id,
        "pool": pool,
        "unavailable": unavailable,
        "disabled": disabled,
    })))
}

// ---------------------------------------------------------------------------
// 请求体解析
// ---------------------------------------------------------------------------

fn required_str<'a>(body: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    body.get(key).and_then(Value::as_str).ok_or_else(|| {
        ApiError::bad_request(format!(
            "字段 {key:?} 必填且必须是字符串。下一步：补上该字段，或检查是不是用数字 / null 代替了字符串。"
        ))
    })
}

fn want_str<'a>(v: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    v.as_str().ok_or_else(|| {
        ApiError::bad_request(format!(
            "字段 {key:?} 必须是字符串。下一步：去掉引号外的数字写法，或不要传 null（不传的字段会沿用现值）。"
        ))
    })
}

fn parse_kind(body: &Value, key: &str) -> Result<ProviderKind, ApiError> {
    match body.get(key).and_then(Value::as_str) {
        None => Err(ApiError::bad_request(format!(
            "字段 {key:?} 必填（preset / custom）。下一步：预设卡片用 preset，自定义地址用 custom。"
        ))),
        Some(raw) => ProviderKind::parse(raw.trim()).ok_or_else(|| {
            ApiError::bad_request(format!(
                "kind {raw:?} 不在枚举里（只有 preset / custom）。下一步：改成这两个值之一。"
            ))
        }),
    }
}

/// 缺省用 `default`；给了就必须是非负整数范围内的正数。
fn pick_u32(body: &Value, key: &str, default: u32) -> Result<u32, ApiError> {
    match body.get(key) {
        None => Ok(default),
        Some(v) => want_positive_u32(v, key),
    }
}

fn want_positive_u32(v: &Value, key: &str) -> Result<u32, ApiError> {
    let n = match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
    .ok_or_else(|| {
        ApiError::bad_request(format!(
            "字段 {key:?} 必须是正整数（实际 {v}）。下一步：去掉引号或写成纯数字。"
        ))
    })?;
    if n <= 0 {
        return Err(ApiError::bad_request(format!(
            "字段 {key:?}={n} 不合法（必须 > 0）。下一步：把它改成正整数。"
        )));
    }
    if n > i64::from(u32::MAX) {
        return Err(ApiError::bad_request(format!(
            "字段 {key:?}={n} 超过 u32 上限（{}）。下一步：调小到合理范围。",
            u32::MAX
        )));
    }
    Ok(n as u32)
}

/// 这行即将成为默认项时，必须先能构造出运行时 provider —— 否则一次
/// 失败的管理操作会把正在服务的对话通道打瞎。
fn ensure_default_buildable(p: &LlmProvider, becomes_default: bool) -> Result<(), ApiError> {
    if !becomes_default {
        return Ok(());
    }
    if !p.enabled {
        return Ok(());
    }
    let cfg = p.to_llm_config();
    crate::llm::build(&cfg).map(|_| ()).map_err(|detail| {
        ApiError::bad_request(format!(
            "校验通过了，但把「{}」设为默认时构造 provider 失败：{detail}。\
             下一步：检查 base_url 是否以 http:// 或 https:// 开头；数据库里的旧默认值未改动。",
            p.name
        ))
    })
}

fn clear_default_flag(state: &AppState) -> Result<(), ApiError> {
    let db = state.db()?;
    db.call(|pool, _rt| {
        Box::pin(async move {
            let r: Result<(), quill_agent::AgentError> = async {
                sqlx::query("UPDATE llm_providers SET is_default = 0 WHERE is_default = 1")
                    .execute(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("清默认标记", e))?;
                Ok(())
            }
            .await;
            r
        })
    })
    .map_err(storage)
}
