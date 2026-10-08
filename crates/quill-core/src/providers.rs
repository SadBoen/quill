//! provider 的**内核侧**模型、校验与探测（queue Q015）。
//!
//! **搬家记录**：本文件的内容原在 `quill-server/src/llm_providers.rs`（那份文件
//! 把「存储」与「provider 侧协议知识」混在一起）。2026-10-08 按
//! `docs/KERNEL-PORTS.md §3` 切开：**协议知识住内核**（模型行模型与校验、
//! 预设/自定义种类、`/models` 探测与解析、模型卡的派生字段），
//! **存储（9 条 SQL）留在壳**（`quill-server::llm_providers`，它对这里做 re-export）。
//!
//! 为什么这些属于内核：它们是「与上游模型端点打交道」的知识，跟
//! `vendor/goose/crates/goose/src/providers/` 里各 provider 的模型清单解析是同一层；
//! 留在 HTTP 层会让 CLI 与测试都用不上。
//!
//! 铁律照旧（原文件头）：模型池里的每一个模型都必须来自上游 `/models` 的真实响应，
//! 探测不到就报中文原因，绝不退回硬编码列表，也不编 `context_window`。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::llm::{AdminConfig, LlmConfig, Protocol, MIN_COMPACTION_THRESHOLD_TOKENS};

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
    /// 32 位大写 hex 的新 id。**错误是普通字符串**（不是壳的 `ApiError`）——
    /// 内核不认壳的错误类型；调用方自己决定把它折成哪个 HTTP 状态码。
    pub fn new_id() -> Result<String, String> {
        let mut b = [0u8; 16];
        getrandom::fill(&mut b)
            .map_err(|e| format!("生成供应商标识失败（系统随机源不可用）：{e}"))?;
        if b == [0u8; 16] {
            b[0] = 1;
        }
        Ok(quill_adapters::ids::to_hex_upper(&b))
    }

    /// 服务端语义校验。数据库侧的 CHECK 承担同样的边界，这里负责「说人话」。
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err(
                "name 不能为空。下一步：给它起一个能区分的显示名，例如「本地 llama.cpp」。".into(),
            );
        }
        if self.preset_id.trim().is_empty() {
            return Err("preset_id 不能为空。下一步：自定义端点填 custom；\
                        预设供应商填预设卡片上的标识。"
                .into());
        }
        if self.base_url.trim().is_empty() {
            return Err(
                "base_url 不能为空。下一步：填完整的接口地址（含协议与版本前缀 /v1），\
                        例如 http://127.0.0.1:18080/v1。"
                    .into(),
            );
        }
        if self.base_url.len() > 512 {
            return Err(format!(
                "base_url 太长（{} 字符，上限 512）。下一步：去掉结尾多余的路径或查询串。",
                self.base_url.len()
            ));
        }
        if self.model.len() > 256 {
            return Err(format!(
                "model 太长（{} 字符，上限 256）。下一步：填 /models 列表里的原始模型 ID。",
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

/// 「这条请求最多能带多少 token」的上游字段候选，**按可信度从高到低**排列。
///
/// `first_positive_u32` 取**第一个命中**的，所以顺序本身就是判断。
///
/// **`n_ctx_train` 被故意排除在外。** llama.cpp 的 `meta` 同时报两个数：
/// - `meta.n_ctx` —— **服务器当前开着的**窗口（用 `-c` 指定的）。
/// - `meta.n_ctx_train` —— 模型**被训练时**的窗口，与这次部署无关。
///
/// 本机实测（2026-10-06，llama-b10068 + Qwen3.5-4B）：
/// `n_ctx = 8192`、`n_ctx_train = 262144`，同一个响应里同时出现。
/// 原来的列表把 `n_ctx_train` 排在**第一位**，于是探测报出 262144 ——
/// 比真实窗口大 **32 倍**。这个数会流到
/// `GET /api/admin/providers/{id}/models` 给界面显示，而 `validate()` 又让用户
/// 「填模型真实的上下文窗口（可先用 …/models 看探测到的 context_window）」——
/// 等于**界面在教用户填一个偏大 32 倍的数**。见 ISSUE-022。
///
/// 只报 `n_ctx_train` 的端点现在会得到 `None`（**不知道**）而不是那个偏大的数。
/// 这是刻意的：`None` 会让界面说「上游没报这个信息」，而报一个错的数会让用户
/// 照着把配置改坏 —— 后者远比前者麻烦。
const CONTEXT_KEYS: [&str; 3] = ["n_ctx", "context_length", "max_model_len"];

const TEXT_HINTS: [&str; 7] = [
    "text",
    "completion",
    "completions",
    "chat",
    "generate",
    "inference",
    "llm",
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

pub fn cache_of(providers: Vec<LlmProvider>) -> Arc<ProviderCache> {
    Arc::new(ProviderCache { providers })
}
