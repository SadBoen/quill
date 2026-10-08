//! provider 的**持久化与装配**（HTTP 层）。内核侧模型与探测见
//! `quill_core::providers`（queue Q015 把协议知识搬了过去），这里只剩：
//!
//! - `llm_providers` 表的读写（9 条 SQL 收在本文件，L4 不许在别处写 SQL）；
//! - 从 `LlmConfig` 播下第一行默认 provider；
//! - 把行喂给内核的 `ProviderCache`。
//!
//! 下面这一组 re-export 是为了让既有调用点（`api_admin` / `api_providers` 等）
//! 的 `crate::llm_providers::LlmProvider` 这类路径**一个字都不用改**。

use crate::db::{now_ms, storage_error};
use crate::error::ApiError;
use crate::llm::LlmConfig;

pub use quill_core::providers::{
    cache_of, display_name_of, parse_models_payload, probe_models, LlmProvider, ModelCard,
    ProviderCache, ProviderKind, CUSTOM_PRESET_ID, PROBE_TIMEOUT,
};

// ---------------------------------------------------------------------------
// 持久化
// ---------------------------------------------------------------------------

const COLUMNS: &str = "id,name,preset_id,kind,protocol,base_url,api_key,model,\
     max_context_tokens,compaction_threshold_tokens,max_output_tokens,\
     enabled,is_default,created_at,updated_at";

fn storage(e: quill_agent::AgentError) -> ApiError {
    ApiError::storage_unavailable(e.to_string())
}

fn row_to_provider(row: &sqlx::sqlite::SqliteRow) -> Result<LlmProvider, quill_agent::AgentError> {
    use sqlx::Row;
    let s = |name: &str| -> Result<String, quill_agent::AgentError> {
        row.try_get::<String, _>(name)
            .map_err(|e| storage_error(&format!("读 llm_providers.{name}"), e))
    };
    let n = |name: &str| -> Result<i64, quill_agent::AgentError> {
        row.try_get::<i64, _>(name)
            .map_err(|e| storage_error(&format!("读 llm_providers.{name}"), e))
    };
    let kind_raw = s("kind")?;
    let kind = ProviderKind::parse(&kind_raw).ok_or_else(|| {
        crate::db::invariant_broken(format!(
            "llm_providers.kind={kind_raw:?} 不在枚举里：数据脏了，下一步：删除该行后由 admin 重建。"
        ))
    })?;
    let protocol_raw = s("protocol")?;
    let protocol =
        LlmProvider::parse_protocol(&protocol_raw).map_err(crate::db::invariant_broken)?;

    let u32_of = |v: i64, name: &str| -> Result<u32, quill_agent::AgentError> {
        if v < 0 || v > i64::from(u32::MAX) {
            return Err(crate::db::invariant_broken(format!(
                "llm_providers.{name}={v} 超出 u32 范围"
            )));
        }
        Ok(v as u32)
    };

    Ok(LlmProvider {
        id: s("id")?,
        name: s("name")?,
        preset_id: s("preset_id")?,
        kind,
        protocol,
        base_url: s("base_url")?,
        api_key: s("api_key")?,
        model: s("model")?,
        max_context_tokens: u32_of(n("max_context_tokens")?, "max_context_tokens")?,
        compaction_threshold_tokens: u32_of(
            n("compaction_threshold_tokens")?,
            "compaction_threshold_tokens",
        )?,
        max_output_tokens: u32_of(n("max_output_tokens")?, "max_output_tokens")?,
        enabled: n("enabled")? != 0,
        is_default: n("is_default")? != 0,
        created_at: n("created_at")?,
        updated_at: n("updated_at")?,
    })
}

pub fn list(state: &crate::state::AppState) -> Result<Vec<LlmProvider>, ApiError> {
    let db = state.db()?;
    let rows = db
        .call(|pool, _rt| {
            Box::pin(async move {
                let r: Result<Vec<LlmProvider>, quill_agent::AgentError> = async {
                    let sql = format!("SELECT {COLUMNS} FROM llm_providers ORDER BY is_default DESC, created_at ASC, id ASC");
                    let out = sqlx::query(&sql)
                        .fetch_all(&pool)
                        .await
                        .map_err(|e| storage_error("列模型供应商", e))?;
                    out.iter().map(row_to_provider).collect()
                }
                .await;
                r
            })
        })
        .map_err(storage)?;
    Ok(rows)
}

