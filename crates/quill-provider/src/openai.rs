use std::time::Duration;

use async_stream::try_stream;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::{Client, Response};
use serde_json::Value;

use crate::error::{excerpt, redact_url, ProviderError};
use crate::provider::{BoxFuture, Provider, ProviderStream};
use crate::sse::{SseDecoder, SseEvent};
use crate::types::{ChatRequest, ChatResponse, ModelInfo, StreamDelta, StreamSummary};
use crate::wire::{
    build_request_body, parse_chat_response, parse_models_list, parse_stream_chunk,
    usage_from_value, StreamChunk, ToolCallAssembler,
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct OpenAiCompatible {
    name: String,

    base_url: String,

    model: String,

    api_key: Option<String>,

    client: Client,

    timeout: Duration,
}

impl OpenAiCompatible {
    /// `base_url` must already carry the version prefix, e.g. `http://127.0.0.1:8080/v1`.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Result<Self, ProviderError> {
        let base_url = normalize_base_url(&base_url.into())?;
        let client = Client::builder()
            .build()
            .map_err(|e| ProviderError::NotConfigured {
                detail: format!("HTTP 客户端初始化失败：{e}"),
            })?;
        Ok(Self {
            name: "openai-compatible".to_string(),
            base_url,
            model: model.into(),
            api_key,
            client,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// An empty request model means "use the one this provider was built with".
    pub fn resolve_model<'r>(&'r self, request: &'r ChatRequest) -> &'r str {
        let asked = request.model.trim();
        if asked.is_empty() {
            &self.model
        } else {
            asked
        }
    }

    pub fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    pub fn models_url(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn auth_headers(&self) -> Result<HeaderMap, ProviderError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(key) = &self.api_key {
            let mut value = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| {
                ProviderError::NotConfigured {
                    detail: "API key 含有非 ASCII 字符，没法放进 Authorization 头".into(),
                }
            })?;
            value.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        Ok(headers)
    }

    async fn post_json(&self, url: &str, body: &Value) -> Result<Response, ProviderError> {
        let headers = self.auth_headers()?;
        let response = self
            .client
            .post(url)
            .headers(headers)
            .timeout(self.timeout)
            .json(body)
            .send()
            .await
            .map_err(|e| classify_transport(url, &e))?;
        check_status(response, &self.model).await
    }

    async fn post_value(&self, url: &str, body: &Value) -> Result<Value, ProviderError> {
        let response = self.post_json(url, body).await?;
        read_json(response, url).await
    }

    async fn get_json(&self, url: &str) -> Result<Value, ProviderError> {
        let headers = self.auth_headers()?;
        let response = self
            .client
            .get(url)
            .headers(headers)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| classify_transport(url, &e))?;
        let response = check_status(response, &self.model).await?;
        read_json(response, url).await
    }
}

async fn read_json(response: Response, url: &str) -> Result<Value, ProviderError> {
    let text = response
        .text()
        .await
        .map_err(|e| classify_transport(url, &e))?;
    serde_json::from_str(&text).map_err(|e| ProviderError::MalformedResponse {
        detail: format!("响应体不是合法 JSON：{e}。原文：{}", excerpt(&text)),
    })
}

impl Provider for OpenAiCompatible {
    fn name(&self) -> &str {
        &self.name
    }

    fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
        Box::pin(async move {
            request.validate()?;
            let url = self.chat_url();
            let body = build_request_body(request, self.resolve_model(request), false);
            let value = self.post_value(&url, &body).await?;
            parse_chat_response(&value)
        })
    }

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
        Box::pin(async move {
            request.validate()?;
            let url = self.chat_url();
            let body = build_request_body(request, self.resolve_model(request), true);
            let response = self.post_json(&url, &body).await?;
            Ok(sse_response_to_stream(url, response))
        })
    }

    fn models<'a>(&'a self) -> BoxFuture<'a, Vec<ModelInfo>> {
        Box::pin(async move {
            let url = self.models_url();
            let value = self.get_json(&url).await?;
            parse_models_list(&value)
        })
    }
}

fn normalize_base_url(raw: &str) -> Result<String, ProviderError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ProviderError::NotConfigured {
            detail: "base_url 是空串".into(),
        });
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(ProviderError::NotConfigured {
            detail: format!("base_url「{trimmed}」不是 http:// 或 https:// 开头"),
        });
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

