//! 上下文压缩的**壳侧接线**（queue Q018）。
//!
//! 内核侧 [`quill_core::compaction`] 只有纯逻辑（何时压、压成什么形状、前后账怎么
//! 连续），两样只有壳才知道的东西由这里注入：
//!
//! 1. **用哪个模型生成摘要** —— [`ProviderSummarizer`]，就是这一轮对话用的 provider；
//! 2. **怎么数 token** —— [`CharHeuristic`]，近似值（见它的文档）。
//!
//! 接线位置：`api_chat::prepare_turn` 拼完历史之后、交给对话循环之前。压缩后的历史
//! 替换掉原历史（原历史仍在 `messages` 表里，本模块不做归档写库 —— 内核交回的
//! `archived` 就是给调用方用的，quill 的 messages 表本来就已经存着它们）。
//!
//! **为什么压缩失败不让整轮失败**：超阈值却压不动（比如摘要请求本身也超窗口）时，
//! 直接把这一轮报错等于「聊天彻底不可用」；而按原历史继续至少可能答得出。所以这里
//! 照原历史继续，但**在响应里如实带上 `note`**（`compacted:false` + 原因）——
//! 静默降级才是这条仓库不允许的。

use std::future::Future;
use std::pin::Pin;

use quill_core::compaction::{
    compact, CompactionError, CompactionMessage, CompactionMode, CompactionModel,
    CompactionOutcome, GeneratedSummary, MessageRole, TokenEstimator,
};
use quill_core::llm::LlmConfig;
use quill_provider::{Message, ProviderError, SharedProvider};

/// token 估算：**近似值，不是真分词**。
///
/// 规则：ASCII 字符 4 个算 1 token，非 ASCII（中文为主）1 个字符算 1 token。
/// 这是这类「没带分词器时」的常用折算，对中英混排比「一律 chars/4」诚实得多 ——
/// 中文按 chars/4 会低报约 4 倍，阈值就永远打不到，压缩形同没接。
///
/// **未验证**：真实 token 数由模型端点自己的分词器决定，这里不去猜它。所以响应里
/// 报的是 `estimated_history_tokens`（估算），界面必须照这个说法显示。
pub struct CharHeuristic;

impl TokenEstimator for CharHeuristic {
    fn count_text_tokens(&self, text: &str) -> usize {
        let (ascii, wide) = text.chars().fold((0usize, 0usize), |(ascii, wide), c| {
            if c.is_ascii() {
                (ascii + 1, wide)
            } else {
                (ascii, wide + 1)
            }
        });
        ascii.div_ceil(4) + wide
    }
}

/// 用这一轮对话的 provider 生成摘要（就是内核 [`CompactionModel`] 的壳侧实现）。
struct ProviderSummarizer<'a> {
    provider: &'a SharedProvider,
    config: &'a LlmConfig,
}

impl CompactionModel for ProviderSummarizer<'_> {
    fn complete<'a>(
        &'a self,
        system: &'a str,
        request: &'a [CompactionMessage],
    ) -> Pin<Box<dyn Future<Output = Result<GeneratedSummary, CompactionError>> + Send + 'a>> {
        Box::pin(async move {
            let mut messages = Vec::with_capacity(request.len() + 1);
            messages.push(Message::system(system));
            for m in request {
                messages.push(match m.role {
                    MessageRole::User => Message::user(m.text.clone()),
                    MessageRole::Assistant => Message::assistant(m.text.clone()),
                });
            }
            let chat_request = quill_core::llm::build_request(self.config, messages);
            match self.provider.chat(&chat_request).await {
                Ok(reply) => Ok(GeneratedSummary {
                    raw_output: reply.answer().to_string(),
                    usage: kernel_usage(reply.usage),
                }),
                // **不按关键字猜「是不是上下文超了」**：quill-provider 没有可靠的分类，
                // 照 body 里的词去猜就是替上游说话。一律归到 `Model`，代价是内核那条
                // 「逐级删工具响应重试」的阶梯在本项目里不会触发（内核文档已如实记；
                // 那需要先有一个可信的分类器，属后续工作）。
                Err(e) => Err(provider_error_is_not_guessed(e)),
            }
        })
    }
}

fn kernel_usage(usage: quill_provider::TokenUsage) -> quill_core::compaction::TokenUsage {
    quill_core::compaction::TokenUsage {
        input_tokens: usage.input.map(|v| v as usize),
        output_tokens: usage.output.map(|v| v as usize),
        // `None`：让内核的 `ensure_usage_tokens` 按 goose 口径重算 total = input + output。
        total_tokens: None,
    }
}

/// 这一轮开始前，历史相对阈值处在什么状态。**给界面看的实话**，逐字段都有真来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextNotice {
    /// 压缩前对**历史消息**的 token 估算（不含 system 人格消息与工具 schema ——
    /// 那两样不进这个估算，见模块头）。
    pub estimated_history_tokens: usize,
    /// 配置里的绝对阈值（`compaction_threshold_tokens`）。
    pub threshold_tokens: usize,
    /// 这轮是否真的压缩了。
    pub compacted: bool,
    /// 压缩后 agent 可见历史（含摘要与续跑提示）的估算量；未压缩时等于估算值。
    pub after_tokens: usize,
    /// 摘要消息自身的估算量；未压缩为 0。
    pub summary_tokens: usize,
    /// 该压却没压成时的原因。`None` = 没有异常（含「没超阈值，本来就不该压」）。
    pub note: Option<String>,
}

impl ContextNotice {
    /// 组装成响应里的 `context` 对象。
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "estimated_history_tokens": self.estimated_history_tokens,
            "threshold_tokens": self.threshold_tokens,
            "compacted": self.compacted,
            "after_tokens": self.after_tokens,
            "summary_tokens": self.summary_tokens,
            "note": self.note,
        })
    }
}

