use std::fmt;

pub const DOCTOR_CMD: &str = "quill doctor";
pub const LOCAL_MODELS_PROBE: &str = "curl -sS http://127.0.0.1:8080/v1/models";

/// Error-body excerpt cap, so a pathological gateway page cannot be pasted whole into a message.
pub const MAX_BODY_CHARS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    NotConfigured {
        detail: String,
    },

    Unreachable {
        url: String,

        detail: String,
    },

    Timeout {
        detail: String,
    },

    Status {
        code: u16,

        body: String,
    },

    ModelNotFound {
        model: String,

        detail: String,
    },

    MalformedResponse {
        detail: String,
    },

    UpstreamRejected {
        detail: String,
    },

    InvalidRequest {
        detail: String,
    },
}

impl ProviderError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotConfigured { .. } => "provider_not_configured",
            Self::Unreachable { .. } => "provider_unreachable",
            Self::Timeout { .. } => "provider_timeout",
            Self::Status { .. } => "provider_status",
            Self::ModelNotFound { .. } => "provider_model_not_found",
            Self::MalformedResponse { .. } => "provider_malformed_response",
            Self::UpstreamRejected { .. } => "provider_upstream_rejected",
            Self::InvalidRequest { .. } => "provider_invalid_request",
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Unreachable { .. } | Self::Timeout { .. } => true,
            Self::Status { code, .. } => *code == 408 || *code == 429 || (500..600).contains(code),
            _ => false,
        }
    }

    pub fn fix_command(&self) -> &'static str {
        match self {
            Self::NotConfigured { .. }
            | Self::Timeout { .. }
            | Self::Status { .. }
            | Self::MalformedResponse { .. }
            | Self::UpstreamRejected { .. }
            | Self::InvalidRequest { .. } => DOCTOR_CMD,
            Self::Unreachable { .. } | Self::ModelNotFound { .. } => LOCAL_MODELS_PROBE,
        }
    }
}

/// Shorten a response body to a single-line excerpt suitable for an error message.
pub fn excerpt(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_BODY_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(MAX_BODY_CHARS).collect();
    format!("{head}…")
}

/// Drop credentials, query and fragment from a URL before it reaches a message or a log.
pub fn redact_url(raw: &str) -> String {
    let no_query = raw.split('?').next().unwrap_or(raw);
    let no_fragment = no_query.split('#').next().unwrap_or(no_query);
    match no_query.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('@') {
            Some((_, host)) => format!("{scheme}://{host}"),
            None => no_fragment.to_string(),
        },
        None => no_fragment.to_string(),
    }
}

fn tail(f: &mut fmt::Formatter<'_>, e: &ProviderError) -> fmt::Result {
    write!(f, "\n→ 下一步：执行 `{}`", e.fix_command())?;
    if e.is_retryable() {
        write!(f, "；重试有可能成功（系统不会自动重跑，请手动重试）")?;
    } else {
        write!(f, "；重试没有意义，请先按上面这句把配置改对")?;
    }
    Ok(())
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured { detail } => {
                write!(f, "LLM 服务还没配置：{detail}")?;
                tail(f, self)
            }
            Self::Unreachable { url, detail } => {
                write!(
                    f,
                    "连不上 LLM 服务 {url}：{detail}。本地模型最常见的原因是 llama-server 根本没起来"
                )?;
                tail(f, self)
            }
            Self::Timeout { detail } => {
                write!(f, "调用 LLM 服务超时：{detail}")?;
                tail(f, self)
            }
            Self::Status { code, body } => {
                write!(f, "LLM 服务返回 HTTP {code}：{body}")?;
                if *code == 401 || *code == 403 {
                    write!(f, "。401/403 基本都是 API key 缺失或已失效，检查构造 provider 时传的 key")?;
                }
                tail(f, self)
            }
            Self::ModelNotFound { model, detail } => {
                write!(f, "LLM 服务上没有名为「{model}」的模型：{detail}")?;
                tail(f, self)
            }
            Self::MalformedResponse { detail } => {
                write!(
                    f,
                    "LLM 服务回了看不懂的内容：{detail}。这不是重试能解决的，请把这一行连同服务端的日志一起反馈"
                )?;
                tail(f, self)
            }
            Self::UpstreamRejected { detail } => {
                write!(f, "LLM 服务在处理请求时拒绝了它：{detail}")?;
                tail(f, self)
            }
            Self::InvalidRequest { detail } => {
                write!(f, "发给 LLM 服务的请求本身不合法：{detail}")?;
                tail(f, self)
            }
        }
    }
}

