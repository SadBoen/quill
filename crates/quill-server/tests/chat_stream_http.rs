//! 流式发消息的 HTTP 端到端证据。
//!
//! 判据不是「返回 200」，而是**逐帧核对**：
//!   - 增量确实是一帧一帧出来的，不是攒完一次性倒出来；
//!   - 工具轮的正文**真的被丢弃**了（界面若把它留下，用户会看到一段
//!     中途变调的答案，而存档里只有最终那段）；
//!   - `done` 的负载与老路由逐字段同形；
//!   - 上游不支持流式时**自动退回一次性**，而不是让聊天彻底不可用。

mod common;

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Context, Poll};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_provider::{
    BoxFuture, ChatRequest, ChatResponse, FinishReason, ModelInfo, Provider, ProviderStream,
    StreamDelta, StreamSummary, TokenUsage, ToolCall,
};
use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN_A: &str = "tok-a";

/// 按顺序吐完就结束的流。
///
/// 自己写而不引 futures-util：dev-dependencies 里没有它，而这里只需要
/// 「从 Vec 里逐个取值」。
struct IterStream<T> {
    items: std::vec::IntoIter<T>,
}

impl<T: Send + Unpin + 'static> futures_core::Stream for IterStream<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<T>> {
        // `IntoIter` 是 `Unpin`，所以 `Self` 也是。
        Poll::Ready(self.get_mut().items.next())
    }
}

fn iter_stream<T: Send + Unpin + 'static>(
    items: Vec<T>,
) -> Pin<Box<dyn futures_core::Stream<Item = T> + Send>> {
    Box::pin(IterStream {
        items: items.into_iter(),
    })
}

/// 第 0 轮：说一句 + 要工具；第 1 轮：给最终答案。
#[derive(Debug, Default)]
struct TwoRoundProvider {
    round: AtomicUsize,
    seen: Mutex<Vec<ChatRequest>>,
}

impl Provider for TwoRoundProvider {
    fn name(&self) -> &str {
        "two-round"
    }

    fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
        self.seen.lock().expect("锁").push(request.clone());
        let n = self.round.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(plain_reply(request, n)) })
    }

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
        self.seen.lock().expect("锁").push(request.clone());
        let n = self.round.fetch_add(1, Ordering::SeqCst);
        let items: Vec<Result<StreamDelta, quill_provider::ProviderError>> = if n == 0 {
            vec![
                Ok(StreamDelta::Text("我先查一下。".into())),
                Ok(StreamDelta::ToolCall(ToolCall {
                    id: "call_1".into(),
                    // 故意叫一个不存在的工具：结果必须是「失败」，
                    // 界面不能被告知成功。
                    name: "no_such_tool".into(),
                    arguments: serde_json::json!({}),
                })),
                Ok(StreamDelta::Done(StreamSummary {
                    finish_reason: Some(FinishReason::ToolCalls),
                    usage: TokenUsage::new(Some(10), Some(3)),
                    ..Default::default()
                })),
            ]
        } else {
            vec![
                Ok(StreamDelta::Text("最终".into())),
                Ok(StreamDelta::Text("答案。".into())),
                Ok(StreamDelta::Done(StreamSummary {
                    finish_reason: Some(FinishReason::Stop),
                    usage: TokenUsage::new(Some(20), Some(5)),
                    ..Default::default()
                })),
            ]
        };
        Box::pin(async move { Ok(iter_stream(items) as ProviderStream) })
    }

    fn models<'a>(&'a self) -> BoxFuture<'a, Vec<ModelInfo>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn plain_reply(request: &ChatRequest, round: usize) -> ChatResponse {
    if round == 0 {
        ChatResponse {
            id: None,
            model: request.model.clone(),
            text: "我先查一下。".into(),
            reasoning: String::new(),
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "no_such_tool".into(),
                arguments: serde_json::json!({}),
            }],
            finish_reason: Some(FinishReason::ToolCalls),
            usage: TokenUsage::new(Some(10), Some(3)),
        }
    } else {
        ChatResponse {
            id: None,
            model: request.model.clone(),
            text: "最终答案。".into(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            finish_reason: Some(FinishReason::Stop),
            usage: TokenUsage::new(Some(20), Some(5)),
        }
    }
}

