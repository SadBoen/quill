use std::sync::Arc;
use std::time::Duration;

use quill_provider::{ChatRequest, Message, OpenAiCompatible, SharedProvider};

/// 实例级 LLM 协议。`admin_config.protocol` 的合法取值。
/// 三个值都映射到 OpenAI 兼容请求（OpenRouter / Anthropic 兼容端都走
/// `/v1/chat/completions`），先把这层耦合点放在一起，后续真接 Anthropic
/// 原生 Messages 时再按协议分流。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Openai,
    Anthropic,
    Openrouter,
}

impl Protocol {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "openai" => Some(Self::Openai),
            "anthropic" => Some(Self::Anthropic),
            "openrouter" => Some(Self::Openrouter),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Openrouter => "openrouter",
        }
    }
}

/// 前端拿到字符串直接下拉显示，不强求 enum；这里只用来校验写入合法。
impl Default for Protocol {
    fn default() -> Self {
        Self::Openai
    }
}

/// admin 表行（单行 id=1）形状。所有字段都 NOT NULL；调用方读不到时
/// 由 `AdminConfig::from_llm_config` / `to_llm_config` 在「实例视图」里做转换。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminConfig {
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_context_tokens: u32,
    pub compaction_threshold_tokens: u32,
    pub max_output_tokens: u32,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmConfig {
    pub protocol: Protocol,

    pub base_url: String,

    pub model: String,

    pub api_key: Option<String>,

    pub timeout_secs: u64,

    /// 单次出参上限。和 `AdminConfig::max_output_tokens` 一一对应；保留旧名
    /// 是为了不破坏 `QUILL_LLM_MAX_TOKENS` 这条环境变量契约。
    pub max_tokens: u32,

    /// 上下文窗口上限。新字段，env var 路径用 `DEFAULT_MAX_CONTEXT_TOKENS`。
    pub max_context_tokens: u32,

    /// 触发压紧的 token 阈值。新字段，env var 路径默认 = max_context_tokens。
    pub compaction_threshold_tokens: u32,

    pub enabled: bool,
}

/// 与不设任何环境变量时的行为一致。测试和「先造一个空壳再逐项覆盖」的
/// 构造路径都走这里，避免每加一个字段就把所有构造点编译打穿。
impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            protocol: Protocol::Openai,
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            api_key: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_tokens: DEFAULT_MAX_TOKENS,
            max_context_tokens: DEFAULT_MAX_CONTEXT_TOKENS,
            compaction_threshold_tokens: DEFAULT_COMPACTION_THRESHOLD_TOKENS,
            enabled: true,
        }
    }
}

/// 必须与仓库里 `.wsl-llama.sh` 实际启动的端口一致，否则不设环境变量时
/// 默认连不上；同时和 `quill-provider` 的 `LLAMA_CPP_BASE_URL` 保持同一口径。
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:18080/v1";
pub const DEFAULT_MODEL: &str = "local";
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// 推理模型（Qwen3.5 等）会先把预算花在 thinking 上：实测 4B 模型在 2048 下
/// thinking 就吃光额度、只回思考没有正文，4096 才稳。宁可多留也别让用户
/// 看到空回复——超限会走 `provider_unavailable` 并提示调这个旋钮。
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

// 这条断言在构造上不会失败（clippy 对常量 assert 的提示是对的），所以用**编译期**
// 断言：改小 DEFAULT_MAX_TOKENS 的人会在构建时被拦下，而不是等跑测试。
const _: () = assert!(DEFAULT_MAX_TOKENS >= 4096, "默认 token 预算对推理模型太小");
pub const DEFAULT_MAX_CONTEXT_TOKENS: u32 = 32768;
pub const DEFAULT_COMPACTION_THRESHOLD_TOKENS: u32 = 8000;
/// `compaction_threshold_tokens` 的下限：4001 留出正文空间，避免一压就空。
pub const MIN_COMPACTION_THRESHOLD_TOKENS: u32 = 4001;

