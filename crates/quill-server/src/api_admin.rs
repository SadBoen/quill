//! 实例级 LLM provider 配置的读 / 写接口。
//!
//! 设计要点：
//! - 只接受 admin 令牌（用 `RequireAdmin` extractor 兜底，非 admin 直接 403）。
//! - 校验分两层：先在 `LlmConfig` / `AdminConfig` 的 `validate` 里跑语义检查；
//!   再让 SQL 层跑 CHECK（protocol enum / 字段上下界），让数据库在
//!   「绕过 API 直写 INSERT」这条路径上也仍然拒绝坏数据。
//! - PUT 成功后立刻把新的 `SharedProvider` 装回 `state.llm`，让后续聊天请求
//!   直接走到新配置上；旧的 `SharedProvider` 在已发起的请求跑完后自动释放。

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::RequireAdmin;
use crate::db::now_ms;
use crate::error::ApiError;
use crate::llm::{AdminConfig, LlmConfig, Protocol};
use crate::llm_providers::{self, LlmProvider, ProviderKind};
use crate::state::AppState;

/// `GET /api/admin/config`：读**默认 provider**。
/// 解析失败时一律按 `provider_unavailable` 返回 503 + 中文 next_step；
/// 调用方要做的是「看不到」，不该被错误码绕晕。
///
/// **响应里没有 api_key**：只给 `has_api_key`。密钥只存在于数据库与进程内，
/// 任何 admin 路由都不回传明文（`/api/admin/providers*` 同口径）。
/// `PUT /api/admin/config` 的**请求体**仍接受 api_key：缺省 / 空串 = 沿用旧值。
pub async fn get(
    State(state): State<AppState>,
    _admin: RequireAdmin,
) -> Result<Json<Value>, ApiError> {
    let cfg = load_admin_config(&state)?;
    Ok(Json(admin_config_to_json(&cfg)))
}

/// `PUT /api/admin/config`：更新**默认 provider**（不存在就建一条
/// `kind=custom` 的），保持既有热重载语义。
/// 校验失败 → 400；DB 错误 → 503；成功后立即热替换运行时 provider。
pub async fn put(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let incoming = parse_admin_config(&body)?;
    incoming.validate().map_err(ApiError::bad_request)?;

    let now = now_ms();
    // 0002 里历史存在的 openrouter 落到 llm_providers 时映射成 openai。
    let protocol = LlmProvider::parse_protocol(incoming.protocol.as_str())
        .map_err(ApiError::bad_request)?;

    let p = match llm_providers::default_provider(&state)? {
        Some(mut cur) => {
            cur.protocol = protocol;
            cur.base_url = incoming.base_url.trim().to_string();
            if !incoming.api_key.trim().is_empty() {
                cur.api_key = incoming.api_key.trim().to_string();
            }
            cur.model = incoming.model.trim().to_string();
            cur.max_context_tokens = incoming.max_context_tokens;
            cur.compaction_threshold_tokens = incoming.compaction_threshold_tokens;
            cur.max_output_tokens = incoming.max_output_tokens;
            cur.enabled = true;
            cur.updated_at = now;
            cur
        }
        None => LlmProvider {
            id: LlmProvider::new_id()?,
            name: "默认模型服务".to_string(),
            preset_id: llm_providers::CUSTOM_PRESET_ID.to_string(),
            kind: ProviderKind::Custom,
            protocol,
            base_url: incoming.base_url.trim().to_string(),
            api_key: incoming.api_key.trim().to_string(),
            model: incoming.model.trim().to_string(),
            max_context_tokens: incoming.max_context_tokens,
            compaction_threshold_tokens: incoming.compaction_threshold_tokens,
            max_output_tokens: incoming.max_output_tokens,
            enabled: true,
            is_default: true,
            created_at: now,
            updated_at: now,
        },
    };
    p.validate().map_err(ApiError::bad_request)?;

    // 先确认新默认项能构造出运行时 provider，再落库：失败的写操作不该
    // 动到正在服务的对话通道。
    let llm_cfg: LlmConfig = p.to_llm_config();
    let new_provider = match crate::llm::build(&llm_cfg) {
        Ok(p) => Some(p),
        Err(detail) => {
            return Err(ApiError::bad_request(format!(
                "校验通过了，但构造 provider 失败：{detail}。\
                 下一步：检查 base_url / api_key / 模型名是否正确；表里的旧值未改动。"
            )));
        }
    };

    if llm_providers::get(&state, &p.id)?.is_some() {
        llm_providers::update(&state, &p)?;
    } else {
        llm_providers::insert(&state, &p)?;
    }
    state.reload_providers()?;
    state.replace_llm(new_provider, llm_cfg);

    let mut persisted = incoming;
    persisted.protocol = p.protocol;
    persisted.api_key = p.api_key;
    persisted.updated_at = now;
    Ok(Json(admin_config_to_json(&persisted)))
}