fn user_id() -> quill_domain::UserId {
    quill_domain::UserId::parse(UID_A).expect("测试 UID 必须合法")
}

fn state(t: &TestDb, provider: Option<Arc<dyn Provider>>) -> AppState {
    let resolver = EnvTokenResolver::new(vec![(
        TOKEN_A.to_string(),
        AuthContext {
            user_id: user_id(),
            is_admin: true,
        },
    )]);
    AppState {
        config: Config::from_env(),
        tokens: Arc::new(resolver),
        db: Some(t.bridge()),
        db_problem: None,
        llm: Arc::new(RwLock::new(provider)),
        llm_config: Arc::new(RwLock::new(Default::default())),
        providers: Arc::new(RwLock::new(Default::default())),
        login_limiter: Arc::new(Default::default()),
        pbkdf2: quill_control::Pbkdf2Params::for_tests(),
    }
}

fn req(method: &str, path: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN_A}"));
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    b.body(match body {
        Some(v) => Body::from(v.to_string()),
        None => Body::empty(),
    })
    .expect("构造请求失败")
}

async fn text(resp: axum::response::Response) -> String {
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("读响应体")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("响应体必须是 UTF-8")
}

fn json(t: &str) -> serde_json::Value {
    serde_json::from_str(t)
        .unwrap_or_else(|e| panic!("响应必须是 JSON（{e}）：{}", t.replace('\n', "\\n")))
}

/// 把 SSE 响应体拆成一串 `(事件名, 负载)`。
///
/// 自己拆而不是用现成库：`event:` / `data:` 两行的规则就这两条，
/// 引入一个解析器反而看不出服务端到底发了什么形状。
fn parse_sse(body: &str) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    let mut event = String::new();
    let mut data = String::new();
    for line in body.split('\n') {
        if line.is_empty() {
            if !event.is_empty() || !data.is_empty() {
                out.push((event.clone(), json(data.trim())));
            }
            event.clear();
            data.clear();
        } else if let Some(v) = line.strip_prefix("event: ") {
            event = v.to_string();
        } else if let Some(v) = line.strip_prefix("data: ") {
            data.push_str(v);
        } else if line.starts_with(':') {
            // keep-alive 注释，忽略。
        } else {
            panic!("SSE 里出现了非字段行：{line:?}（整段：{body}）");
        }
    }
    if !event.is_empty() || !data.is_empty() {
        out.push((event, json(data.trim())));
    }
    out
}

fn seed_user(t: &TestDb) {
    let id = user_id().as_bytes().to_vec();
    t.bridge()
        .call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(
                    "INSERT OR IGNORE INTO users (id, username, username_norm, display_name, \
                     password_hash, password_salt, password_algo, role, pwd_changed_at, \
                     created_at, updated_at) \
                     VALUES (?,?,?,?,zeroblob(32),zeroblob(16),'pbkdf2-hmac-sha256$i=600000',\
                     'owner',0,0,0)",
                )
                .bind(id)
                .bind(UID_A)
                .bind(UID_A)
                .bind("测试用户")
                .execute(&pool)
                .await
                .map_err(|e| quill_server::db::storage_error("测试铺用户", e))?;
                Ok(())
            })
        })
        .expect("铺用户失败");
}