/// 把历史行变成内核的压缩输入。
fn kernel_history(history: &[(String, String)]) -> Vec<CompactionMessage> {
    history
        .iter()
        .map(|(role, text)| {
            if role == "user" {
                CompactionMessage::user(text.clone())
            } else {
                CompactionMessage::assistant(text.clone())
            }
        })
        .collect()
}

/// 历史行的默认映射（不压缩时用）。
fn plain_history(history: &[(String, String)]) -> Vec<Message> {
    history
        .iter()
        .filter_map(|(role, text)| match role.as_str() {
            "user" => Some(Message::user(text.clone())),
            "assistant" if !text.trim().is_empty() => Some(Message::assistant(text.clone())),
            _ => None,
        })
        .collect()
}

/// 判断并在必要时压缩历史，返回「给对话循环用的消息」与「给界面看的实话」。
///
/// 阈值来自 `config.compaction_threshold_tokens` —— 这就是 Q018 要的那件事：
/// 这个值从此**真的被读**，且不超阈值时一次模型都不调（内核用「调用次数 == 0」钉住）。
pub async fn prepare_history(
    provider: &SharedProvider,
    config: &LlmConfig,
    history: &[(String, String)],
) -> (Vec<Message>, ContextNotice) {
    let estimator = CharHeuristic;
    let kernel_history = kernel_history(history);
    let estimated = estimator.count_chat_tokens("", &kernel_history);
    let threshold = config.compaction_threshold_tokens as usize;

    let base = ContextNotice {
        estimated_history_tokens: estimated,
        threshold_tokens: threshold,
        compacted: false,
        after_tokens: estimated,
        summary_tokens: 0,
        note: None,
    };

    let model = ProviderSummarizer { provider, config };
    let outcome = match compact(
        &model,
        &estimator,
        &kernel_history,
        threshold,
        CompactionMode::Auto,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            return (
                plain_history(history),
                ContextNotice {
                    note: Some(format!(
                        "这一轮的历史（估算 {estimated} tokens）超过了压缩阈值 {threshold}，\
                         但自动压缩没能完成：{e}。已按未压缩的历史继续本轮；\
                         下一步：检查模型端点是否可用，或把 QUILL_LLM_COMPACTION_THRESHOLD_TOKENS 调大。"
                    )),
                    ..base
                },
            );
        }
    };

    match outcome {
        // 未超阈值：内核直接原样返回、一次模型都没调。这里也照原样走。
        CompactionOutcome::Unchanged { .. } => (plain_history(history), base),
        CompactionOutcome::Compacted(result) => {
            let mut messages = vec![
                // 摘要与续跑提示的角色由内核定（user / assistant，照 goose）。
                Message::user(result.summary.text.clone()),
                Message::assistant(result.continuation.text.clone()),
            ];
            if let Some(preserved) = &result.preserved_user {
                messages.push(Message::user(preserved.text.clone()));
            }
            (
                messages,
                ContextNotice {
                    compacted: true,
                    after_tokens: result.accounting.after_tokens,
                    summary_tokens: result.accounting.summary_tokens,
                    ..base
                },
            )
        }
    }
}

/// 上游错误一律映射成 [`CompactionError::Model`]（**不猜**「是不是上下文超了」）。
fn provider_error_is_not_guessed(e: ProviderError) -> CompactionError {
    CompactionError::Model(e.message().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_four_chars_per_token_and_cjk_is_one() {
        let est = CharHeuristic;
        assert_eq!(est.count_text_tokens("abcd"), 1);
        assert_eq!(est.count_text_tokens("abcdefgh"), 2);
        // 中文按字计，不按 4 字 1 个 —— 否则中文历史会被低报 4 倍。
        assert_eq!(est.count_text_tokens("你好世界"), 4);
        // 混排：4 个 ASCII 算 1，2 个汉字算 2。
        assert_eq!(est.count_text_tokens("abcd你好"), 3);
    }

    #[test]
    fn a_threshold_below_the_estimate_is_what_triggers_compaction() {
        // 纯函数边界：内核的严格大于在这里被引用一次，防止调用方自己发明比较规则。
        let est = CharHeuristic;
        let text = "x".repeat(400); // 100 tokens
        let tokens = est.count_text_tokens(&text);
        assert_eq!(tokens, 100);
        assert!(!quill_core::compaction::should_compact_over_tokens(
            tokens, tokens
        ));
        assert!(quill_core::compaction::should_compact_over_tokens(
            tokens,
            tokens - 1
        ));
    }

    #[test]
    fn plain_history_drops_empty_assistant_and_keeps_order() {
        let history = vec![
            ("user".to_string(), "你好".to_string()),
            ("assistant".to_string(), "   ".to_string()),
            ("assistant".to_string(), "在".to_string()),
            ("user".to_string(), "继续".to_string()),
        ];
        let msgs = plain_history(&history);
        assert_eq!(msgs.len(), 3, "空白助手消息不该带着噪音进上下文");
        assert_eq!(msgs[0].role, quill_provider::Role::User);
        assert_eq!(msgs[1].role, quill_provider::Role::Assistant);
        assert_eq!(msgs[2].role, quill_provider::Role::User);
    }

    #[test]
    fn a_provider_failure_is_reported_as_a_model_error_not_guessed() {
        // 上游回了个 400：不许猜成「上下文超了」（那会让内核去走删工具响应的阶梯）。
        let err = provider_error_is_not_guessed(ProviderError::Status {
            code: 400,
            body: "context length exceeded".to_string(),
        });
        match err {
            CompactionError::Model(message) => {
                assert!(message.contains("400|context") || !message.is_empty())
            }
            other => panic!("必须归到 Model，实际：{other:?}"),
        }
    }
}
