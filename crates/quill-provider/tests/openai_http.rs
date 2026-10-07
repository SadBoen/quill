use futures_util::StreamExt;
use quill_provider::{
    ChatRequest, FinishReason, Message, OpenAiCompatible, Provider, ProviderError, StreamDelta,
    TokenUsage,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider_for(server: &MockServer) -> OpenAiCompatible {
    OpenAiCompatible::new(format!("{}/v1", server.uri()), "qwen2.5-3b", None)
        .expect("mock server 的 base_url 必须合法")
}

fn request() -> ChatRequest {
    ChatRequest::new("qwen2.5-3b", vec![Message::user("你好")])
}

#[tokio::test]
async fn chat_returns_the_parsed_completion() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "chatcmpl-1",
            "model": "qwen2.5-3b",
            "choices": [{
                "message": { "role": "assistant", "content": "你好！" },
                "finish_reason": "stop",
            }],
            "usage": { "prompt_tokens": 4, "completion_tokens": 3 },
        })))
        .mount(&server)
        .await;

    let got = provider_for(&server)
        .chat(&request())
        .await
        .expect("合法响应必须成功");

    assert_eq!(got.text, "你好！");
    assert_eq!(got.id.as_deref(), Some("chatcmpl-1"));
    assert_eq!(got.finish_reason, Some(FinishReason::Stop));
    assert_eq!(got.usage, TokenUsage::new(Some(4), Some(3)));
}

#[tokio::test]
async fn chat_sends_the_model_and_the_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{ "message": { "content": "ok" } }]
        })))
        .mount(&server)
        .await;

    let body = provider_for(&server)
        .chat(&request().with_temperature(0.2).with_max_tokens(32))
        .await
        .expect("合法响应必须成功");
    assert_eq!(body.text, "ok");

    let requests = server.received_requests().await.expect("能读到请求记录");
    let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("请求体是 JSON");
    assert_eq!(sent["model"], "qwen2.5-3b");
    assert_eq!(sent["messages"][0]["role"], "user");
    assert_eq!(sent["messages"][0]["content"], "你好");
    assert_eq!(sent["temperature"], 0.2);
    assert_eq!(sent["max_tokens"], 32);
    assert!(sent.get("stream").is_none(), "非流式请求不该带 stream 字段");
}

#[tokio::test]
async fn chat_reports_a_404_as_model_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(r#"{"error":{"message":"no route for this model"}}"#),
        )
        .mount(&server)
        .await;

    let err = provider_for(&server)
        .chat(&request())
        .await
        .expect_err("404 必须报错");
    match err {
        ProviderError::ModelNotFound { model, .. } => assert_eq!(model, "qwen2.5-3b"),
        other => panic!("应为 ModelNotFound，实际 {other:?}"),
    }
}

#[tokio::test]
async fn chat_reports_a_401_with_the_body_attached() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401).set_body_string(r#"{"error":{"message":"bad key"}}"#),
        )
        .mount(&server)
        .await;

    let err = provider_for(&server)
        .chat(&request())
        .await
        .expect_err("401 必须报错");
    match &err {
        ProviderError::Status { code, body } => {
            assert_eq!(*code, 401);
            assert!(body.contains("bad key"), "正文要带上：{body}");
        }
        other => panic!("应为 Status，实际 {other:?}"),
    }
    assert!(!err.is_retryable(), "401 重试没有意义");
}

#[tokio::test]
async fn chat_reports_an_unreachable_server_as_unreachable() {
    // Take a port from the OS and hand it straight back, so nothing is listening.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("能拿到空闲端口");
        listener.local_addr().expect("能读到本地地址")
    };

    let err = OpenAiCompatible::new(format!("http://{closed}/v1"), "m", None)
        .expect("base_url 合法")
        .chat(&request())
        .await
        .expect_err("连不上必须报错");

    assert!(
        matches!(err, ProviderError::Unreachable { .. }),
        "应为 Unreachable，实际 {err:?}"
    );
    assert!(err.is_retryable(), "连不上重试有意义");
    let msg = err.to_string();
    assert!(
        msg.contains("/v1/chat/completions"),
        "要点名打不通的地址：{msg}"
    );
    assert!(msg.contains("llama-server"), "要给出最可能的原因：{msg}");
}