impl std::error::Error for ProviderError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_of_each() -> Vec<ProviderError> {
        vec![
            ProviderError::NotConfigured {
                detail: "base_url 为空".into(),
            },
            ProviderError::Unreachable {
                url: "http://127.0.0.1:8080/v1/chat/completions".into(),
                detail: "connection refused".into(),
            },
            ProviderError::Timeout {
                detail: "等了 30s".into(),
            },
            ProviderError::Status {
                code: 500,
                body: "internal error".into(),
            },
            ProviderError::ModelNotFound {
                model: "qwen".into(),
                detail: "not found".into(),
            },
            ProviderError::MalformedResponse {
                detail: "choices 不是数组".into(),
            },
            ProviderError::UpstreamRejected {
                detail: "context length exceeded".into(),
            },
            ProviderError::InvalidRequest {
                detail: "messages 为空".into(),
            },
        ]
    }

    #[test]
    fn every_display_names_its_own_fix_command() {
        for e in one_of_each() {
            let msg = e.to_string();
            assert!(
                msg.contains(e.fix_command()),
                "[{}] 文案没给出 fix_command()：\n{msg}",
                e.code()
            );
            assert!(
                msg.contains(if e.is_retryable() {
                    "重试有可能成功"
                } else {
                    "重试没有意义"
                }),
                "[{}] 文案没说清重试有没有用：\n{msg}",
                e.code()
            );
        }
    }

    #[test]
    fn every_display_carries_the_offending_identity() {
        let cases: Vec<(ProviderError, &str)> = vec![
            (
                ProviderError::Unreachable {
                    url: "http://127.0.0.1:8080/v1".into(),
                    detail: "refused".into(),
                },
                "127.0.0.1:8080",
            ),
            (
                ProviderError::Status {
                    code: 503,
                    body: "upstream down".into(),
                },
                "503",
            ),
            (
                ProviderError::ModelNotFound {
                    model: "qwen2.5-3b".into(),
                    detail: "404".into(),
                },
                "qwen2.5-3b",
            ),
        ];
        for (e, needle) in cases {
            assert!(
                e.to_string().contains(needle),
                "[{}] 文案丢了关键标识 {needle}：{e}",
                e.code()
            );
        }
    }

    #[test]
    fn fix_commands_are_single_line_and_non_empty() {
        for e in one_of_each() {
            let cmd = e.fix_command();
            assert!(!cmd.is_empty(), "[{}] 命令是空的", e.code());
            assert!(!cmd.contains('\n'), "[{}] 命令不是单行：{cmd}", e.code());
            assert!(!cmd.ends_with(' '), "[{}] 命令尾部有多余空格", e.code());
        }
    }

    #[test]
    fn codes_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for e in one_of_each() {
            assert!(seen.insert(e.code()), "错误码重复：{}", e.code());
        }
        assert_eq!(seen.len(), 8, "8 个变体的错误码必须互不相同");
    }

    #[test]
    fn only_transport_and_5xx_are_retryable() {
        for e in one_of_each() {
            let expect = matches!(e, ProviderError::Unreachable { .. } | ProviderError::Timeout { .. })
                || matches!(e, ProviderError::Status { code, .. }
                    if code == 408 || code == 429 || (500..600).contains(&code));
            assert_eq!(e.is_retryable(), expect, "[{}] 重试判定不对", e.code());
        }

        assert!(!ProviderError::ModelNotFound {
            model: "x".into(),
            detail: String::new(),
        }
        .is_retryable());
        assert!(ProviderError::Status {
            code: 429,
            body: String::new(),
        }
        .is_retryable());
    }

    #[test]
    fn excerpt_flattens_and_truncates_on_a_char_boundary() {
        assert_eq!(excerpt("  a\n\t b  "), "a b");
        assert_eq!(excerpt(""), "");

        let long = "错".repeat(MAX_BODY_CHARS + 50);
        let got = excerpt(&long);
        assert!(got.ends_with('…'), "超长正文要截断标记：{}", got.len());
        assert_eq!(got.chars().count(), MAX_BODY_CHARS + 1);
    }

    #[test]
    fn redact_url_drops_credentials_query_and_fragment() {
        assert_eq!(
            redact_url("https://user:pw@api.example.com/v1/chat?key=sk-secret#frag"),
            "https://api.example.com/v1/chat"
        );
        assert_eq!(
            redact_url("http://127.0.0.1:8080/v1/models"),
            "http://127.0.0.1:8080/v1/models"
        );
        assert_eq!(redact_url("v1/models"), "v1/models");
    }

    #[test]
    fn auth_status_message_points_at_the_api_key() {
        let msg = ProviderError::Status {
            code: 401,
            body: "invalid api key".into(),
        }
        .to_string();
        assert!(msg.contains("API key"), "401 必须点出 API key：{msg}");
    }
}