async fn create_session(app: AppState) -> String {
    let resp = build_router(app)
        .oneshot(req(
            "POST",
            "/api/sessions",
            Some(serde_json::json!({ "title": "流式测试" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK, "建会话应成功");
    let v = json(&text(resp).await);
    v["id"].as_str().expect("会话 id 必须是字符串").to_string()
}

fn session_blob(sid: &str) -> Vec<u8> {
    let hex: String = sid.chars().filter(|c| *c != '-').collect();
    assert_eq!(hex.len(), 32, "会话 id 必须是 32 位十六进制：{sid}");
    (0..16)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("会话 id 必须是十六进制"))
        .collect()
}

fn assistant_rows(t: &TestDb, sid: &str) -> Vec<String> {
    let sid = session_blob(sid);
    let uid = user_id().as_bytes().to_vec();
    t.bridge()
        .call(move |pool, _rt| {
            Box::pin(async move {
                let rows = sqlx::query(
                    "SELECT content FROM messages WHERE user_id = ? AND session_id = ? \
                     AND role = 'assistant' ORDER BY seq",
                )
                .bind(uid)
                .bind(sid)
                .fetch_all(&pool)
                .await
                .map_err(|e| quill_server::db::storage_error("读消息", e))?;
                Ok(rows
                    .into_iter()
                    .map(|r| sqlx::Row::get(&r, "content"))
                    .collect::<Vec<String>>())
            })
        })
        .expect("读消息失败")
}

#[tokio::test]
async fn the_stream_carries_every_frame_and_drops_the_tool_rounds_text() {
    let t = TestDb::new("stream-rounds");
    seed_user(&t);
    let provider = Arc::new(TwoRoundProvider::default());
    let app = state(&t, Some(provider.clone()));
    let sid = create_session(app.clone()).await;

    let resp = build_router(app)
        .oneshot(req(
            "POST",
            &format!("/api/sessions/{sid}/messages/stream"),
            Some(serde_json::json!({ "content": "帮我看看" })),
        ))
        .await
        .expect("oneshot 失败");

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream"),
        "不是 SSE 的 content-type，前端的 getReader 分支根本不会进"
    );
    let raw = text(resp).await;
    // 真实响应体必须以「空行」收尾：SSE 靠空行分帧，缺了它最后那一帧在
    // 连接关闭时会被浏览器丢掉，而「done 丢了」正是最难排查的那种故障。
    assert!(
        raw.ends_with("\n\n"),
        "响应体必须以空行收尾：{}",
        raw.replace('\n', "\\n")
    );
    let frames = parse_sse(&raw);
    let names: Vec<&str> = frames.iter().map(|(n, _)| n.as_str()).collect();

    assert_eq!(
        names,
        vec![
            "user_message",
            "delta",   // 我先查一下。
            "discard", // 这一轮的正文会被工具往返覆盖，先抹掉
            "tool_call",
            "tool_result",
            "delta", // 最终
            "delta", // 答案。
            "done",
        ],
        "帧的顺序与内容都要对：{frames:#?}"
    );

    // 增量必须**逐帧**到达，而不是攒成一坨再倒出来。
    let deltas: Vec<String> = frames
        .iter()
        .filter(|(n, _)| n == "delta")
        .map(|(_, d)| d["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(deltas, vec!["我先查一下。", "最终", "答案。"]);

    let discard = frames
        .iter()
        .find(|(n, _)| n == "discard")
        .map(|(_, d)| d.clone())
        .expect("工具轮应当发 discard");
    assert_eq!(
        discard["text"], "我先查一下。",
        "discard 必须带上要抹掉的原文，前端只拿到个数字是抹不掉的：{discard}"
    );

    let tool_result = frames
        .iter()
        .find(|(n, _)| n == "tool_result")
        .map(|(_, d)| d.clone())
        .expect("应当有 tool_result");
    assert_eq!(
        tool_result["ok"], false,
        "工具不存在就是失败，不能报成功：{tool_result}"
    );

    let done = frames
        .last()
        .filter(|(n, _)| n == "done")
        .map(|(_, d)| d.clone())
        .expect("最后一帧必须是 done");
    assert_eq!(done["reply"], "最终答案。");
    assert_eq!(done["tool_rounds"], 1, "工具往返确实发生过：{done}");
    assert_eq!(
        done["tool_calls"][0]["ok"], false,
        "不存在的工具在轨迹里也必须是失败：{done}"
    );
    // 两轮的 usage 都要算进去，不能只报最后一轮。
    assert_eq!(done["usage"]["input"], 30);
    assert_eq!(done["usage"]["output"], 8);

    assert_eq!(
        assistant_rows(&t, &sid),
        vec!["最终答案。".to_string()],
        "存档里只该有最终那段；工具轮那句绝不能落库"
    );
}

/// 断言两份 JSON 的**结构**逐层相同：键集合一样、数组长度一样、叶子类型一样。
///
/// 不能比值 —— id、时间戳、`turn_ms` 每轮都不一样，比值只能得到一个恒假的断言。
/// 这里要证明的是「流式那条路没有少字段、也没有多出字段」。
fn assert_same_shape(a: &serde_json::Value, b: &serde_json::Value, path: &str) {
    match (a, b) {
        (serde_json::Value::Object(x), serde_json::Value::Object(y)) => {
            let missing: Vec<&String> = x.keys().filter(|k| !y.contains_key(*k)).collect();
            let extra: Vec<&String> = y.keys().filter(|k| !x.contains_key(*k)).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{path} 的键集合不一致：流式缺 {missing:?}，老路由多 {extra:?}"
            );
            for (k, v) in x {
                assert_same_shape(v, &y[k], &format!("{path}.{k}"));
            }
        }
        (serde_json::Value::Array(x), serde_json::Value::Array(y)) => {
            assert_eq!(x.len(), y.len(), "{path} 的数组长度不一致");
            for (i, (xi, yi)) in x.iter().zip(y).enumerate() {
                assert_same_shape(xi, yi, &format!("{path}[{i}]"));
            }
        }
        (x, y) => assert_eq!(kind_of(x), kind_of(y), "{path} 的类型不一致：{x} vs {y}"),
    }
}

fn kind_of(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[tokio::test]
async fn the_done_payload_has_the_same_shape_as_the_old_route() {
    let t = TestDb::new("stream-shape");
    seed_user(&t);

    let app = state(&t, Some(Arc::new(TwoRoundProvider::default())));
    let sid = create_session(app.clone()).await;
    let streamed = parse_sse(
        &text(
            build_router(app)
                .oneshot(req(
                    "POST",
                    &format!("/api/sessions/{sid}/messages/stream"),
                    Some(serde_json::json!({ "content": "帮我看看" })),
                ))
                .await
                .expect("流式失败"),
        )
        .await,
    );
    let streamed_done = streamed
        .last()
        .filter(|(n, _)| n == "done")
        .map(|(_, d)| d.clone())
        .expect("最后一帧是 done");

    let app = state(&t, Some(Arc::new(TwoRoundProvider::default())));
    let sid2 = create_session(app.clone()).await;
    let plain = json(
        &text(
            build_router(app)
                .oneshot(req(
                    "POST",
                    &format!("/api/sessions/{sid2}/messages"),
                    Some(serde_json::json!({ "content": "帮我看看" })),
                ))
                .await
                .expect("老路由失败"),
        )
        .await,
    );

    assert_same_shape(&streamed_done, &plain, "done");
}

/// 一个只支持一次性调用的上游（`stream: true` 直接被拒）。
#[derive(Debug, Default)]
struct NoStreamProvider {
    round: AtomicUsize,
}

impl Provider for NoStreamProvider {
    fn name(&self) -> &str {
        "no-stream"
    }

    fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
        let _ = self.round.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(ChatResponse {
                id: None,
                model: request.model.clone(),
                text: "退回来了。".into(),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish_reason: Some(FinishReason::Stop),
                usage: TokenUsage::new(Some(7), Some(2)),
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
        Box::pin(async {
            Err(quill_provider::ProviderError::Status {
                code: 400,
                body: r#"{"error":"stream not supported"}"#.into(),
            })
        })
    }

    fn models<'a>(&'a self) -> BoxFuture<'a, Vec<ModelInfo>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[tokio::test]
async fn an_upstream_that_cannot_stream_still_answers_instead_of_failing() {
    let t = TestDb::new("stream-fallback");
    seed_user(&t);
    let app = state(&t, Some(Arc::new(NoStreamProvider::default())));
    let sid = create_session(app.clone()).await;

    let frames = parse_sse(
        &text(
            build_router(app)
                .oneshot(req(
                    "POST",
                    &format!("/api/sessions/{sid}/messages/stream"),
                    Some(serde_json::json!({ "content": "你好" })),
                ))
                .await
                .expect("oneshot 失败"),
        )
        .await,
    );

    assert!(
        frames.iter().all(|(n, _)| n != "error"),
        "上游不支持流式不是「这一轮失败」：{frames:#?}"
    );
    let done = frames
        .last()
        .filter(|(n, _)| n == "done")
        .map(|(_, d)| d.clone())
        .expect("退回一次性之后仍要有 done");
    assert_eq!(done["reply"], "退回来了。");
}

/// 一个什么都不给的上游：连一次性也失败。
#[derive(Debug, Default)]
struct DeadProvider;

impl Provider for DeadProvider {
    fn name(&self) -> &str {
        "dead"
    }

    fn chat<'a>(&'a self, _request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
        Box::pin(async move {
            Err(quill_provider::ProviderError::Unreachable {
                url: "http://127.0.0.1:1/v1".into(),
                detail: "连接被拒绝".into(),
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
        Box::pin(async {
            Err(quill_provider::ProviderError::Unreachable {
                url: "http://127.0.0.1:1/v1".into(),
                detail: "连接被拒绝".into(),
            })
        })
    }

    fn models<'a>(&'a self) -> BoxFuture<'a, Vec<ModelInfo>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[tokio::test]
async fn a_failure_inside_the_stream_arrives_as_an_error_frame_with_a_next_step() {
    let t = TestDb::new("stream-error");
    seed_user(&t);
    let app = state(&t, Some(Arc::new(DeadProvider)));
    let sid = create_session(app.clone()).await;

    let frames = parse_sse(
        &text(
            build_router(app)
                .oneshot(req(
                    "POST",
                    &format!("/api/sessions/{sid}/messages/stream"),
                    Some(serde_json::json!({ "content": "你好" })),
                ))
                .await
                .expect("oneshot 失败"),
        )
        .await,
    );

    let err = frames
        .last()
        .filter(|(n, _)| n == "error")
        .map(|(_, d)| d.clone())
        .unwrap_or_else(|| panic!("最后一帧必须是 error：{frames:#?}"));
    assert_eq!(err["code"], "provider_unavailable", "{err}");
    assert!(
        err["next_step"].as_str().is_some_and(|s| s.len() > 10),
        "错误帧必须带可执行的「下一步」，否则用户只看到一句连接被拒绝：{err}"
    );
}

#[tokio::test]
async fn an_empty_content_is_rejected_before_any_stream_starts() {
    let t = TestDb::new("stream-empty");
    seed_user(&t);
    let app = state(&t, Some(Arc::new(TwoRoundProvider::default())));
    let sid = create_session(app.clone()).await;

    let resp = build_router(app)
        .oneshot(req(
            "POST",
            &format!("/api/sessions/{sid}/messages/stream"),
            Some(serde_json::json!({ "content": "   " })),
        ))
        .await
        .expect("oneshot 失败");

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "准备阶段的错误仍然要是正常的 4xx 信封，不能变成一串流"
    );
    let body = json(&text(resp).await);
    assert_eq!(body["error"]["code"], "bad_request", "{body}");
    assert!(
        body["error"]["detail"]
            .as_str()
            .is_some_and(|s| s.contains("content")),
        "错误必须说清是哪个字段不对：{body}"
    );
}
