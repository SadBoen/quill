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

    /// **只有发生了什么**，不含 `Display` 追加的那句「→ 下一步：…」。
    ///
    /// 为什么要单独开一个出口：错误信封**已经有**一个结构化的 `next_step` 字段
    /// （`ApiError::next_step()`，`Page.tsx` 把它渲染成独立段落）。如果 detail 又用
    /// `Display` 把同一句下一步拼进去，界面上就会出现**两条自称「下一步」的段落**，
    /// 而且两条的具体程度往往差很远 ——
    /// 实测（2026-10-06，`sb-adaptive-cruise-control`）detail 里那条含糊地说
    /// 「请先按上面这句把配置改对」，而「上面这句」指的实际是上游那段 JSON。见 ISSUE-020。
    ///
    /// **`Display` 本身一个字都没改**：`Display` 仍然带下一步，因为流式、CLI、
    /// agent 内部这些地方没有 `ApiError::next_step()` 可用，它们靠的就是 `Display`。
    /// 这次只让 HTTP 信封改用 `message()`，好让「下一步」在信封里**只有一个出口**。
    pub fn message(&self) -> String {
        match self {
            Self::NotConfigured { detail } => format!("LLM 服务还没配置：{detail}"),
            Self::Unreachable { url, detail } => format!(
                "连不上 LLM 服务 {url}：{detail}。本地模型最常见的原因是 llama-server 根本没起来"
            ),
            Self::Timeout { detail } => format!("调用 LLM 服务超时：{detail}"),
            Self::Status { code, body } => {
                let mut s = format!("LLM 服务返回 HTTP {code}：{body}");
                if *code == 401 || *code == 403 {
                    s.push_str(
                        "。401/403 基本都是 API key 缺失或已失效，检查构造 provider 时传的 key",
                    );
                }
                s
            }
            Self::ModelNotFound { model, detail } => {
                format!("LLM 服务上没有名为「{model}」的模型：{detail}")
            }
            Self::MalformedResponse { detail } => format!(
                "LLM 服务回了看不懂的内容：{detail}。这不是重试能解决的，请把这一行连同服务端的日志一起反馈"
            ),
            Self::UpstreamRejected { detail } => {
                format!("LLM 服务在处理请求时拒绝了它：{detail}")
            }
            Self::InvalidRequest { detail } => format!("发给 LLM 服务的请求本身不合法：{detail}"),
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
    /// = `message()` + `tail()`。
    ///
    /// **不要再在这里另写一份变体文案**：正文只有 `message()` 一个来源，
    /// 否则两条出口迟早会跑偏（`message()` 是 HTTP 错误信封 detail 用的那个）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())?;
        tail(f, self)
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

    /// ISSUE-020 的另一半：拆出 `message()` **只是**为了让 HTTP 信封的 detail
    /// 不再重复「下一步」，**不是**把下一步删掉。
    ///
    /// 流式、CLI、agent 内部那些地方没有 `ApiError::next_step()` 可用，
    /// 它们靠的就是 `Display`。所以这里钉死：`Display` 仍带下一步，
    /// `message()` 只带发生了什么。
    #[test]
    fn message_drops_the_next_step_but_display_keeps_it() {
        for e in one_of_each() {
            let bare = e.message();
            let shown = e.to_string();

            assert!(
                !bare.contains("下一步"),
                "[{}] message() 不该带下一步：\n{bare}",
                e.code()
            );
            assert!(
                shown.contains("下一步"),
                "[{}] Display 必须仍带下一步：\n{shown}",
                e.code()
            );
            // Display 只能比 message() 多那一段，多出来的就该是 tail。
            assert!(
                shown.starts_with(&bare),
                "[{}] Display 应以 message() 开头：\n{shown}",
                e.code()
            );
        }
    }
}