#[tokio::test]
async fn chat_reports_a_malformed_body_as_malformed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&server)
        .await;

    let err = provider_for(&server)
        .chat(&request())
        .await
        .expect_err("非法 JSON 必须报错");
    assert!(
        matches!(err, ProviderError::MalformedResponse { .. }),
        "应为 MalformedResponse，实际 {err:?}"
    );
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn chat_rejects_an_invalid_request_before_any_http_call() {
    let server = MockServer::start().await;
    let err = provider_for(&server)
        .chat(&ChatRequest::new("qwen2.5-3b", Vec::new()))
        .await
        .expect_err("空 messages 必须在本地就被拒");
    assert!(matches!(err, ProviderError::InvalidRequest { .. }));

    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "本地校验失败时不该发请求"
    );
}

#[tokio::test]
async fn stream_yields_text_deltas_then_done() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\n",
        "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n\n",
        "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let mut stream = provider_for(&server)
        .stream(&request())
        .await
        .expect("流式请求必须成功");

    let mut text = String::new();
    let mut summary = None;
    while let Some(item) = stream.next().await {
        match item.expect("每一帧都要能解析") {
            StreamDelta::Text(t) => text.push_str(&t),
            StreamDelta::Reasoning(r) => panic!("这轮不该有思考内容：{r}"),
            StreamDelta::ToolCall(call) => panic!("这轮不该有工具调用：{call:?}"),
            StreamDelta::Done(s) => summary = Some(s),
        }
    }

    assert_eq!(text, "你好");
    let summary = summary.expect("流必须以 Done 收尾");
    assert_eq!(summary.finish_reason, Some(FinishReason::Stop));
    assert_eq!(summary.usage.total(), Some(7));

    let requests = server.received_requests().await.expect("能读到请求记录");
    let sent: serde_json::Value = serde_json::from_slice(&requests[0].body).expect("请求体是 JSON");
    assert_eq!(sent["stream"], true);
    assert_eq!(sent["stream_options"]["include_usage"], true);
}

#[tokio::test]
async fn stream_survives_a_frame_split_across_tcp_reads() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"id\":\"c1\",\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n",
        "\n",
        "data: {\"id\":\"c1\",\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let mut stream = provider_for(&server)
        .stream(&request())
        .await
        .expect("流式请求必须成功");

    let mut text = String::new();
    while let Some(item) = stream.next().await {
        if let StreamDelta::Text(t) = item.expect("每一帧都要能解析") {
            text.push_str(&t);
        }
    }
    assert_eq!(text, "AB", "跨读的帧必须拼回同一事件");
}

#[tokio::test]
async fn stream_reassembles_a_tool_call_split_over_many_frames() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_0\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"pa\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"a.rs\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let mut stream = provider_for(&server)
        .stream(&request())
        .await
        .expect("流式请求必须成功");

    let mut calls = Vec::new();
    while let Some(item) = stream.next().await {
        if let StreamDelta::ToolCall(call) = item.expect("每一帧都要能解析") {
            calls.push(call);
        }
    }
    assert_eq!(calls.len(), 1, "只应拼出一条完整的工具调用");
    assert_eq!(calls[0].id, "call_0");
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].arguments["path"], "a.rs");
}

#[tokio::test]
async fn stream_surfaces_a_malformed_frame_as_an_error_item() {
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n\n",
        "data: {broken\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;

    let mut stream = provider_for(&server)
        .stream(&request())
        .await
        .expect("流式请求本身必须成功");

    let mut saw_text = false;
    let mut failure = None;
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamDelta::Text(_)) => saw_text = true,
            Ok(_) => {}
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    assert!(saw_text, "坏帧之前的正常增量仍要能读到");
    let failure = failure.expect("坏帧必须变成流里的一个错误项");
    assert!(matches!(failure, ProviderError::MalformedResponse { .. }));
}

#[tokio::test]
async fn models_lists_what_the_server_serves() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list",
            "data": [{ "id": "qwen2.5-3b", "owned_by": "llamacpp" }],
        })))
        .mount(&server)
        .await;

    let got = provider_for(&server)
        .models()
        .await
        .expect("合法 models 响应必须成功");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, "qwen2.5-3b");
    assert_eq!(got[0].owned_by.as_deref(), Some("llamacpp"));
}

#[tokio::test]
async fn models_sends_the_api_key_when_one_is_configured() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": [] })))
        .mount(&server)
        .await;

    OpenAiCompatible::new(format!("{}/v1", server.uri()), "m", Some("sk-test".into()))
        .expect("能构造")
        .models()
        .await
        .expect("合法响应必须成功");

    let requests = server.received_requests().await.expect("能读到请求记录");
    let auth = requests[0]
        .headers
        .get("authorization")
        .expect("带 key 时必须有 Authorization 头");
    assert_eq!(auth.to_str().expect("头是 ASCII"), "Bearer sk-test");
}
