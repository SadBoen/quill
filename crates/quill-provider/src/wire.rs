use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::error::{excerpt, ProviderError};
use crate::types::{ChatRequest, ChatResponse, FinishReason, Message, MessageContent, ToolCall, ToolSpec, TokenUsage};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FunctionDelta {
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolCallDelta {
    #[serde(default)]
    pub index: Option<u32>,

    #[serde(default)]
    pub id: Option<String>,

    #[serde(default)]
    pub function: FunctionDelta,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChoiceDelta {
    #[serde(default)]
    pub content: Option<String>,

    #[serde(default, alias = "reasoning", alias = "reasoning_content")]
    pub reasoning_content: Option<String>,

    #[serde(default)]
    pub tool_calls: Vec<ToolCallDelta>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StreamChoice {
    #[serde(default)]
    pub delta: ChoiceDelta,

    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StreamChunk {
    #[serde(default)]
    pub id: Option<String>,

    #[serde(default)]
    pub model: Option<String>,

    #[serde(default)]
    pub choices: Vec<StreamChoice>,

    #[serde(default)]
    pub usage: Option<Value>,
}

pub fn message_to_wire(message: &Message) -> Value {
    let mut obj = Map::new();
    obj.insert("role".into(), json!(message.role));
    match &message.content {
        MessageContent::Text { text } => {
            obj.insert("content".into(), json!(text));
        }
        MessageContent::ToolCalls { calls } => {
            obj.insert("content".into(), Value::Null);
            obj.insert(
                "tool_calls".into(),
                Value::Array(calls.iter().map(tool_call_to_wire).collect()),
            );
        }
        MessageContent::ToolResult {
            tool_call_id,
            name,
            content,
        } => {
            obj.insert("tool_call_id".into(), json!(tool_call_id));
            obj.insert("name".into(), json!(name));
            obj.insert("content".into(), json!(content));
        }
    }
    Value::Object(obj)
}

pub fn tool_call_to_wire(call: &ToolCall) -> Value {
    let arguments = serde_json::to_string(&call.arguments)
        .unwrap_or_else(|_| "{}".to_string());
    json!({
        "id": call.id,
        "type": "function",
        "function": { "name": call.name, "arguments": arguments },
    })
}

pub fn tool_spec_to_wire(spec: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.description,
            "parameters": spec.parameters,
        },
    })
}

/// Build the `POST /chat/completions` body. `stream` also asks for the usage
/// frame, which is the only place an OpenAI-compatible server reports token
/// counts during a stream.
pub fn build_request_body(request: &ChatRequest, model: &str, stream: bool) -> Value {
    let mut obj = Map::new();
    obj.insert("model".into(), json!(model));
    obj.insert(
        "messages".into(),
        Value::Array(request.messages.iter().map(message_to_wire).collect()),
    );
    if let Some(t) = request.temperature {
        obj.insert("temperature".into(), json!(t));
    }
    if let Some(m) = request.max_tokens {
        obj.insert("max_tokens".into(), json!(m));
    }
    if !request.stop.is_empty() {
        obj.insert("stop".into(), json!(request.stop));
    }
    if !request.tools.is_empty() {
        obj.insert(
            "tools".into(),
            Value::Array(request.tools.iter().map(tool_spec_to_wire).collect()),
        );
    }
    if stream {
        obj.insert("stream".into(), Value::Bool(true));
        obj.insert("stream_options".into(), json!({ "include_usage": true }));
    }
    Value::Object(obj)
}

/// A 2xx body that reports a failure instead of a completion.
pub fn upstream_error(value: &Value) -> Option<ProviderError> {
    let from_error = value.get("error").and_then(|e| {
        let text = match e {
            Value::String(s) => Some(s.clone()),
            other => other
                .get("message")
                .or_else(|| other.get("msg"))
                .and_then(Value::as_str)
                .map(String::from)
                .or_else(|| Some(other.to_string())),
        };
        text.map(|detail| ProviderError::UpstreamRejected { detail })
    });
    let from_object = (value.get("object").and_then(Value::as_str) == Some("error"))
        .then(|| ProviderError::UpstreamRejected {
            detail: value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("服务在流中报了错但没给原因")
                .to_string(),
        });
    from_error.or(from_object)
}

pub fn parse_chat_response(value: &Value) -> Result<ChatResponse, ProviderError> {
    if let Some(err) = upstream_error(value) {
        return Err(err);
    }
    let choice = value
        .pointer("/choices/0")
        .filter(|c| c.is_object())
        .ok_or_else(|| ProviderError::MalformedResponse {
            detail: "响应里没有 choices[0]，拿不到模型输出".into(),
        })?;
    let message = choice
        .get("message")
        .filter(|m| m.is_object())
        .ok_or_else(|| ProviderError::MalformedResponse {
            detail: "choices[0] 里没有 message 字段".into(),
        })?;

    let (text, inline_think) = split_think_block(
        message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let reasoning = match read_reasoning(message) {
        r if !r.trim().is_empty() => r,
        _ => inline_think,
    };
    let tool_calls = parse_tool_calls(message.get("tool_calls"))?;

    Ok(ChatResponse {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .map(String::from),
        model: value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        text,
        reasoning,
        tool_calls,
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(FinishReason::parse),
        usage: extract_usage(value),
    })
}

/// 不同推理模型把思考放在 `reasoning_content` 或 `reasoning` 里，
/// 有的还把它塞进 `content` 内部的 `<think>` 块。
pub fn read_reasoning(message: &Value) -> String {
    for key in ["reasoning_content", "reasoning"] {
        if let Some(s) = message.get(key).and_then(Value::as_str) {
            if !s.trim().is_empty() {
                return s.to_string();
            }
        }
    }
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        if let Some(inner) = strip_think_block(content) {
            return inner;
        }
    }
    String::new()
}

/// 有些模型把思考混在 `content` 里。拆成（正文, 思考），
/// 思考块必须从正文里摘掉 —— 否则界面会把内部推演当成回答显示出来。
fn split_think_block(content: &str) -> (String, String) {
    let Some(start) = content.find("<think>") else {
        return (content.to_string(), String::new());
    };
    let after = &content[start + "<think>".len()..];
    let end = after.find("</think>");
    let (inner, rest) = match end {
        Some(e) => (after[..e].to_string(), after[e + "</think>".len()..].to_string()),
        None => (after.to_string(), String::new()),
    };
    let mut text = String::with_capacity(content.len());
    text.push_str(&content[..start]);
    text.push_str(&rest);
    (text, inner)
}

fn strip_think_block(content: &str) -> Option<String> {
    let (_, inner) = split_think_block(content);
    if inner.trim().is_empty() {
        None
    } else {
        Some(inner)
    }
}

/// Read token counts out of a `usage` object. Tolerates the double-wrapped
/// `{"usage":{"usage":{…}}}` shape some gateways emit and servers that only
/// report one half of the pair.
///
/// 缓存两项按两种真实形状都读，因为两种都在真机上出现过：
///   - OpenAI 系：`prompt_tokens_details.cached_tokens`（vLLM / llama.cpp / OpenRouter）
///   - Anthropic 系：`cache_read_input_tokens` / `cache_creation_input_tokens` 顶层字段
///
/// 两者都**不存在**时返回 `None`（不知道），不是 `Some(0)`（真的没缓存）。
/// 这个区分直接决定统计条敢不敢显示「命中率」。
pub fn usage_from_value(usage: &Value) -> TokenUsage {
    let nested = usage.get("usage").filter(|n| n.is_object());
    let usage = nested.unwrap_or(usage);
    if !usage.is_object() {
        return TokenUsage::default();
    }
    let read = |key: &str| {
        usage
            .get(key)
            .and_then(Value::as_u64)
            .map(|v| u32::try_from(v).unwrap_or(u32::MAX))
    };
    // OpenAI 形状嵌在 details 里；Anthropic 形状直接挂顶层。
    let openai_cached = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(Value::as_u64)
        .map(|v| u32::try_from(v).unwrap_or(u32::MAX));
    let cache_read = openai_cached.or_else(|| read("cache_read_input_tokens"));
    let cache_write = read("cache_write_input_tokens").or_else(|| read("cache_creation_input_tokens"));

    TokenUsage::new(read("prompt_tokens"), read("completion_tokens"))
        .with_cache(cache_read, cache_write)
}

pub fn extract_usage(response: &Value) -> TokenUsage {
    match response.get("usage") {
        Some(usage) if usage.is_object() => usage_from_value(usage),
        _ => TokenUsage::default(),
    }
}

pub fn parse_tool_calls(raw: Option<&Value>) -> Result<Vec<ToolCall>, ProviderError> {
    let Some(raw) = raw.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let items = raw.as_array().ok_or_else(|| ProviderError::MalformedResponse {
        detail: format!("tool_calls 应该是数组，实际是 {}", kind_of(raw)),
    })?;
    items.iter().map(tool_call_from_wire).collect()
}

pub fn tool_call_from_wire(raw: &Value) -> Result<ToolCall, ProviderError> {
    let name = raw
        .pointer("/function/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.trim().is_empty() {
        return Err(ProviderError::MalformedResponse {
            detail: "有一个 tool_call 没有 function.name，无法路由这个调用".into(),
        });
    }
    let id = raw.get("id").and_then(Value::as_str).unwrap_or_default();
    let arguments = raw
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let index = raw.get("index").and_then(Value::as_u64).unwrap_or(0);
    let id = if id.is_empty() {
        synthetic_id(index)
    } else {
        id.to_string()
    };
    tool_call_from_parts(&id, name, arguments)
}

pub fn tool_call_from_parts(
    id: &str,
    name: &str,
    arguments: &str,
) -> Result<ToolCall, ProviderError> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return Ok(ToolCall::new(id, name, json!({})));
    }
    let parsed: Value = serde_json::from_str(trimmed).map_err(|e| {
        ProviderError::MalformedResponse {
            detail: format!("工具「{name}」的参数不是合法 JSON：{e}。原文：{}", excerpt(trimmed)),
        }
    })?;
    if !parsed.is_object() {
        return Err(ProviderError::MalformedResponse {
            detail: format!(
                "工具「{name}」的参数必须是 JSON 对象，实际是 {}",
                kind_of(&parsed)
            ),
        });
    }
    Ok(ToolCall::new(id, name, parsed))
}

