//! 多模型供应商的数据模型、持久化与**真实**模型探测。
//!
//! 铁律：模型池里的每一个模型都必须来自上游 `/models` 的真实响应。
//! 探测不到就是探测不到 —— 返回中文原因，绝不退回硬编码列表，也绝不
//! 用一个编造的 `context_window` 填坑。派生字段（display_name /
//! modality / context_window）只允许从上游上报的字段里推，推不出来
//! 就给 `null` 或 `"unknown"`。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::db::{now_ms, storage_error};
use crate::error::ApiError;
use crate::llm::{AdminConfig, LlmConfig, Protocol, MIN_COMPACTION_THRESHOLD_TOKENS};

/// 探测上游 `/models` 的超时。探测是同步的 UI 操作，不能让一个挂死的
/// 端点把模型管理页拖住。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// 预设提供商在 `kind='preset'` 时使用的 kind 值之外的兜底：自定义端点。
pub const CUSTOM_PRESET_ID: &str = "custom";

/// `llm_providers.kind` 的两个合法取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Preset,
    Custom,
}

impl ProviderKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "preset" => Some(Self::Preset),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preset => "preset",
            Self::Custom => "custom",
        }
    }
}

/// 一行 provider。`api_key` 只在进程内与数据库里出现，任何 JSON 响应
/// 都只给出 `has_api_key`（见 `to_json`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProvider {
    pub id: String,
    pub name: String,
    pub preset_id: String,
    pub kind: ProviderKind,
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_context_tokens: u32,
    pub compaction_threshold_tokens: u32,
    pub max_output_tokens: u32,
    pub enabled: bool,
    pub is_default: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 进程内的 provider 缓存。读路径（healthz / 默认 provider 翻译）走它，
/// 写路径成功后由 `AppState::reload_providers` 整体刷新。
#[derive(Debug, Clone, Default)]
pub struct ProviderCache {
    pub providers: Vec<LlmProvider>,
}

impl ProviderCache {
    pub fn default_provider(&self) -> Option<&LlmProvider> {
        self.providers
            .iter()
            .find(|p| p.is_default)
            .or_else(|| self.providers.first())
    }

    pub fn default_provider_id(&self) -> Option<&str> {
        self.default_provider().map(|p| p.id.as_str())
    }
}

impl LlmProvider {
    /// 32 位**大写** hex：与项目既有约定一致（SQLite `hex()` 读出来是大写，
    /// 写路径必须逐字节相同，否则前端按 id 去重会全部失配）。
    pub fn new_id() -> Result<String, ApiError> {
        let mut b = [0u8; 16];
        getrandom::fill(&mut b)
            .map_err(|e| ApiError::internal(format!("生成供应商标识失败（系统随机源不可用）：{e}")))?;
        if b == [0u8; 16] {
            b[0] = 1;
        }
        Ok(hex_upper(&b))
    }

    /// 服务端语义校验。数据库侧的 CHECK 承担同样的边界，这里负责「说人话」。
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("name 不能为空。下一步：给它起一个能区分的显示名，例如「本地 llama.cpp」。".into());
        }
        if self.preset_id.trim().is_empty() {
            return Err("preset_id 不能为空。下一步：自定义端点填 custom；\
                        预设供应商填预设卡片上的标识。"
                .into());
        }
        if self.base_url.trim().is_empty() {
            return Err("base_url 不能为空。下一步：填完整的接口地址（含协议与版本前缀 /v1），\
                        例如 http://127.0.0.1:18080/v1。"
                .into());
        }
        if self.base_url.len() > 512 {
            return Err(format!(
                "base_url 太长（{} 字符，上限 512）。下一步：去掉结尾多余的路径或查询串。",
                self.base_url.len()
            ));
        }
        if self.model.len() > 256 {
            return Err(format!(
                "model 太长（{} 字符，上限 256）。下一步：填 /models 列表里的原始模型 ID。"
                    ,
                self.model.len()
            ));
        }
        if self.max_context_tokens == 0 {
            return Err("max_context_tokens 必须 > 0。下一步：填模型真实的上下文窗口\
                        （可先用 GET /api/admin/providers/{id}/models 看探测到的 context_window）。"
                .into());
        }
        if self.compaction_threshold_tokens < MIN_COMPACTION_THRESHOLD_TOKENS {
            return Err(format!(
                "compaction_threshold_tokens={} 过小（必须 ≥ {}，否则一压就空）。\
                 下一步：留出至少 4k 给正文。",
                self.compaction_threshold_tokens, MIN_COMPACTION_THRESHOLD_TOKENS
            ));
        }
        if self.compaction_threshold_tokens > self.max_context_tokens {
            return Err(format!(
                "compaction_threshold_tokens({}) 不能大于 max_context_tokens({})。\
                 下一步：调小阈值或调大上下文窗口。",
                self.compaction_threshold_tokens, self.max_context_tokens
            ));
        }
        if self.max_output_tokens == 0 {
            return Err("max_output_tokens 必须 > 0。下一步：填单次回复的 token 上限（推理模型建议 4096 以上）。"
                .into());
        }
        if self.max_output_tokens > self.max_context_tokens {
            return Err(format!(
                "max_output_tokens({}) 不能大于 max_context_tokens({})。下一步：先扩窗口再放大单次预算。",
                self.max_output_tokens, self.max_context_tokens
            ));
        }
        Ok(())
    }

    /// 对外形状。**没有 api_key 字段** —— 只有 `has_api_key`。
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "preset_id": self.preset_id,
            "kind": self.kind.as_str(),
            "protocol": self.protocol.as_str(),
            "base_url": self.base_url,
            "has_api_key": !self.api_key.is_empty(),
            "model": self.model,
            "max_context_tokens": self.max_context_tokens,
            "compaction_threshold_tokens": self.compaction_threshold_tokens,
            "max_output_tokens": self.max_output_tokens,
            "enabled": self.enabled,
            "is_default": self.is_default,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }

    pub fn to_llm_config(&self) -> LlmConfig {
        let mut cfg = admin_config_of(self).to_llm_config();
        cfg.enabled = self.enabled && !self.base_url.trim().is_empty();
        cfg
    }

    /// `admin_config` 旧契约的投影。明文 api_key 只在进程内流转：
    /// 对外一律由 `admin_config_to_json` 折成 `has_api_key`。
    pub fn as_admin_config(&self) -> AdminConfig {
        admin_config_of(self)
    }

    /// 0002 表里历史存在的 `openrouter` 落到本表时映射成 `openai`。
    pub fn protocol_str(&self) -> &'static str {
        match self.protocol {
            Protocol::Anthropic => "anthropic",
            _ => "openai",
        }
    }

    pub fn parse_protocol(raw: &str) -> Result<Protocol, String> {
        match raw {
            "openai" | "openrouter" => Ok(Protocol::Openai),
            "anthropic" => Ok(Protocol::Anthropic),
            other => Err(format!(
                "protocol {other:?} 不在枚举里（provider 只支持 openai / anthropic）。\
                 下一步：改成这两个值之一再重试。"
            )),
        }
    }
}