/// Split the valid UTF-8 prefix off a byte buffer, keeping any trailing partial
/// character for the next read.
pub fn split_utf8(bytes: &[u8]) -> (String, Vec<u8>) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), Vec::new()),
        Err(e) if e.error_len().is_none() => {
            let valid = e.valid_up_to();
            let text = String::from_utf8_lossy(&bytes[..valid]).into_owned();
            (text, bytes[valid..].to_vec())
        }
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), Vec::new()),
    }
}

pub fn classify_transport(url: &str, error: &reqwest::Error) -> ProviderError {
    let safe = redact_url(url);
    if error.is_timeout() {
        return ProviderError::Timeout {
            detail: format!("等 {safe} 的响应超过了设定时限"),
        };
    }
    if error.is_connect() {
        return ProviderError::Unreachable {
            url: safe,
            detail: "连接被拒绝，目标端口上没有进程在听".into(),
        };
    }
    if let Some(status) = error.status() {
        return classify_status("", status.as_u16(), &error.to_string());
    }
    ProviderError::Unreachable {
        url: safe,
        detail: excerpt(&error.to_string()),
    }
}

/// Map a non-2xx response onto the taxonomy, keeping the status and a body excerpt.
pub fn classify_status(model: &str, code: u16, body: &str) -> ProviderError {
    let excerpt = excerpt(body);
    if code == 404 || (code == 400 && looks_like_missing_model(&excerpt)) {
        return ProviderError::ModelNotFound {
            model: model.to_string(),
            detail: excerpt,
        };
    }
    if code == 408 || code == 504 {
        return ProviderError::Timeout {
            detail: format!("HTTP {code}：{excerpt}"),
        };
    }
    ProviderError::Status {
        code,
        body: excerpt,
    }
}

fn looks_like_missing_model(text: &str) -> bool {
    let lower = text.to_lowercase();
    if !lower.contains("model") {
        return false;
    }
    [
        "not found",
        "does not exist",
        "no such model",
        "unknown model",
        "unsupported model",
        "not a valid model",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

async fn check_status(response: Response, model: &str) -> Result<Response, ProviderError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(classify_status(model, status.as_u16(), &body))
}

/// One decoded SSE event's worth of output. `Stop` means the stream is over.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Deltas(Vec<StreamDelta>),

    Stop,
}

/// Turn one decoded SSE event into the deltas it carries. Pure, so the whole
/// streaming state machine is testable without a socket.
pub fn step_event(
    event: &SseEvent,
    tools: &mut ToolCallAssembler,
    summary: &mut StreamSummary,
) -> Result<Step, ProviderError> {
    let SseEvent::Data(payload) = event else {
        return Ok(Step::Stop);
    };
    let Some(chunk) = parse_stream_chunk(payload)? else {
        return Ok(Step::Deltas(Vec::new()));
    };
    let deltas = apply_chunk(&chunk, tools, summary)?;
    Ok(Step::Deltas(deltas))
}

fn apply_chunk(
    chunk: &StreamChunk,
    tools: &mut ToolCallAssembler,
    summary: &mut StreamSummary,
) -> Result<Vec<StreamDelta>, ProviderError> {
    if let Some(id) = &chunk.id {
        summary.id = Some(id.clone());
    }
    if let Some(model) = &chunk.model {
        summary.model = Some(model.clone());
    }
    if let Some(usage) = &chunk.usage {
        let parsed = usage_from_value(usage);
        if parsed != Default::default() {
            summary.usage = parsed;
        }
    }

    let mut deltas = Vec::new();
    for (i, choice) in chunk.choices.iter().enumerate() {
        if let Some(r) = choice
            .delta
            .reasoning_content
            .as_deref()
            .filter(|t| !t.is_empty())
        {
            deltas.push(StreamDelta::Reasoning(r.to_string()));
        }
        if let Some(text) = choice.delta.content.as_deref().filter(|t| !t.is_empty()) {
            deltas.push(StreamDelta::Text(text.to_string()));
        }
        if !choice.delta.tool_calls.is_empty() {
            tools.push(&choice.delta.tool_calls, i);
        }
        if let Some(raw) = &choice.finish_reason {
            let reason = crate::types::FinishReason::parse(raw);
            summary.finish_reason = Some(reason.clone());
            if matches!(reason, crate::types::FinishReason::ToolCalls) {
                for call in tools.take()? {
                    deltas.push(StreamDelta::ToolCall(call));
                }
            }
        }
    }
    Ok(deltas)
}