pub fn get(state: &crate::state::AppState, id: &str) -> Result<Option<LlmProvider>, ApiError> {
    let db = state.db()?;
    let id = id.trim().to_string();
    let row = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let r: Result<Option<LlmProvider>, quill_agent::AgentError> = async {
                    let sql = format!("SELECT {COLUMNS} FROM llm_providers WHERE id = ?");
                    let out = sqlx::query(&sql)
                        .bind(&id)
                        .fetch_optional(&pool)
                        .await
                        .map_err(|e| storage_error("读模型供应商", e))?;
                    match out {
                        Some(row) => Ok(Some(row_to_provider(&row)?)),
                        None => Ok(None),
                    }
                }
                .await;
                r
            })
        })
        .map_err(storage)?;
    Ok(row)
}

/// 取默认 provider。表为空时返回 None（不是错误 —— 首启还没种行是合法的）。
pub fn default_provider(state: &crate::state::AppState) -> Result<Option<LlmProvider>, ApiError> {
    let db = state.db()?;
    let row = db
        .call(|pool, _rt| {
            Box::pin(async move {
                let r: Result<Option<LlmProvider>, quill_agent::AgentError> = async {
                    let sql = format!(
                        "SELECT {COLUMNS} FROM llm_providers ORDER BY is_default DESC, created_at ASC LIMIT 1"
                    );
                    let out = sqlx::query(&sql)
                        .fetch_optional(&pool)
                        .await
                        .map_err(|e| storage_error("读默认模型供应商", e))?;
                    match out {
                        Some(row) => Ok(Some(row_to_provider(&row)?)),
                        None => Ok(None),
                    }
                }
                .await;
                r
            })
        })
        .map_err(storage)?;
    Ok(row)
}

pub fn count(state: &crate::state::AppState) -> Result<i64, ApiError> {
    let db = state.db()?;
    db.call(|pool, _rt| {
        Box::pin(async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM llm_providers")
                .fetch_one(&pool)
                .await
                .map_err(|e| storage_error("数模型供应商", e))
        })
    })
    .map_err(storage)
}

pub fn insert(state: &crate::state::AppState, p: &LlmProvider) -> Result<(), ApiError> {
    let db = state.db()?;
    let p = p.clone();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r: Result<(), quill_agent::AgentError> = async {
                sqlx::query(
                    "INSERT INTO llm_providers(id,name,preset_id,kind,protocol,base_url,api_key,\
                     model,max_context_tokens,compaction_threshold_tokens,max_output_tokens,\
                     enabled,is_default,created_at,updated_at)\
                     VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                )
                .bind(&p.id)
                .bind(&p.name)
                .bind(&p.preset_id)
                .bind(p.kind.as_str())
                .bind(p.protocol_str())
                .bind(&p.base_url)
                .bind(&p.api_key)
                .bind(&p.model)
                .bind(i64::from(p.max_context_tokens))
                .bind(i64::from(p.compaction_threshold_tokens))
                .bind(i64::from(p.max_output_tokens))
                .bind(i64::from(p.enabled))
                .bind(i64::from(p.is_default))
                .bind(p.created_at)
                .bind(p.updated_at)
                .execute(&pool)
                .await
                .map_err(|e| storage_error("写模型供应商", e))?;
                Ok(())
            }
            .await;
            r
        })
    })
    .map_err(storage)
}

pub fn update(state: &crate::state::AppState, p: &LlmProvider) -> Result<(), ApiError> {
    let db = state.db()?;
    let p = p.clone();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r: Result<(), quill_agent::AgentError> = async {
                sqlx::query(
                    "UPDATE llm_providers SET name=?,preset_id=?,kind=?,protocol=?,base_url=?,\
                     api_key=?,model=?,max_context_tokens=?,compaction_threshold_tokens=?,\
                     max_output_tokens=?,enabled=?,updated_at=? WHERE id=?",
                )
                .bind(&p.name)
                .bind(&p.preset_id)
                .bind(p.kind.as_str())
                .bind(p.protocol_str())
                .bind(&p.base_url)
                .bind(&p.api_key)
                .bind(&p.model)
                .bind(i64::from(p.max_context_tokens))
                .bind(i64::from(p.compaction_threshold_tokens))
                .bind(i64::from(p.max_output_tokens))
                .bind(i64::from(p.enabled))
                .bind(p.updated_at)
                .bind(&p.id)
                .execute(&pool)
                .await
                .map_err(|e| storage_error("更新模型供应商", e))?;
                Ok(())
            }
            .await;
            r
        })
    })
    .map_err(storage)
}

pub fn delete(state: &crate::state::AppState, id: &str) -> Result<(), ApiError> {
    let db = state.db()?;
    let id = id.trim().to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r: Result<(), quill_agent::AgentError> = async {
                sqlx::query("DELETE FROM llm_providers WHERE id = ?")
                    .bind(&id)
                    .execute(&pool)
                    .await
                    .map_err(|e| storage_error("删模型供应商", e))?;
                Ok(())
            }
            .await;
            r
        })
    })
    .map_err(storage)
}