fn synthetic_id(index: u64) -> String {
    format!("call_{index}")
}

pub fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "布尔值",
        Value::Number(_) => "数字",
        Value::String(_) => "字符串",
        Value::Array(_) => "数组",
        Value::Object(_) => "对象",
    }
}

/// Reassembles tool calls that arrive as a stream of partial argument fragments
/// keyed by `index`. The first fragment carries `id` and `name`; later ones
/// only append to `arguments`.
#[derive(Debug, Default)]
pub struct ToolCallAssembler {
    slots: BTreeMap<u32, PartialCall>,
}

#[derive(Debug, Default)]
struct PartialCall {
    id: String,

    name: String,

    arguments: String,
}

impl ToolCallAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn push(&mut self, deltas: &[ToolCallDelta], first_index: usize) {
        for (offset, delta) in deltas.iter().enumerate() {
            let index = delta
                .index
                .map(|i| i as usize)
                .unwrap_or(first_index + offset);
            let slot = self.slots.entry(index as u32).or_default();
            if let Some(id) = delta.id.as_deref().filter(|s| !s.is_empty()) {
                slot.id = id.to_string();
            }
            if let Some(name) = delta.function.name.as_deref().filter(|s| !s.is_empty()) {
                slot.name = name.to_string();
            }
            if let Some(args) = delta.function.arguments.as_deref() {
                slot.arguments.push_str(args);
            }
        }
    }

    /// Drain every accumulated call in `index` order, parsing the joined arguments.
    pub fn take(&mut self) -> Result<Vec<ToolCall>, ProviderError> {
        let slots = std::mem::take(&mut self.slots);
        slots
            .into_iter()
            .map(|(index, slot)| {
                let id = if slot.id.is_empty() {
                    synthetic_id(u64::from(index))
                } else {
                    slot.id
                };
                tool_call_from_parts(&id, &slot.name, &slot.arguments)
            })
            .collect()
    }
}