fn admin_config_of(p: &LlmProvider) -> AdminConfig {
    AdminConfig {
        protocol: p.protocol,
        base_url: p.base_url.clone(),
        api_key: p.api_key.clone(),
        model: p.model.clone(),
        max_context_tokens: p.max_context_tokens,
        compaction_threshold_tokens: p.compaction_threshold_tokens,
        max_output_tokens: p.max_output_tokens,
        updated_at: p.updated_at,
    }
}

pub fn hex_upper(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

// ---------------------------------------------------------------------------
// 探测
// ---------------------------------------------------------------------------

/// 一个探测到的模型。`context_window` 为 `None` 表示上游没报这个信息 ——
/// 不编数字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCard {
    pub id: String,
    pub display_name: String,
    pub modality: &'static str,
    pub context_window: Option<u32>,
    pub owned_by: Option<String>,
}

impl ModelCard {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "display_name": self.display_name,
            "modality": self.modality,
            "context_window": self.context_window,
            "owned_by": self.owned_by,
        })
    }
}

/// 权重文件后缀：只用来剥掉文件名尾巴，不参与任何判断。
const WEIGHT_SUFFIXES: [&str; 6] = [".gguf", ".safetensors", ".ggml", ".bin", ".pt", ".pth"];

/// 从模型 ID 派生可读短名：取 basename 并剥掉权重后缀。
/// `D:\models\Qwen3.5-4B-Q4_K_M.gguf` → `Qwen3.5-4B-Q4_K_M`。
pub fn display_name_of(id: &str) -> String {
    let base = id
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(id)
        .trim()
        .to_string();
    let lower = base.to_ascii_lowercase();
    for suffix in WEIGHT_SUFFIXES {
        if lower.ends_with(suffix) && base.len() > suffix.len() {
            return base[..base.len() - suffix.len()].to_string();
        }
    }
    if base.is_empty() {
        id.to_string()
    } else {
        base
    }
}

fn first_positive_u32(v: &Value, keys: &[&str]) -> Option<u32> {
    for k in keys {
        let Some(n) = v.get(*k) else { continue };
        if let Some(u) = n.as_u64() {
            if u > 0 && u <= u64::from(u32::MAX) {
                return Some(u as u32);
            }
        }
    }
    None
}

const CONTEXT_KEYS: [&str; 3] = ["n_ctx_train", "context_length", "max_model_len"];

const TEXT_HINTS: [&str; 7] = [
    "text", "completion", "completions", "chat", "generate", "inference", "llm",
];

fn capability_tokens(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.to_ascii_lowercase()),
        Value::Array(items) => {
            for i in items {
                if let Some(s) = i.as_str() {
                    out.push(s.to_ascii_lowercase());
                }
            }
        }
        Value::Object(map) => {
            for (k, val) in map {
                if val.as_bool() == Some(false) {
                    continue;
                }
                out.push(k.to_ascii_lowercase());
            }
        }
        _ => {}
    }
}