fn admin_config_to_json(cfg: &AdminConfig) -> Value {
    json!({
        "protocol": cfg.protocol.as_str(),
        "base_url": cfg.base_url,
        "has_api_key": !cfg.api_key.trim().is_empty(),
        "model": cfg.model,
        "max_context_tokens": cfg.max_context_tokens,
        "compaction_threshold_tokens": cfg.compaction_threshold_tokens,
        "max_output_tokens": cfg.max_output_tokens,
        "updated_at": cfg.updated_at,
    })
}

fn parse_admin_config(body: &Value) -> Result<AdminConfig, ApiError> {
    fn pick<'a>(body: &'a Value, key: &str) -> Result<&'a str, ApiError> {
        body.get(key).and_then(Value::as_str).ok_or_else(|| {
            ApiError::bad_request(format!(
                "字段 {key:?} 必须是字符串。下一步：检查请求体里是否漏了字段，或用了数字 / null。"
            ))
        })
    }

    let protocol_raw = pick(body, "protocol")?;
    let protocol = Protocol::parse(protocol_raw).ok_or_else(|| {
        ApiError::bad_request(format!(
            "protocol {protocol_raw:?} 不在枚举里（必须是 openai / anthropic / openrouter）。\
             下一步：改成这三个值中的一个再重试。"
        ))
    })?;

    let base_url = pick(body, "base_url")?.trim().to_string();
    if base_url.is_empty() {
        return Err(ApiError::bad_request(String::from(
            "base_url 不能为空。下一步：填完整的接口地址（含 http(s) 与版本前缀 /v1）。",
        )));
    }
    let model = pick(body, "model")?.trim().to_string();
    if model.is_empty() {
        return Err(ApiError::bad_request(String::from(
            "model 不能为空。下一步：填模型服务列出的模型名，例如 gpt-4o、qwen3.5、claude-3-5 等。",
        )));
    }
    let api_key = body
        .get("api_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let max_context_tokens = pick_positive(body, "max_context_tokens")?;
    let compaction_threshold_tokens = pick_positive(body, "compaction_threshold_tokens")?;
    let max_output_tokens = pick_positive(body, "max_output_tokens")?;

    Ok(AdminConfig {
        protocol,
        base_url,
        api_key,
        model,
        max_context_tokens,
        compaction_threshold_tokens,
        max_output_tokens,
        updated_at: 0, // 由 PUT 写入时填 now
    })
}

fn pick_positive(body: &Value, key: &str) -> Result<u32, ApiError> {
    let v = body.get(key).ok_or_else(|| {
        ApiError::bad_request(format!(
            "字段 {key:?} 缺失。下一步：补一个正整数（token 数），参考默认值 32768 / 8000 / 4096。"
        ))
    })?;
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

/// 实例视图的「单条模型配置」= **默认 provider**。
///
/// 真相来源是 `llm_providers`；只有在一行都没有（首启播种失败）时才回退
/// 到 0002 的 `admin_config` 单行表，那张表自 0003 起只读。
pub fn load_admin_config(state: &AppState) -> Result<AdminConfig, ApiError> {
    if let Some(p) = llm_providers::default_provider(state)? {
        return Ok(p.as_admin_config());
    }
    load_legacy_admin_config(state)
}

fn load_legacy_admin_config(state: &AppState) -> Result<AdminConfig, ApiError> {
    let db = state.db()?;
    let out = db
        .call(|pool, _rt| {
            Box::pin(async move {
                let r: Result<Option<AdminConfig>, quill_agent::AgentError> = async {
                    let row = sqlx::query(
                        "SELECT protocol, base_url, api_key, model, \
                         max_context_tokens, compaction_threshold_tokens, \
                         max_output_tokens, updated_at FROM admin_config WHERE id=1",
                    )
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("读 admin_config", e))?;
                    let Some(row) = row else { return Ok(None) };
                    use sqlx::Row;
                    let protocol: String = row
                        .try_get("protocol")
                        .map_err(|e| crate::db::storage_error("读 protocol", e))?;
                    let base_url: String = row
                        .try_get("base_url")
                        .map_err(|e| crate::db::storage_error("读 base_url", e))?;
                    let api_key: String = row
                        .try_get("api_key")
                        .map_err(|e| crate::db::storage_error("读 api_key", e))?;
                    let model: String = row
                        .try_get("model")
                        .map_err(|e| crate::db::storage_error("读 model", e))?;
                    let max_context_tokens: i64 = row
                        .try_get("max_context_tokens")
                        .map_err(|e| crate::db::storage_error("读 max_context_tokens", e))?;
                    let compaction_threshold_tokens: i64 = row
                        .try_get("compaction_threshold_tokens")
                        .map_err(|e| {
                            crate::db::storage_error("读 compaction_threshold_tokens", e)
                        })?;
                    let max_output_tokens: i64 = row
                        .try_get("max_output_tokens")
                        .map_err(|e| crate::db::storage_error("读 max_output_tokens", e))?;
                    let updated_at: i64 = row
                        .try_get("updated_at")
                        .map_err(|e| crate::db::storage_error("读 updated_at", e))?;
                    let proto = Protocol::parse(&protocol).ok_or_else(|| {
                        crate::db::invariant_broken(format!(
                            "admin_config.protocol={protocol:?} 不在枚举里：数据脏了，下一步：执行 PUT /api/admin/config 覆盖。"
                        ))
                    })?;
                    Ok(Some(AdminConfig {
                        protocol: proto,
                        base_url,
                        api_key,
                        model,
                        max_context_tokens: i64_to_u32(max_context_tokens, "max_context_tokens")?,
                        compaction_threshold_tokens: i64_to_u32(
                            compaction_threshold_tokens,
                            "compaction_threshold_tokens",
                        )?,
                        max_output_tokens: i64_to_u32(max_output_tokens, "max_output_tokens")?,
                        updated_at,
                    }))
                }
                .await;
                r
            })
        })
        .map_err(|e| ApiError::storage_unavailable(e.to_string()))?;
    out.ok_or_else(|| {
        ApiError::storage_unavailable(
            "admin_config 表里没有任何行。下一步：先发一次 PUT /api/admin/config 写入即可恢复。",
        )
    })
}