/// 把某一行挪成唯一默认项：先清掉其他默认，再置位。
pub fn set_default(state: &crate::state::AppState, id: &str) -> Result<(), ApiError> {
    let db = state.db()?;
    let id = id.trim().to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let r: Result<(), quill_agent::AgentError> = async {
                sqlx::query(
                    "UPDATE llm_providers SET is_default = 0 WHERE is_default = 1 AND id <> ?",
                )
                .bind(&id)
                .execute(&pool)
                .await
                .map_err(|e| storage_error("清默认标记", e))?;
                sqlx::query("UPDATE llm_providers SET is_default = 1, updated_at = ? WHERE id = ?")
                    .bind(now_ms())
                    .bind(&id)
                    .execute(&pool)
                    .await
                    .map_err(|e| storage_error("设默认供应商", e))?;
                Ok(())
            }
            .await;
            r
        })
    })
    .map_err(storage)
}

/// 首启播种：写一条 `kind=custom` 的默认 provider。已存在行时什么都不做。
pub fn seed_default_from_llm_config(
    state: &crate::state::AppState,
    cfg: &LlmConfig,
    name: &str,
) -> Result<Option<LlmProvider>, ApiError> {
    let now = now_ms();
    let p = LlmProvider {
        id: LlmProvider::new_id().map_err(ApiError::internal)?,
        name: name.to_string(),
        preset_id: CUSTOM_PRESET_ID.to_string(),
        kind: ProviderKind::Custom,
        protocol: cfg.protocol,
        base_url: cfg.base_url.clone(),
        api_key: cfg.api_key.clone().unwrap_or_default(),
        model: cfg.model.clone(),
        max_context_tokens: cfg.max_context_tokens,
        compaction_threshold_tokens: cfg.compaction_threshold_tokens,
        max_output_tokens: cfg.max_tokens,
        enabled: true,
        is_default: true,
        created_at: now,
        updated_at: now,
    };
    insert(state, &p)?;
    Ok(Some(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    // `Protocol` 只在测试里用到（构造测试行），所以 import 放在测试模块里 ——
    // 放文件顶部会让 lib 构建多一个未使用 import（clippy -D warnings 会红）。
    use crate::llm::Protocol;
    use serde_json::json;

    #[test]
    fn display_name_strips_the_directory_and_the_weight_suffix() {
        assert_eq!(
            display_name_of("D:\\00_ProgramFiles\\llama\\models\\Qwen3.5-4B-Q4_K_M.gguf"),
            "Qwen3.5-4B-Q4_K_M"
        );
        assert_eq!(
            display_name_of("/models/qwen3-32b.safetensors"),
            "qwen3-32b"
        );
        assert_eq!(display_name_of("gpt-4o"), "gpt-4o");
        assert_eq!(display_name_of("org/model.bin"), "model");
    }

    #[test]
    /// 这条测试的**断言在 2026-10-06 被推翻过一次**，所以名字与注释都留了痕迹。
    ///
    /// 原来它断言 `n_ctx_train`（训练窗口）**优先于** `n_ctx`（实例窗口），
    /// 还写着「n_ctx 不是训练上下文，不许拿它顶包」。当时的理由是：
    /// 「训练上下文才是模型真正的能力上限，实例开小是部署的事」。
    /// 那个理由在**能力**问题上成立，在**「这条请求最多能带多少 token」**问题上
    /// 完全不成立 —— 而 `ModelCard.context_window` 正是后一个问题，因为
    /// `validate()` 让用户照着它填 `max_context_tokens`，界面也照着它显示。
    ///
    /// 本机实测坐实了后果：llama-b10068 + Qwen3.5-4B，`-c 8192` 起的，
    /// 同一个响应里 `n_ctx = 8192` / `n_ctx_train = 262144`。
    /// 旧行为会报出 262144 —— **偏大 32 倍**，而服务器只吞 8192。
    /// 见 ISSUE-022。
    fn context_window_is_the_serving_window_never_the_trained_one() {
        let payload = json!({
            "object": "list",
            "data": [
                {
                    "id": "m.gguf",
                    "owned_by": "llamacpp",
                    "meta": { "n_ctx": 8192, "n_ctx_train": 262144 }
                },
                { "id": "no-meta", "meta": { "n_ctx": 8192 } },
                { "id": "vllm", "meta": { "max_model_len": 131072 } },
                { "id": "train-only", "meta": { "n_ctx_train": 262144 } },
            ]
        });
        let got = parse_models_payload(&payload).expect("合法响应必须解析");
        assert_eq!(
            got[0].context_window,
            Some(8192),
            "实例窗口才是这次部署实际能吞下的量"
        );
        assert_ne!(
            got[0].context_window,
            Some(262144),
            "拿 n_ctx_train 会把上限报大 32 倍"
        );
        assert_eq!(
            got[1].context_window,
            Some(8192),
            "meta 里只有 n_ctx 时就用它"
        );
        assert_eq!(got[2].context_window, Some(131072));
        assert_eq!(
            got[3].context_window, None,
            "只报了训练窗口就该说「不知道」，不能拿它顶包"
        );
        assert_eq!(got[0].owned_by.as_deref(), Some("llamacpp"));
        assert_eq!(got[1].owned_by, None);
    }

    #[test]
    fn modality_is_text_only_with_upstream_evidence() {
        let payload = json!({
            "object": "list",
            "models": [
                { "name": "a.gguf", "type": "model", "capabilities": ["completion"] }
            ],
            "data": [
                { "id": "a.gguf", "meta": {} },
                { "id": "b.gguf", "capabilities": ["text", "vision"] },
                { "id": "c.gguf", "type": "embed" },
                { "id": "d.gguf" },
            ]
        });
        let got = parse_models_payload(&payload).expect("合法响应必须解析");
        assert_eq!(
            got[0].modality, "text",
            "同一响应里 models[].capabilities 是证据"
        );
        assert_eq!(got[1].modality, "text");
        assert_eq!(got[2].modality, "unknown", "embedding 不能猜成 text");
        assert_eq!(got[3].modality, "unknown", "没有任何字段就不许瞎猜");
    }

    #[test]
    fn a_non_openai_payload_is_an_error_not_an_empty_list() {
        let err = parse_models_payload(&json!({ "object": "list" })).expect_err("缺 data 必须报错");
        assert!(err.contains("data"), "错误要说明缺什么：{err}");
        let err = parse_models_payload(&json!({ "data": [] })).expect_err("空列表必须报错");
        assert!(err.contains("空"), "空列表不得被当成探测成功：{err}");
    }

    #[test]
    fn openrouter_rows_are_projected_onto_openai() {
        assert_eq!(
            LlmProvider::parse_protocol("openrouter").expect("历史值必须能读"),
            Protocol::Openai
        );
        assert!(LlmProvider::parse_protocol("bogus").is_err());
    }

    #[test]
    fn ids_are_32_uppercase_hex() {
        let id = LlmProvider::new_id().expect("随机源可用");
        assert_eq!(id.len(), 32, "必须是 32 位 hex：{id}");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_lowercase()),
            "必须全是大写 hex（SQLite hex() 读出来是大写）：{id}"
        );
    }

    #[test]
    fn json_never_carries_the_api_key() {
        let p = LlmProvider {
            id: "A".repeat(32),
            name: "本地".into(),
            preset_id: "custom".into(),
            kind: ProviderKind::Custom,
            protocol: Protocol::Openai,
            base_url: "http://x/v1".into(),
            api_key: "sk-super-secret".into(),
            model: "m".into(),
            max_context_tokens: 32768,
            compaction_threshold_tokens: 8000,
            max_output_tokens: 4096,
            enabled: true,
            is_default: true,
            created_at: 1,
            updated_at: 2,
        };
        let v = p.to_json();
        assert_eq!(v["has_api_key"], true);
        assert!(
            v.get("api_key").is_none(),
            "响应里不允许出现 api_key 字段：{v}"
        );
        assert!(!v.to_string().contains("sk-super-secret"));
    }

    #[test]
    fn validate_reports_the_offending_field_in_chinese() {
        let mut p = LlmProvider {
            id: "A".repeat(32),
            name: "本地".into(),
            preset_id: "custom".into(),
            kind: ProviderKind::Custom,
            protocol: Protocol::Openai,
            base_url: "http://x/v1".into(),
            api_key: String::new(),
            model: "m".into(),
            max_context_tokens: 8000,
            compaction_threshold_tokens: 9000,
            max_output_tokens: 4096,
            enabled: true,
            is_default: false,
            created_at: 0,
            updated_at: 0,
        };
        let err = p.validate().expect_err("阈值越界必须被拒");
        assert!(err.contains("compaction_threshold_tokens"), "{err}");
        assert!(err.contains("下一步"), "{err}");

        p.compaction_threshold_tokens = 8000;
        p.max_output_tokens = 0;
        assert!(p.validate().is_err(), "output 必须 > 0");
    }
}