fn sse_response_to_stream(url: String, response: Response) -> ProviderStream {
    Box::pin(try_stream! {
        let mut bytes = response.bytes_stream();
        let mut decoder = SseDecoder::new();
        let mut tools = ToolCallAssembler::new();
        let mut summary = StreamSummary::default();
        let mut leftover: Vec<u8> = Vec::new();
        let mut saw_stop = false;

        'outer: while let Some(next) = bytes.next().await {
            let next = next.map_err(|e| classify_transport(&url, &e))?;
            leftover.extend_from_slice(&next);
            let (text, rest) = split_utf8(&leftover);
            leftover = rest;
            for event in decoder.push(&text)? {
                match step_event(&event, &mut tools, &mut summary)? {
                    Step::Deltas(ds) => for d in ds { yield d; },
                    Step::Stop => { saw_stop = true; break 'outer; }
                }
            }
        }

        if !saw_stop {
            for event in decoder.push(&String::from_utf8_lossy(&leftover))? {
                match step_event(&event, &mut tools, &mut summary)? {
                    Step::Deltas(ds) => for d in ds { yield d; },
                    Step::Stop => break,
                }
            }
            for event in decoder.finish()? {
                match step_event(&event, &mut tools, &mut summary)? {
                    Step::Deltas(ds) => for d in ds { yield d; },
                    Step::Stop => break,
                }
            }
        }

        for call in tools.take()? {
            yield StreamDelta::ToolCall(call);
        }
        yield StreamDelta::Done(summary);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FinishReason;
    use crate::wire::{tool_call_from_parts, FunctionDelta, ToolCallDelta};

    #[test]
    fn a_base_url_is_trimmed_and_slashes_are_dropped() {
        let p = OpenAiCompatible::new("  http://127.0.0.1:8080/v1/  ", "m", None)
            .expect("合法 base_url 必须能构造");
        assert_eq!(p.base_url(), "http://127.0.0.1:8080/v1");
        assert_eq!(p.chat_url(), "http://127.0.0.1:8080/v1/chat/completions");
        assert_eq!(p.models_url(), "http://127.0.0.1:8080/v1/models");
    }

    #[test]
    fn a_non_http_base_url_is_a_not_configured_error() {
        for bad in ["", "   ", "127.0.0.1:8080", "ftp://x"] {
            let err = OpenAiCompatible::new(bad, "m", None).expect_err("非 http base_url 必须被拒");
            assert!(matches!(err, ProviderError::NotConfigured { .. }), "{bad}");
        }
    }

    #[test]
    fn an_empty_request_model_falls_back_to_the_configured_one() {
        let p = OpenAiCompatible::new("http://h/v1", "local-model", None).expect("能构造");
        let empty = ChatRequest::new("", vec![crate::types::Message::user("hi")]);
        assert_eq!(p.resolve_model(&empty), "local-model");

        let named = ChatRequest::new("other", vec![crate::types::Message::user("hi")]);
        assert_eq!(p.resolve_model(&named), "other");
    }

    #[test]
    fn a_404_becomes_model_not_found() {
        let err = classify_status(
            "qwen2.5-3b",
            404,
            r#"{"error":{"message":"no such endpoint"}}"#,
        );
        assert!(matches!(err, ProviderError::ModelNotFound { model, .. } if model == "qwen2.5-3b"));
    }

    #[test]
    fn a_400_that_mentions_a_missing_model_becomes_model_not_found() {
        let err = classify_status("qwen2.5-3b", 400, "model 'qwen2.5-3b' does not exist");
        assert!(matches!(err, ProviderError::ModelNotFound { .. }));
    }

    #[test]
    fn a_400_about_something_else_stays_a_plain_status() {
        let err = classify_status(
            "m",
            400,
            r#"{"error":{"message":"'messages' is required"}}"#,
        );
        assert!(matches!(err, ProviderError::Status { code: 400, .. }));
    }

    #[test]
    fn gateway_timeouts_are_classified_as_timeouts() {
        for code in [408, 504] {
            let err = classify_status("m", code, "gateway timeout");
            assert!(matches!(err, ProviderError::Timeout { .. }), "HTTP {code}");
        }
    }

    #[test]
    fn server_errors_keep_their_status_and_body() {
        let err = classify_status("m", 503, "  upstream\n  is  down  ");
        match err {
            ProviderError::Status { code, body } => {
                assert_eq!(code, 503);
                assert_eq!(body, "upstream is down", "正文要压成单行");
            }
            other => panic!("应为 Status，实际 {other:?}"),
        }
    }

    #[test]
    fn an_error_body_is_truncated_rather_than_pasted_whole() {
        let body = "x".repeat(crate::error::MAX_BODY_CHARS + 100);
        let err = classify_status("m", 500, &body);
        match err {
            ProviderError::Status { body, .. } => {
                assert!(body.chars().count() <= crate::error::MAX_BODY_CHARS + 1);
            }
            other => panic!("应为 Status，实际 {other:?}"),
        }
    }

    #[test]
    fn split_utf8_keeps_a_trailing_partial_character_for_the_next_read() {
        let full = "你".as_bytes();
        let (head, rest) = split_utf8(&full[..1]);
        assert_eq!(head, "");
        assert_eq!(rest, full[..1].to_vec(), "半个 UTF-8 字符必须留到下一轮");

        let (head, rest) = split_utf8(b"ok\xe4\xbd");
        assert_eq!(head, "ok");
        assert_eq!(rest, vec![0xe4, 0xbd]);

        let (head, rest) = split_utf8("你好".as_bytes());
        assert_eq!(head, "你好");
        assert!(rest.is_empty());
    }

    #[test]
    fn a_text_only_stream_emits_text_then_done() {
        let mut tools = ToolCallAssembler::new();
        let mut summary = StreamSummary::default();
        let mut deltas = Vec::new();
        for payload in [
            r#"{"id":"c1","model":"m","choices":[{"delta":{"role":"assistant"}}]}"#,
            r#"{"id":"c1","model":"m","choices":[{"delta":{"content":"你"}}]}"#,
            r#"{"id":"c1","model":"m","choices":[{"delta":{"content":"好"}}]}"#,
            r#"{"id":"c1","model":"m","choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ] {
            let step = step_event(&SseEvent::Data(payload.into()), &mut tools, &mut summary)
                .expect("合法帧不应报错");
            match step {
                Step::Deltas(ds) => deltas.extend(ds),
                Step::Stop => panic!("这一帧不是 DONE"),
            }
        }
        deltas.push(StreamDelta::Done(summary.clone()));

        assert_eq!(
            deltas,
            vec![
                StreamDelta::Text("你".into()),
                StreamDelta::Text("好".into()),
                StreamDelta::Done(summary.clone()),
            ]
        );
        assert_eq!(summary.id.as_deref(), Some("c1"));
        assert_eq!(summary.finish_reason, Some(FinishReason::Stop));
    }

    #[test]
    fn a_streamed_tool_call_is_emitted_once_the_finish_reason_arrives() {
        let mut tools = ToolCallAssembler::new();
        let mut summary = StreamSummary::default();
        let mut deltas = Vec::new();
        for payload in [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_0","function":{"name":"read","arguments":"{\"pa"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a.rs\"}"}}]}}]}"#,
        ] {
            match step_event(&SseEvent::Data(payload.into()), &mut tools, &mut summary)
                .expect("合法帧不应报错")
            {
                Step::Deltas(ds) => deltas.extend(ds),
                Step::Stop => panic!("这一帧不是 DONE"),
            }
            assert!(deltas.is_empty(), "工具调用没拼完不能提前发");
        }

        let last = r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":7,"completion_tokens":2}}"#;
        match step_event(&SseEvent::Data(last.into()), &mut tools, &mut summary)
            .expect("合法帧不应报错")
        {
            Step::Deltas(ds) => deltas.extend(ds),
            Step::Stop => panic!("这一帧不是 DONE"),
        }

        assert_eq!(deltas.len(), 1, "只应发一条完整的工具调用");
        match &deltas[0] {
            StreamDelta::ToolCall(call) => {
                assert_eq!(call.id, "call_0");
                assert_eq!(call.name, "read");
                assert_eq!(call.arguments["path"], "a.rs");
            }
            other => panic!("应为 ToolCall，实际 {other:?}"),
        }
        assert_eq!(summary.finish_reason, Some(FinishReason::ToolCalls));
        assert_eq!(summary.usage.input, Some(7));
        assert_eq!(summary.usage.output, Some(2));
    }

    #[test]
    fn a_stream_that_never_says_tool_calls_still_flushes_at_the_end() {
        let mut tools = ToolCallAssembler::new();
        let mut summary = StreamSummary::default();
        let payload = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_0","function":{"name":"read","arguments":"{}"}}]}}]}"#;
        step_event(&SseEvent::Data(payload.into()), &mut tools, &mut summary)
            .expect("合法帧不应报错");
        assert!(!tools.is_empty());
        let flushed = tools.take().expect("收尾拼装必须成功");
        assert_eq!(flushed[0].name, "read");
    }

    #[test]
    fn gateway_metadata_frames_yield_nothing() {
        let mut tools = ToolCallAssembler::new();
        let mut summary = StreamSummary::default();
        match step_event(
            &SseEvent::Data(r#"{"hook_results":{"a":1}}"#.into()),
            &mut tools,
            &mut summary,
        )
        .expect("元数据帧不该报错")
        {
            Step::Deltas(ds) => assert!(ds.is_empty()),
            Step::Stop => panic!("元数据帧不是 DONE"),
        }
    }

    #[test]
    fn the_done_sentinel_stops_the_stream() {
        let mut tools = ToolCallAssembler::new();
        let mut summary = StreamSummary::default();
        let step = step_event(&SseEvent::Done, &mut tools, &mut summary).expect("DONE 不该报错");
        assert!(matches!(step, Step::Stop));
    }

    #[test]
    fn a_malformed_frame_propagates_out_of_the_state_machine() {
        let mut tools = ToolCallAssembler::new();
        let mut summary = StreamSummary::default();
        let err = step_event(&SseEvent::Data("{oops".into()), &mut tools, &mut summary)
            .expect_err("坏帧必须报错");
        assert!(matches!(err, ProviderError::MalformedResponse { .. }));
    }

    #[test]
    fn a_usage_only_frame_updates_the_summary_without_text() {
        let mut tools = ToolCallAssembler::new();
        let mut summary = StreamSummary::default();
        let payload = r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#;
        match step_event(&SseEvent::Data(payload.into()), &mut tools, &mut summary)
            .expect("usage 帧不该报错")
        {
            Step::Deltas(ds) => assert!(ds.is_empty(), "usage 帧不该产生文本增量"),
            Step::Stop => panic!("usage 帧不是 DONE"),
        }
        assert_eq!(summary.usage.total(), Some(7));
    }

    #[test]
    fn a_partial_frame_is_split_by_index_when_the_index_is_absent() {
        let mut tools = ToolCallAssembler::new();
        let mut summary = StreamSummary::default();
        for payload in [
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"a","function":{"name":"one","arguments":"{\"k\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"1}"}}]}}]}"#,
        ] {
            step_event(&SseEvent::Data(payload.into()), &mut tools, &mut summary)
                .expect("合法帧不应报错");
        }
        let calls = tools.take().expect("拼装必须成功");
        assert_eq!(calls[0].arguments["k"], 1);
    }

    #[test]
    fn a_delta_whose_id_arrives_later_still_wins() {
        let mut tools = ToolCallAssembler::new();
        tools.push(
            &[ToolCallDelta {
                index: Some(0),
                id: Some("call_0".into()),
                function: FunctionDelta {
                    name: Some("read".into()),
                    arguments: Some("{}".into()),
                },
            }],
            0,
        );
        tools.push(
            &[ToolCallDelta {
                index: Some(0),
                id: Some(String::new()),
                function: FunctionDelta {
                    name: Some(String::new()),
                    arguments: Some(String::new()),
                },
            }],
            0,
        );
        let calls = tools.take().expect("拼装必须成功");
        assert_eq!(calls[0].id, "call_0", "空的续片不能抹掉已有的 id");
        assert_eq!(calls[0].name, "read", "空的续片不能抹掉已有的名字");
    }

    #[test]
    fn a_stream_built_from_parts_keeps_a_valid_id() {
        let call = tool_call_from_parts("call_x", "read", "{}").expect("合法参数必须成功");
        assert_eq!(call.id, "call_x");
    }
}