fn i64_to_u32(v: i64, name: &str) -> Result<u32, quill_agent::AgentError> {
    if v < 0 || v > i64::from(u32::MAX) {
        return Err(crate::db::invariant_broken(format!(
            "admin_config.{name}={v} 超出 u32 范围"
        )));
    }
    Ok(v as u32)
}

/// 给 `server::build_state` 启动时使用：把 env-derived 配置回填到表里。
/// 返回值用于日志；写失败不致命 — 启动告警里会体现。
pub fn seed_admin_config_from_env(
    state: &AppState,
    env_cfg: &LlmConfig,
) -> Result<(), ApiError> {
    let db = state.db()?;
    let now = now_ms();
    let cfg = AdminConfig::from_llm_config(env_cfg, now);
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r: Result<(), quill_agent::AgentError> = async {
                // 不存在就 INSERT；存在则不动（已存在的 admin 写入覆盖）。
                let exists: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM admin_config WHERE id=1",
                )
                .fetch_one(&pool)
                .await
                .map_err(|e| crate::db::storage_error("探测 admin_config", e))?;
                if exists == 0 {
                    sqlx::query(
                        "INSERT INTO admin_config(id,protocol,base_url,api_key,model,\
                         max_context_tokens,compaction_threshold_tokens,max_output_tokens,\
                         updated_at) VALUES(1,?,?,?,?,?,?,?,?)",
                    )
                    .bind(cfg.protocol.as_str())
                    .bind(&cfg.base_url)
                    .bind(&cfg.api_key)
                    .bind(&cfg.model)
                    .bind(i64::from(cfg.max_context_tokens))
                    .bind(i64::from(cfg.compaction_threshold_tokens))
                    .bind(i64::from(cfg.max_output_tokens))
                    .bind(cfg.updated_at)
                    .execute(&pool)
                    .await
                    .map_err(|e| crate::db::storage_error("回填 admin_config", e))?;
                }
                Ok(())
            }
            .await;
            r
        })
    })
    .map_err(|e| ApiError::storage_unavailable(e.to_string()))?;
    Ok(())
}

/// 首启播种后的收尾：以**默认 provider** 为真相重建运行时 provider。
/// 表里没有任何 provider 时什么都不做（`server::build_state` 会退回 env）。
pub fn ensure_admin_config_seeded(state: &AppState) {
    if let Ok(cache) = state.reload_providers() {
        state.apply_default_provider(&cache);
    }
}

/// 仅供 cfg(test) 调用 —— 把 body 反序列化逻辑在测试里也能复用一份，
/// 而不是再写一遍 JSON 解析。
#[cfg(test)]
pub fn parse_for_test(body: &Value) -> Result<AdminConfig, ApiError> {
    parse_admin_config(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_admin_config_requires_all_fields_with_chinese_errors() {
        let err = parse_admin_config(&json!({})).expect_err("缺字段必须报错");
        assert!(err.detail().contains("下一步"));
        let body = json!({
            "protocol": "bogus",
            "base_url": "http://x/v1",
            "api_key": "",
            "model": "m",
            "max_context_tokens": 32768,
            "compaction_threshold_tokens": 8000,
            "max_output_tokens": 2048,
        });
        let err = parse_admin_config(&body).expect_err("bogus protocol 必须报错");
        assert!(err.detail().contains("下一步"));
        assert!(err.detail().contains("openai"));
    }
}