/// modality 只在有**上游证据**时才判成 text，否则 unknown —— 不猜。
fn derive_modality(item: &Value, sibling: Option<&Value>) -> &'static str {
    let mut tokens = Vec::new();
    for src in [Some(item), sibling].into_iter().flatten() {
        for key in ["capabilities", "type", "modalities", "input_modalities"] {
            if let Some(v) = src.get(key) {
                capability_tokens(v, &mut tokens);
            }
        }
    }
    if tokens.iter().any(|t| TEXT_HINTS.contains(&t.as_str())) {
        "text"
    } else {
        "unknown"
    }
}

/// 从一次 `/models` 响应里抽出模型卡片。返回 Err 表示「这不是可用的
/// OpenAI 兼容 models 响应」——调用方必须把它当 unavailable，不许当成空列表。
pub fn parse_models_payload(payload: &Value) -> Result<Vec<ModelCard>, String> {
    let data = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "响应里没有 data 数组，不是 OpenAI 兼容的模型列表。".to_string())?;

    // llama.cpp 这类实现还会另给一份 `models`（带 capabilities），
    // 按 name/model 与 data[].id 对齐，只用来补派生字段。
    let siblings = payload.get("models").and_then(Value::as_array);
    let lookup = |id: &str| -> Option<&Value> {
        siblings?.iter().find(|m| {
            m.get("name").and_then(Value::as_str) == Some(id)
                || m.get("model").and_then(Value::as_str) == Some(id)
        })
    };

    let mut out = Vec::with_capacity(data.len());
    for item in data {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        let meta = item.get("meta");
        let context_window = meta
            .and_then(|m| first_positive_u32(m, &CONTEXT_KEYS))
            .or_else(|| first_positive_u32(item, &CONTEXT_KEYS));
        out.push(ModelCard {
            id: id.to_string(),
            display_name: display_name_of(id),
            modality: derive_modality(item, lookup(id)),
            context_window,
            owned_by: item
                .get("owned_by")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        });
    }
    if out.is_empty() {
        return Err("上游返回了空的模型列表（data 里没有任何带 id 的条目）。".to_string());
    }
    Ok(out)
}

fn models_url(base_url: &str) -> String {
    format!("{}/models", base_url.trim().trim_end_matches('/'))
}

/// 真实探测一次上游模型列表。Err 里是中文可读原因 + 修复方向。
pub async fn probe_models(base_url: &str, api_key: &str) -> Result<Vec<ModelCard>, String> {
    let url = models_url(base_url);
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败：{e}。下一步：检查本机 TLS 组件是否完整。"))?;

    let mut req = client.get(&url);
    let key = api_key.trim();
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            // reqwest 的原文是英文，翻译成能照着做的中文原因再交给前端。
            let cause = if e.is_timeout() {
                format!("超过 {} 秒仍无响应", PROBE_TIMEOUT.as_secs())
            } else if e.is_connect() {
                "连接被拒绝或主机不可达".to_string()
            } else if e.is_request() {
                "请求构造或发送失败".to_string()
            } else {
                "网络错误".to_string()
            };
            return Err(format!(
                "探测 {url} 失败：{cause}（{e}）。下一步：确认地址与端口可达\
                 （本地模型先启动 llama-server），路径要带版本前缀 /v1。"
            ));
        }
    };

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "{url} 返回 HTTP {}。下一步：核对 api_key 与该端点的访问权限（OpenAI 兼容端通常需要 Bearer 令牌）。",
            status.as_u16()
        ));
    }
    let payload: Value = serde_json::from_str(&text).map_err(|e| {
        format!(
            "{url} 的响应不是 JSON（{e}）。下一步：base_url 要指到 OpenAI 兼容的根路径（例如 .../v1），\
             而不是 HTML 页面或反向代理的错误页。"
        )
    })?;
    parse_models_payload(&payload)
}

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
    let protocol = LlmProvider::parse_protocol(&protocol_raw)
        .map_err(crate::db::invariant_broken)?;

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
                sqlx::query("UPDATE llm_providers SET is_default = 0 WHERE is_default = 1 AND id <> ?")
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
        id: LlmProvider::new_id()?,
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

/// 共享缓存句柄的便捷构造（测试与启动路径用）。
pub fn cache_of(providers: Vec<LlmProvider>) -> Arc<ProviderCache> {
    Arc::new(ProviderCache { providers })
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn context_window_prefers_the_trained_length_and_never_invents_one() {
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
            ]
        });
        let got = parse_models_payload(&payload).expect("合法响应必须解析");
        assert_eq!(got[0].context_window, Some(262144), "训练上下文优先于实例上下文");
        assert_eq!(
            got[1].context_window, None,
            "n_ctx 不是训练上下文，不许拿它顶包"
        );
        assert_eq!(got[2].context_window, Some(131072));
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
        assert_eq!(got[0].modality, "text", "同一响应里 models[].capabilities 是证据");
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
            id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_lowercase()),
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
