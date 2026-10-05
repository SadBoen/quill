use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::ProviderError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    Text {
        text: String,
    },

    ToolCalls {
        calls: Vec<ToolCall>,
    },

    ToolResult {
        tool_call_id: String,

        name: String,

        content: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,

    pub content: MessageContent,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self::plain(Role::System, text)
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::plain(Role::User, text)
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::plain(Role::Assistant, text)
    }

    pub fn assistant_tool_calls(calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: MessageContent::ToolCalls { calls },
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: MessageContent::ToolResult {
                tool_call_id: tool_call_id.into(),
                name: name.into(),
                content: content.into(),
            },
        }
    }

    fn plain(role: Role, text: impl Into<String>) -> Self {
        Self {
            role,
            content: MessageContent::Text { text: text.into() },
        }
    }

    pub fn text(&self) -> Option<&str> {
        match &self.content {
            MessageContent::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }

    pub fn tool_calls(&self) -> &[ToolCall] {
        match &self.content {
            MessageContent::ToolCalls { calls } => calls.as_slice(),
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,

    pub name: String,

    pub arguments: Value,
}

impl ToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,

    pub description: String,

    pub parameters: Value,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    pub fn with_parameters(mut self, parameters: Value) -> Self {
        self.parameters = parameters;
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: Option<u32>,

    pub output: Option<u32>,
}

impl TokenUsage {
    pub fn new(input: Option<u32>, output: Option<u32>) -> Self {
        Self { input, output }
    }

    pub fn total(&self) -> Option<u32> {
        match (self.input, self.output) {
            (Some(i), Some(o)) => Some(i.saturating_add(o)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,

    Other(String),

    Unknown,
}

impl FinishReason {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "" => Self::Unknown,
            "stop" => Self::Stop,
            "length" => Self::Length,
            "tool_calls" | "function_call" => Self::ToolCalls,
            "content_filter" => Self::ContentFilter,
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    pub id: Option<String>,

    pub model: String,

    pub text: String,

    /// 推理模型的思考过程。`text` 为空而这里非空，说明 token 预算被思考吃光了。
    pub reasoning: String,

    pub tool_calls: Vec<ToolCall>,

    pub finish_reason: Option<FinishReason>,

    pub usage: TokenUsage,
}

impl ChatResponse {
    /// 真正能拿去展示或存档的正文。
    /// 思考内容不是回答，绝不能拿它冒充答案。
    pub fn answer(&self) -> &str {
        self.text.trim()
    }

    pub fn has_answer(&self) -> bool {
        !self.answer().is_empty() || !self.tool_calls.is_empty()
    }

    /// 正文空、但模型确实在思考 —— 几乎总是 max_tokens 被思考过程吃光。
    pub fn truncated_by_reasoning(&self) -> bool {
        self.text.trim().is_empty()
            && !self.reasoning.trim().is_empty()
            && self.tool_calls.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,

    pub owned_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,

    pub messages: Vec<Message>,

    pub temperature: Option<f64>,

    pub max_tokens: Option<u32>,

    pub stop: Vec<String>,

    pub tools: Vec<ToolSpec>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            temperature: None,
            max_tokens: None,
            stop: Vec::new(),
            tools: Vec::new(),
        }
    }

    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_stop(mut self, stop: impl IntoIterator<Item = String>) -> Self {
        self.stop = stop.into_iter().collect();
        self
    }

    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = tools;
        self
    }

    /// An empty `model` is allowed: the provider then falls back to the model it was built with.
    pub fn validate(&self) -> Result<(), ProviderError> {
        if self.messages.is_empty() {
            return Err(ProviderError::InvalidRequest {
                detail: "messages 一条都没有，模型没有可读的内容".into(),
            });
        }
        if let Some(t) = self.temperature {
            if !(0.0..=2.0).contains(&t) || t.is_nan() {
                return Err(ProviderError::InvalidRequest {
                    detail: format!("temperature={t} 越界，合法区间是 0.0..=2.0"),
                });
            }
        }
        if self.max_tokens == Some(0) {
            return Err(ProviderError::InvalidRequest {
                detail: "max_tokens=0 会让模型一个字都吐不出来".into(),
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for tool in &self.tools {
            if tool.name.trim().is_empty() {
                return Err(ProviderError::InvalidRequest {
                    detail: "有工具的名字是空串，服务端无法路由这个调用".into(),
                });
            }
            if !seen.insert(tool.name.as_str()) {
                return Err(ProviderError::InvalidRequest {
                    detail: format!("工具名「{}」重复了", tool.name),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StreamSummary {
    pub id: Option<String>,

    pub model: Option<String>,

    pub finish_reason: Option<FinishReason>,

    pub usage: TokenUsage,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamDelta {
    Text(String),

    /// 推理模型的思考增量。它不是答案，展示时要与 `Text` 区分开。
    Reasoning(String),

    ToolCall(ToolCall),

    Done(StreamSummary),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_request() -> ChatRequest {
        ChatRequest::new("m", vec![Message::user("hi")])
    }

    #[test]
    fn a_plain_request_validates() {
        assert!(ok_request().validate().is_ok());
    }

    #[test]
    fn an_empty_conversation_is_rejected() {
        let err = ChatRequest::new("m", Vec::new())
            .validate()
            .expect_err("空 messages 必须被拒");
        assert!(matches!(err, ProviderError::InvalidRequest { .. }));
        assert!(err.to_string().contains("messages"));
    }

    #[test]
    fn an_empty_model_is_allowed_so_the_provider_can_supply_one() {
        assert!(ChatRequest::new("", vec![Message::user("hi")])
            .validate()
            .is_ok());
    }

    #[test]
    fn temperature_outside_the_openai_range_is_rejected() {
        for t in [-0.1, 2.1, f64::NAN] {
            let err = ok_request().with_temperature(t).validate();
            assert!(err.is_err(), "temperature={t} 应当被拒");
        }
        assert!(ok_request().with_temperature(0.0).validate().is_ok());
        assert!(ok_request().with_temperature(2.0).validate().is_ok());
    }

    #[test]
    fn zero_max_tokens_is_rejected() {
        let err = ok_request()
            .with_max_tokens(0)
            .validate()
            .expect_err("max_tokens=0 必须被拒");
        assert!(err.to_string().contains("max_tokens"));
    }

    #[test]
    fn duplicate_and_blank_tool_names_are_rejected() {
        let dup = ok_request()
            .with_tools(vec![ToolSpec::new("read", "a"), ToolSpec::new("read", "b")])
            .validate();
        assert!(dup.is_err(), "重名工具必须被拒");

        let blank = ok_request()
            .with_tools(vec![ToolSpec::new("  ", "a")])
            .validate();
        assert!(blank.is_err(), "空工具名必须被拒");
    }

    #[test]
    fn a_new_tool_spec_defaults_to_an_empty_object_schema() {
        let spec = ToolSpec::new("read", "读文件");
        assert_eq!(spec.parameters["type"], "object");
        assert!(spec.parameters["properties"].as_object().is_some_and(|p| p.is_empty()));
    }

    #[test]
    fn message_accessors_see_only_their_own_variant() {
        let text = Message::user("你好");
        assert_eq!(text.text(), Some("你好"));
        assert!(text.tool_calls().is_empty());

        let call = ToolCall::new("call_1", "read", json!({ "path": "a.rs" }));
        let msg = Message::assistant_tool_calls(vec![call.clone()]);
        assert!(msg.text().is_none());
        assert_eq!(msg.tool_calls(), &[call]);

        let result = Message::tool_result("call_1", "read", "ok");
        assert_eq!(result.role, Role::Tool);
        assert!(result.text().is_none());
    }

    #[test]
    fn token_usage_total_needs_both_halves() {
        assert_eq!(TokenUsage::new(Some(3), Some(4)).total(), Some(7));
        assert_eq!(TokenUsage::new(Some(3), None).total(), None);
        assert_eq!(TokenUsage::new(None, Some(4)).total(), None);
        assert_eq!(
            TokenUsage::new(Some(u32::MAX), Some(1)).total(),
            Some(u32::MAX),
            "总量溢出要饱和，不能回绕"
        );
    }

    #[test]
    fn finish_reason_parsing_covers_every_wire_form() {
        assert_eq!(FinishReason::parse("stop"), FinishReason::Stop);
        assert_eq!(FinishReason::parse("length"), FinishReason::Length);
        assert_eq!(FinishReason::parse("tool_calls"), FinishReason::ToolCalls);
        assert_eq!(
            FinishReason::parse("function_call"),
            FinishReason::ToolCalls
        );
        assert_eq!(
            FinishReason::parse("content_filter"),
            FinishReason::ContentFilter
        );
        assert_eq!(FinishReason::parse(""), FinishReason::Unknown);
        assert_eq!(
            FinishReason::parse("weird_new_reason"),
            FinishReason::Other("weird_new_reason".into())
        );
    }
}