/// Parse one SSE payload. `Ok(None)` means "gateway metadata, nothing to consume".
pub fn parse_stream_chunk(payload: &str) -> Result<Option<StreamChunk>, ProviderError> {
    let value: Value = serde_json::from_str(payload).map_err(|e| ProviderError::MalformedResponse {
        detail: format!("流里的一帧不是合法 JSON：{e}。原文：{}", excerpt(payload)),
    })?;

    if let Some(err) = upstream_error(&value) {
        return Err(err);
    }
    if !value.is_object() {
        return Err(ProviderError::MalformedResponse {
            detail: format!(
                "流里的一帧应该是 JSON 对象，实际是 {}。原文：{}",
                kind_of(&value),
                excerpt(payload)
            ),
        });
    }
    if !value.as_object().is_some_and(|o| o.contains_key("choices")) {
        return Ok(None);
    }

    serde_json::from_value(value).map(Some).map_err(|e| {
        ProviderError::MalformedResponse {
            detail: format!("流里的一帧结构不认识：{e}"),
        }
    })
}

pub fn parse_models_list(value: &Value) -> Result<Vec<crate::types::ModelInfo>, ProviderError> {
    if let Some(err) = upstream_error(value) {
        return Err(err);
    }
    let data = value.get("data").and_then(Value::as_array).ok_or_else(|| {
        ProviderError::MalformedResponse {
            detail: "models 响应里没有 data 数组".into(),
        }
    })?;
    Ok(data
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(Value::as_str)?;
            Some(crate::types::ModelInfo {
                id: id.to_string(),
                owned_by: m
                    .get("owned_by")
                    .and_then(Value::as_str)
                    .map(String::from),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_message_becomes_role_plus_content() {
        assert_eq!(
            message_to_wire(&Message::user("你好")),
            json!({ "role": "user", "content": "你好" })
        );
        assert_eq!(
            message_to_wire(&Message::system("规则")),
            json!({ "role": "system", "content": "规则" })
        );
    }

    #[test]
    fn an_assistant_tool_call_message_keeps_the_null_content_openai_expects() {
        let msg = Message::assistant_tool_calls(vec![ToolCall::new("c1", "read", json!({"p":1}))]);
        assert_eq!(
            message_to_wire(&msg),
            json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "c1",
                    "type": "function",
                    "function": { "name": "read", "arguments": "{\"p\":1}" },
                }],
            })
        );
    }

    #[test]
    fn a_tool_result_message_carries_the_call_id() {
        assert_eq!(
            message_to_wire(&Message::tool_result("c1", "read", "ok")),
            json!({ "role": "tool", "tool_call_id": "c1", "name": "read", "content": "ok" })
        );
    }

    fn sample_request() -> ChatRequest {
        ChatRequest::new("ignored", vec![Message::user("hi")])
    }

    #[test]
    fn the_body_omits_every_unset_optional_field() {
        assert_eq!(
            build_request_body(&sample_request(), "m", false),
            json!({ "model": "m", "messages": [{ "role": "user", "content": "hi" }] })
        );
    }

    #[test]
    fn optional_fields_appear_only_when_set() {
        let req = sample_request()
            .with_temperature(0.3)
            .with_max_tokens(64)
            .with_stop(["\n\n".to_string()])
            .with_tools(vec![ToolSpec::new("read", "读文件")
                .with_parameters(json!({ "type": "object", "properties": { "p": { "type": "string" } } }))]);
        let body = build_request_body(&req, "wire-model", false);
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["max_tokens"], 64);
        assert_eq!(body["stop"], json!(["\n\n"]));
        assert_eq!(body["tools"][0]["function"]["name"], "read");
        assert_eq!(
            body["tools"][0]["function"]["parameters"]["properties"]["p"]["type"],
            "string"
        );
    }

    #[test]
    fn streaming_asks_for_the_usage_frame() {
        let body = build_request_body(&sample_request(), "m", true);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);

        let body = build_request_body(&sample_request(), "m", false);
        assert!(body.get("stream").is_none());
        assert!(body.get("stream_options").is_none());
    }

    #[test]
    fn a_plain_completion_parses_into_a_chat_response() {
        let value = json!({
            "id": "chatcmpl-1",
            "model": "qwen2.5-3b",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "你好！" },
                "finish_reason": "stop",
            }],
            "usage": { "prompt_tokens": 11, "completion_tokens": 5, "total_tokens": 16 },
        });
        let got = parse_chat_response(&value).expect("合法响应必须解析成功");
        assert_eq!(got.id.as_deref(), Some("chatcmpl-1"));
        assert_eq!(got.model, "qwen2.5-3b");
        assert_eq!(got.text, "你好！");
        assert_eq!(got.finish_reason, Some(FinishReason::Stop));
        assert_eq!(got.usage, TokenUsage::new(Some(11), Some(5)));
        assert!(got.tool_calls.is_empty());
    }

    #[test]
    fn a_tool_call_completion_parses_its_arguments() {
        let value = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_a",
                        "type": "function",
                        "function": { "name": "read", "arguments": "{\"path\":\"a.rs\"}" },
                    }],
                },
                "finish_reason": "tool_calls",
            }],
        });
        let got = parse_chat_response(&value).expect("合法响应必须解析成功");
        assert_eq!(got.text, "");
        assert_eq!(got.finish_reason, Some(FinishReason::ToolCalls));
        assert_eq!(got.tool_calls.len(), 1);
        assert_eq!(got.tool_calls[0].name, "read");
        assert_eq!(got.tool_calls[0].arguments["path"], "a.rs");
    }

    #[test]
    fn a_response_without_choices_is_malformed_not_a_panic() {
        let err = parse_chat_response(&json!({ "id": "x" })).expect_err("缺 choices 必须报错");
        assert!(matches!(err, ProviderError::MalformedResponse { .. }));
        assert!(err.to_string().contains("choices"));
    }

    #[test]
    fn a_response_whose_message_is_missing_is_malformed() {
        let err = parse_chat_response(&json!({ "choices": [{ "index": 0 }] }))
            .expect_err("缺 message 必须报错");
        assert!(err.to_string().contains("message"));
    }

    #[test]
    fn an_error_object_in_a_200_response_becomes_upstream_rejected() {
        let err = parse_chat_response(&json!({
            "error": { "message": "context length exceeded", "code": "context_length_exceeded" }
        }))
        .expect_err("error 字段必须转成错误");
        assert!(matches!(err, ProviderError::UpstreamRejected { .. }));
        assert!(err.to_string().contains("context length exceeded"));
    }

    #[test]
    fn a_bare_error_string_is_also_upstream_rejected() {
        let err = parse_chat_response(&json!({ "error": "boom" })).expect_err("必须报错");
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn object_tagged_error_frames_are_caught_too() {
        let err = upstream_error(&json!({ "object": "error", "message": "load failed" }))
            .expect("object=error 必须识别");
        assert!(err.to_string().contains("load failed"));
    }

    #[test]
    fn usage_extraction_handles_the_shapes_servers_actually_send() {
        assert_eq!(
            usage_from_value(&json!({ "prompt_tokens": 3, "completion_tokens": 4 })),
            TokenUsage::new(Some(3), Some(4))
        );
        assert_eq!(
            usage_from_value(&json!({ "usage": { "prompt_tokens": 8, "completion_tokens": 1 } })),
            TokenUsage::new(Some(8), Some(1)),
            "双层 usage 包装要能拆开"
        );
        assert_eq!(
            usage_from_value(&json!({ "prompt_tokens": 3, "completion_tokens": null })),
            TokenUsage::new(Some(3), None),
            "present-but-null 不能挡住另一半"
        );
        assert_eq!(usage_from_value(&json!({})), TokenUsage::default());
        assert_eq!(usage_from_value(&json!("nope")), TokenUsage::default());
    }

    /// 缓存 token 是这轮迁移的核心：Octop 的统计条要显示「缓存命中」，
    /// 而 quill 原本压根不读这两个数。读不到就只能显示空，不能显示 0。
    #[test]
    fn cache_tokens_are_read_from_both_real_world_shapes() {
        // OpenAI 系：嵌在 prompt_tokens_details 里（vLLM / llama.cpp / OpenRouter）。
        let openai = usage_from_value(&json!({
            "prompt_tokens": 1000,
            "completion_tokens": 20,
            "prompt_tokens_details": { "cached_tokens": 900 },
        }));
        assert_eq!(openai.cache_read, Some(900));
        assert_eq!(openai.cache_write, None, "OpenAI 形状没有缓存写，不许编一个 0");
        assert_eq!(openai.input, Some(1000));
        assert!((openai.cache_read_ratio().expect("应当能算") - 0.9).abs() < 1e-6);

        // Anthropic 系：cache_read_input_tokens / cache_creation_input_tokens 挂顶层。
        let anthropic = usage_from_value(&json!({
            "prompt_tokens": 1000,
            "completion_tokens": 20,
            "cache_read_input_tokens": 700,
            "cache_creation_input_tokens": 300,
        }));
        assert_eq!(anthropic.cache_read, Some(700));
        assert_eq!(anthropic.cache_write, Some(300));
    }

    #[test]
    fn a_missing_cache_field_means_unknown_not_zero() {
        let u = usage_from_value(&json!({ "prompt_tokens": 500, "completion_tokens": 10 }));
        assert_eq!(u.cache_read, None, "没上报就是不知道，绝不能塌成 Some(0)");
        assert_eq!(u.cache_write, None);
        assert_eq!(
            u.cache_read_ratio(),
            None,
            "命中率不知道时不能报 0%，那等于凭空造一个数字"
        );
    }

    #[test]
    fn a_reported_zero_cache_count_stays_zero_and_not_none() {
        let u = usage_from_value(&json!({
            "prompt_tokens": 500,
            "completion_tokens": 10,
            "prompt_tokens_details": { "cached_tokens": 0 },
        }));
        assert_eq!(u.cache_read, Some(0), "上游确实报了 0，这是真实值");
        assert_eq!(u.cache_read_ratio(), Some(0.0));
    }

    #[test]
    fn cache_ratio_is_unknown_when_there_was_no_input() {
        let u = usage_from_value(&json!({ "prompt_tokens_details": { "cached_tokens": 10 } }));
        assert_eq!(u.cache_read, Some(10));
        assert_eq!(u.cache_read_ratio(), None, "分母为 None，不能算比率");
    }

    #[test]
    fn total_still_excludes_the_cache_breakdown() {
        // goose 的口径：cache_read 是 input 的子集。再加一次会算重。
        let u = usage_from_value(&json!({
            "prompt_tokens": 1000,
            "completion_tokens": 20,
            "prompt_tokens_details": { "cached_tokens": 900 },
        }));
        assert_eq!(u.total(), Some(1020));
    }

    #[test]
    fn a_response_without_usage_reports_unknown_usage() {
        let got = parse_chat_response(&json!({
            "choices": [{ "message": { "content": "x" } }]
        }))
        .expect("合法响应必须解析成功");
        assert_eq!(got.usage, TokenUsage::default());
        assert!(got.usage.total().is_none());
    }

    #[test]
    fn tool_call_arguments_assemble_from_fragments() {
        let mut a = ToolCallAssembler::new();
        a.push(
            &[ToolCallDelta {
                index: Some(0),
                id: Some("call_0".into()),
                function: FunctionDelta {
                    name: Some("read".into()),
                    arguments: Some("{\"pa".into()),
                },
            }],
            0,
        );
        a.push(
            &[ToolCallDelta {
                index: Some(0),
                function: FunctionDelta {
                    name: None,
                    arguments: Some("th\":\"a.rs\"}".into()),
                },
                ..Default::default()
            }],
            0,
        );
        let calls = a.take().expect("拼装必须成功");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_0");
        assert_eq!(calls[0].name, "read");
        assert_eq!(calls[0].arguments["path"], "a.rs");
    }

    #[test]
    fn parallel_tool_calls_keep_their_index_order() {
        let mut a = ToolCallAssembler::new();
        a.push(
            &[
                ToolCallDelta {
                    index: Some(1),
                    id: Some("call_1".into()),
                    function: FunctionDelta {
                        name: Some("second".into()),
                        arguments: Some("{}".into()),
                    },
                },
                ToolCallDelta {
                    index: Some(0),
                    id: Some("call_0".into()),
                    function: FunctionDelta {
                        name: Some("first".into()),
                        arguments: Some("{}".into()),
                    },
                },
            ],
            0,
        );
        let calls = a.take().expect("拼装必须成功");
        let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["first", "second"], "必须按 index 排序，不能按到达顺序");
    }

    #[test]
    fn a_delta_without_an_index_uses_its_position_in_the_chunk() {
        let mut a = ToolCallAssembler::new();
        a.push(
            &[ToolCallDelta {
                id: Some("call_0".into()),
                function: FunctionDelta {
                    name: Some("read".into()),
                    arguments: Some("{}".into()),
                },
                ..Default::default()
            }],
            0,
        );
        let calls = a.take().expect("拼装必须成功");
        assert_eq!(calls[0].name, "read");
    }

    #[test]
    fn an_absent_tool_id_gets_a_synthetic_one() {
        let mut a = ToolCallAssembler::new();
        a.push(
            &[ToolCallDelta {
                index: Some(3),
                function: FunctionDelta {
                    name: Some("read".into()),
                    arguments: None,
                },
                ..Default::default()
            }],
            0,
        );
        let calls = a.take().expect("拼装必须成功");
        assert_eq!(calls[0].id, "call_3");
        assert_eq!(calls[0].arguments, json!({}), "没给参数等于空对象");
    }

    #[test]
    fn split_arguments_that_do_not_form_json_are_reported_not_panicked_on() {
        let mut a = ToolCallAssembler::new();
        a.push(
            &[ToolCallDelta {
                index: Some(0),
                id: Some("call_0".into()),
                function: FunctionDelta {
                    name: Some("read".into()),
                    arguments: Some("{\"path\":".into()),
                },
            }],
            0,
        );
        let err = a.take().expect_err("半截 JSON 必须报错");
        assert!(matches!(err, ProviderError::MalformedResponse { .. }));
        assert!(err.to_string().contains("read"));
    }

    #[test]
    fn non_object_tool_arguments_are_rejected() {
        let err = tool_call_from_parts("c", "read", "[1,2]").expect_err("数组参数必须被拒");
        assert!(err.to_string().contains("数组"));
        let err = tool_call_from_parts("c", "read", "7").expect_err("标量参数必须被拒");
        assert!(err.to_string().contains("数字"));
    }

    #[test]
    fn a_tool_call_without_a_name_is_rejected() {
        let err = tool_call_from_wire(&json!({ "id": "c", "function": { "arguments": "{}" } }))
            .expect_err("没有函数名必须被拒");
        assert!(err.to_string().contains("function.name"));
    }

    #[test]
    fn tool_calls_that_are_not_an_array_are_rejected() {
        let err = parse_tool_calls(Some(&json!({ "a": 1 }))).expect_err("非数组必须被拒");
        assert!(err.to_string().contains("对象"));
    }

    #[test]
    fn absent_tool_calls_yield_an_empty_list() {
        assert!(parse_tool_calls(None).expect("None 合法").is_empty());
        assert!(parse_tool_calls(Some(&Value::Null))
            .expect("null 合法")
            .is_empty());
    }

    #[test]
    fn a_well_formed_stream_chunk_parses() {
        let chunk = parse_stream_chunk(
            r#"{"id":"c1","model":"m","choices":[{"delta":{"content":"你"}}]}"#,
        )
        .expect("合法帧必须解析成功")
        .expect("带 choices 的帧不是元数据");
        assert_eq!(chunk.id.as_deref(), Some("c1"));
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("你"));
    }

    #[test]
    fn malformed_json_in_a_frame_is_a_classified_error_not_a_panic() {
        let err = parse_stream_chunk("{\"choices\":").expect_err("半截 JSON 必须报错");
        assert!(matches!(err, ProviderError::MalformedResponse { .. }));
        assert!(err.to_string().contains("合法 JSON"));
    }

    #[test]
    fn a_json_array_payload_is_rejected_while_an_object_is_not() {
        let err = parse_stream_chunk("[{\"choices\":[]}]").expect_err("数组帧必须被拒");
        assert!(matches!(err, ProviderError::MalformedResponse { .. }));
        assert!(err.to_string().contains("数组"), "要点明实际类型：{err}");

        let meta = parse_stream_chunk("{\"hook_results\":{}}")
            .expect("无 choices 的对象是网关元数据，不该报错");
        assert!(meta.is_none(), "无 choices 的对象帧要被跳过");

        let usage = parse_stream_chunk(r#"{"choices":[],"usage":{"prompt_tokens":2}}"#)
            .expect("usage-only 帧合法")
            .expect("choices 为空仍是真实帧");
        assert_eq!(usage.usage.as_ref().expect("usage 帧必须带 usage").get("prompt_tokens"), Some(&json!(2)));
    }

    #[test]
    fn an_error_frame_mid_stream_is_upstream_rejected() {
        let err = parse_stream_chunk(r#"{"error":{"message":"generation aborted"}}"#)
            .expect_err("流中 error 帧必须报错");
        assert!(matches!(err, ProviderError::UpstreamRejected { .. }));
        assert!(err.to_string().contains("generation aborted"));
    }

    #[test]
    fn a_models_list_parses_into_infos() {
        let got = parse_models_list(&json!({
            "object": "list",
            "data": [
                { "id": "qwen2.5-3b", "owned_by": "local" },
                { "id": "nomic-embed" },
                { "nope": 1 },
            ],
        }))
        .expect("合法 models 响应必须解析成功");
        assert_eq!(got.len(), 2, "没有 id 的条目要丢掉");
        assert_eq!(got[0].id, "qwen2.5-3b");
        assert_eq!(got[0].owned_by.as_deref(), Some("local"));
        assert!(got[1].owned_by.is_none());
    }

    #[test]
    fn a_models_list_without_data_is_malformed() {
        let err = parse_models_list(&json!({ "object": "list" })).expect_err("缺 data 必须报错");
        assert!(err.to_string().contains("data"));
    }

    /// 真机上观察到的形状：MiniCPM5-1B 是推理模型，max_tokens 给小了会把
    /// 预算全烧在 reasoning_content 上，content 留空。
    #[test]
    fn reasoning_content_is_kept_apart_from_the_answer() {
        let got = parse_chat_response(&json!({
            "id": "c1",
            "model": "local",
            "choices": [{
                "finish_reason": "length",
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning_content": "嗯，用户问的是 1+1……"
                }
            }],
            "usage": { "prompt_tokens": 22, "completion_tokens": 80 }
        }))
        .expect("推理型响应必须能解析");

        assert_eq!(got.answer(), "", "正文确实是空的");
        assert!(!got.reasoning.is_empty(), "思考内容不能被丢掉");
        assert!(!got.has_answer(), "只有思考没有正文时，不得声称拿到了回答");
        assert!(
            got.truncated_by_reasoning(),
            "必须能识别出「预算被思考吃光」这一情形"
        );
    }

    #[test]
    fn a_normal_answer_from_a_reasoning_model_still_parses() {
        let got = parse_chat_response(&json!({
            "choices": [{
                "finish_reason": "stop",
                "message": { "content": "1+1=2", "reasoning_content": "先算加法" }
            }]
        }))
        .expect("应能解析");

        assert_eq!(got.answer(), "1+1=2");
        assert!(got.has_answer());
        assert!(!got.truncated_by_reasoning(), "有正文就不算被思考截断");
        assert_eq!(got.reasoning, "先算加法");
    }

    #[test]
    fn the_reasoning_alias_is_also_accepted() {
        let got = parse_chat_response(&json!({
            "choices": [{ "message": { "content": "", "reasoning": "换个字段名也一样" } }]
        }))
        .expect("应能解析");
        assert_eq!(got.reasoning, "换个字段名也一样");
    }

    #[test]
    fn a_think_block_inside_content_becomes_reasoning_not_answer() {
        let got = parse_chat_response(&json!({
            "choices": [{ "message": { "content": "<think>内部推演</think>答案" } }]
        }))
        .expect("应能解析");
        assert_eq!(got.reasoning, "内部推演");
        assert_eq!(got.answer(), "答案");
    }

    #[test]
    fn tool_calls_alone_count_as_having_an_answer() {
        let got = parse_chat_response(&json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "",
                    "reasoning_content": "该查天气",
                    "tool_calls": [{
                        "id": "t1",
                        "type": "function",
                        "function": { "name": "get_weather", "arguments": "{\"city\":\"北京\"}" }
                    }]
                }
            }]
        }))
        .expect("应能解析");
        assert!(got.has_answer(), "发起工具调用就是一种有效产出");
        assert!(!got.truncated_by_reasoning());
    }
}