impl LlmConfig {
    pub fn from_env() -> (Self, Vec<crate::config::Warning>) {
        let mut warnings = Vec::new();

        let base_url = env_or("QUILL_LLM_BASE_URL", DEFAULT_BASE_URL, "QUILL_LLM_BASE_URL", &mut warnings);
        let model = env_or("QUILL_LLM_MODEL", DEFAULT_MODEL, "QUILL_LLM_MODEL", &mut warnings);
        let api_key = std::env::var("QUILL_LLM_API_KEY")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let timeout_secs = std::env::var("QUILL_LLM_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let max_tokens = std::env::var("QUILL_LLM_MAX_TOKENS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MAX_TOKENS);

        // 新字段：env var 路径下用默认值；admin 表路径下会被 `to_llm_config` 覆盖。
        let max_context_tokens = std::env::var("QUILL_LLM_MAX_CONTEXT_TOKENS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MAX_CONTEXT_TOKENS);

        let compaction_threshold_tokens = std::env::var("QUILL_LLM_COMPACTION_THRESHOLD_TOKENS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .filter(|n| *n >= MIN_COMPACTION_THRESHOLD_TOKENS)
            .unwrap_or(DEFAULT_COMPACTION_THRESHOLD_TOKENS);

        (
            Self {
                protocol: Protocol::Openai,
                enabled: !base_url.is_empty(),
                base_url,
                model,
                api_key,
                timeout_secs,
                max_tokens,
                max_context_tokens,
                compaction_threshold_tokens,
            },
            warnings,
        )
    }
}

fn env_or(
    key: &str,
    default: &str,
    source: &str,
    warnings: &mut Vec<crate::config::Warning>,
) -> String {
    match std::env::var(key) {
        Err(_) => default.to_string(),
        Ok(raw) => {
            let t = raw.trim();
            if t.is_empty() {
                warnings.push(crate::config::Warning {
                    source: source.to_string(),
                    message: format!("设为空串，已回退默认 {default}。"),
                });
                default.to_string()
            } else {
                t.trim_end_matches('/').to_string()
            }
        }
    }
}

pub fn build(cfg: &LlmConfig) -> Result<SharedProvider, String> {
    OpenAiCompatible::new(cfg.base_url.clone(), cfg.model.clone(), cfg.api_key.clone())
        .map(|p| Arc::new(p.with_timeout(Duration::from_secs(cfg.timeout_secs))) as SharedProvider)
        .map_err(|e| e.to_string())
}

/// 推理模型会把 token 预算花在思考上，预算太小就只有一个空回复。
/// 这里给足下限，并在超限时明确告诉调用方是哪个旋钮要调。
pub fn build_request(cfg: &LlmConfig, messages: Vec<Message>) -> ChatRequest {
    ChatRequest::new(cfg.model.clone(), messages).with_max_tokens(cfg.max_tokens)
}

impl AdminConfig {
    /// 给 admin API 用的服务端字段校验。校验失败返回中文 next_step。
    /// 不在这里写数据库 — 数据库侧的 CHECK 由 `admin_config` 表自己承担，
    /// 这一层只校验语义（"compaction 必须留出正文空间"等）。
    pub fn validate(&self) -> Result<(), String> {
        if self.base_url.trim().is_empty() {
            return Err("base_url 不能为空。下一步：填上完整的接口地址（含协议与 /v1），例如 http://127.0.0.1:18080/v1。".into());
        }
        if self.model.trim().is_empty() {
            return Err("model 不能为空。下一步：从模型服务的 /v1/models 列出的名字里选一个填上。".into());
        }
        if self.max_context_tokens == 0 {
            return Err("max_context_tokens 必须 > 0。下一步：把它设成模型能容纳的历史长度（默认 32768）。".into());
        }
        if self.compaction_threshold_tokens < MIN_COMPACTION_THRESHOLD_TOKENS {
            return Err(format!(
                "compaction_threshold_tokens={} 过小（必须 ≥ {}，否则一压就空）。下一步：留出至少 4k 给正文。",
                self.compaction_threshold_tokens, MIN_COMPACTION_THRESHOLD_TOKENS
            ));
        }
        if self.max_output_tokens == 0 {
            return Err("max_output_tokens 必须 > 0。下一步：把它设成你希望单次回复的最大 token 数（默认 4096，推理模型需要更大）。".into());
        }
        if self.compaction_threshold_tokens > self.max_context_tokens {
            return Err(format!(
                "compaction_threshold_tokens({}) 不能大于 max_context_tokens({})。\
                 下一步：调小阈值或调大上下文窗口。",
                self.compaction_threshold_tokens, self.max_context_tokens
            ));
        }
        if self.max_output_tokens > self.max_context_tokens {
            return Err(format!(
                "max_output_tokens({}) 不能大于 max_context_tokens({})。\
                 下一步：先扩窗口再放大单次预算。",
                self.max_output_tokens, self.max_context_tokens
            ));
        }
        Ok(())
    }

    pub fn to_llm_config(&self) -> LlmConfig {
        LlmConfig {
            protocol: self.protocol,
            base_url: self.base_url.trim().trim_end_matches('/').to_string(),
            model: self.model.trim().to_string(),
            api_key: if self.api_key.is_empty() {
                None
            } else {
                Some(self.api_key.clone())
            },
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_tokens: self.max_output_tokens,
            max_context_tokens: self.max_context_tokens,
            compaction_threshold_tokens: self.compaction_threshold_tokens,
            enabled: !self.base_url.trim().is_empty(),
        }
    }

    pub fn from_llm_config(cfg: &LlmConfig, updated_at: i64) -> Self {
        Self {
            protocol: cfg.protocol,
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone().unwrap_or_default(),
            model: cfg.model.clone(),
            max_context_tokens: cfg.max_context_tokens,
            compaction_threshold_tokens: cfg.compaction_threshold_tokens,
            max_output_tokens: cfg.max_tokens,
            updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_endpoint_is_the_one_the_repo_script_actually_starts() {
        assert_eq!(DEFAULT_BASE_URL, quill_provider::LLAMA_CPP_BASE_URL);
        assert!(
            DEFAULT_BASE_URL.contains(":18080"),
            "默认端口与 .wsl-llama.sh 不一致，不设环境变量时永远连不上：{DEFAULT_BASE_URL}"
        );
    }

    #[test]
    fn default_matches_from_env_with_nothing_set() {
        let d = LlmConfig::default();
        assert_eq!(d.base_url, DEFAULT_BASE_URL);
        assert_eq!(d.model, DEFAULT_MODEL);
        assert_eq!(d.api_key, None);
        assert!(d.enabled);
        assert_eq!(d.max_context_tokens, DEFAULT_MAX_CONTEXT_TOKENS);
        assert_eq!(
            d.compaction_threshold_tokens,
            DEFAULT_COMPACTION_THRESHOLD_TOKENS
        );
    }

    #[test]
    fn build_request_carries_the_configured_budget() {
        let cfg = LlmConfig {
            max_tokens: 900,
            ..Default::default()
        };
        let req = build_request(&cfg, vec![Message::user("hi")]);
        assert_eq!(req.model, DEFAULT_MODEL);
        assert_eq!(req.max_tokens, Some(900));
    }

    #[test]
    fn protocol_round_trips_through_parse_and_as_str() {
        for (s, p) in [
            ("openai", Protocol::Openai),
            ("anthropic", Protocol::Anthropic),
            ("openrouter", Protocol::Openrouter),
        ] {
            assert_eq!(Protocol::parse(s), Some(p));
            assert_eq!(p.as_str(), s);
        }
        assert_eq!(Protocol::parse("bogus"), None);
    }

    #[test]
    fn validate_rejects_compaction_above_context() {
        let bad = AdminConfig {
            protocol: Protocol::Openai,
            base_url: "http://x/v1".into(),
            api_key: "".into(),
            model: "m".into(),
            max_context_tokens: 8000,
            compaction_threshold_tokens: 9000,
            max_output_tokens: 2048,
            updated_at: 0,
        };
        let err = bad.validate().expect_err("必须被拒");
        assert!(err.contains("下一步"), "错误必须给出修复方向：{err}");
        assert!(err.contains("compaction_threshold"), "要点名出错字段：{err}");
    }

    #[test]
    fn admin_config_round_trips_through_llm_config() {
        let a = AdminConfig {
            protocol: Protocol::Openrouter,
            base_url: "http://x/v1".into(),
            api_key: "  sk-abc  ".into(),
            model: "m".into(),
            max_context_tokens: 32768,
            compaction_threshold_tokens: 8000,
            max_output_tokens: 4096,
            updated_at: 42,
        };
        let l = a.to_llm_config();
        assert_eq!(l.protocol, Protocol::Openrouter);
        assert_eq!(l.base_url, "http://x/v1");
        assert_eq!(l.api_key.as_deref(), Some("  sk-abc  "));
        assert_eq!(l.max_tokens, 4096);
        assert_eq!(l.max_context_tokens, 32768);
        let back = AdminConfig::from_llm_config(&l, 99);
        assert_eq!(back.api_key, "  sk-abc  ");
        assert_eq!(back.protocol, Protocol::Openrouter);
        assert_eq!(back.updated_at, 99);
    }
}
