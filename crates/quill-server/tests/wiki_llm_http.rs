//! 资料库**摄入与问答**的 HTTP 端到端证据（queue Q057）。
//!
//! 这两条路此前是 501 桩，缺的是 `quill_adapters::KnowledgeBackend` 的唯一真实现
//! （`wiki_backend::ProviderKnowledge`）。判据不是「返回 200」，而是四条：
//!   - 摄入**真落盘**模型产出的页，并重建 `index.md`、写变更日志；
//!   - 送模型的提示词里**真的有源文**（否则「摄入」是空转）；
//!   - 模型不按约定回 JSON、或产出的页面不合法 → **报错且一个字都不写**
//!     （静默降级成「这次摄入什么都没做」是最坏的结果：用户以为存进去了）；
//!   - 问答的答案来自模型，且候选页来自确定性的索引检索。

mod common;

use std::sync::{Arc, Mutex, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_provider::{
    BoxFuture, ChatRequest, ChatResponse, FinishReason, MessageContent, ModelInfo, Provider,
    ProviderError, ProviderStream, TokenUsage,
};
use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN_A: &str = "tok-a";

const PAGE: &str = "---\ntitle: 入门\ntype: concept\ncreated: 2026-10-04\nupdated: 2026-10-04\n---\n\n这是入门页。\n";

/// 一个按脚本回话的 provider：把预设正文原样当模型输出，并记下收到的请求。
#[derive(Debug)]
struct ScriptedProvider {
    reply: Mutex<String>,
    seen: Mutex<Vec<ChatRequest>>,
}

impl ScriptedProvider {
    fn new(reply: &str) -> Self {
        Self {
            reply: Mutex::new(reply.to_string()),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn prompts(&self) -> String {
        self.seen
            .lock()
            .expect("锁")
            .iter()
            .flat_map(|r| r.messages.iter())
            .map(|m| match &m.content {
                MessageContent::Text { text } => text.clone(),
                other => format!("{other:?}"),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Provider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
        self.seen.lock().expect("锁").push(request.clone());
        let text = self.reply.lock().expect("锁").clone();
        Box::pin(async move {
            Ok(ChatResponse {
                id: None,
                model: request.model.clone(),
                text,
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish_reason: Some(FinishReason::Stop),
                usage: TokenUsage::new(Some(10), Some(10)),
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
        // 资料库这两条路不走流式；被调用到就是错的，如实报错而不是回一个空流。
        Box::pin(async {
            Err(ProviderError::Status {
                code: 501,
                body: "scripted provider 不支持流式".to_string(),
            })
        })
    }

    fn models<'a>(&'a self) -> BoxFuture<'a, Vec<ModelInfo>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn user_id() -> quill_domain::UserId {
    quill_domain::UserId::parse(UID_A).expect("测试 UID 必须合法")
}

fn wiki_root(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "quill-wiki-llm-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("建临时资料库根");
    p
}

fn app(t: &TestDb, wiki: std::path::PathBuf, provider: Arc<ScriptedProvider>) -> AppState {
    let resolver = EnvTokenResolver::new(vec![(
        TOKEN_A.to_string(),
        AuthContext {
            user_id: user_id(),
            is_admin: true,
        },
    )]);
    let mut config = Config::from_env();
    config.wiki_dir = wiki;
    AppState {
        config,
        tokens: Arc::new(resolver),
        db: Some(t.bridge()),
        db_problem: None,
        llm: Arc::new(RwLock::new(Some(
            provider as quill_provider::SharedProvider,
        ))),
        llm_config: Arc::new(RwLock::new(Default::default())),
        providers: Arc::new(RwLock::new(Default::default())),
        login_limiter: Arc::new(Default::default()),
        pbkdf2: quill_control::Pbkdf2Params::for_tests(),
    }
}

/// 本用例的资料库 store（与 `api_wiki::store_for` 同一构造方式）。
fn store(wiki: &std::path::Path) -> quill_wiki::WikiStore {
    quill_wiki::WikiStore::new(wiki, user_id())
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

async fn post(
    app: AppState,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let resp = build_router(app)
        .oneshot(req("POST", path, Some(body)))
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    (status, json(&text(resp).await))
}

/// 模型按约定回的摄入结果：一页 + 一句 summary。
fn ingest_reply() -> String {
    serde_json::json!({
        "pages": [{ "path": "concepts/入门.md", "content": PAGE }],
        "summary": "从 note.md 建了一页入门。",
    })
    .to_string()
}

#[tokio::test]
async fn ingesting_a_source_writes_the_page_rebuilds_the_index_and_logs_it() {
    let t = TestDb::new("wiki-ingest");
    let root = wiki_root("ingest");
    // 源文要先在 raw 层（这个端点只收路径，不收正文 —— 否则就是一次无出处的写入）。
    store(&root)
        .write_raw("note.md", "这是一份关于入门概念的源文。")
        .expect("铺源文");

    let provider = Arc::new(ScriptedProvider::new(&ingest_reply()));
    let app = app(&t, root.clone(), provider.clone());

    let (status, body) = post(
        app.clone(),
        "/api/wiki/ingest",
        serde_json::json!({ "source": "note.md" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "摄入应当成功：{body}");
    assert_eq!(body["written"], serde_json::json!(["concepts/入门.md"]));

    // 送模型的提示词里**真的有源文**：没有它，这次「摄入」就是空转。
    let prompts = provider.prompts();
    assert!(
        prompts.contains("这是一份关于入门概念的源文。"),
        "提示词里必须带上源文正文"
    );
    assert!(
        prompts.contains("note.md"),
        "提示词里必须点名源文的资料库路径"
    );

    // 真落盘 + 索引重建 + 变更日志。
    let store = store(&root);
    let page = store
        .read_page("concepts/入门.md")
        .expect("页面必须真的写进去了");
    assert_eq!(page.title(), "入门");
    let index = store.read_index().expect("读索引").expect("索引必须存在");
    assert!(index.contains("入门"), "索引里必须有这一页：{index}");
    let log = store.read_log().expect("读日志").expect("日志必须存在");
    assert!(log.contains("note.md"), "变更日志必须点名源文：{log}");
}

#[tokio::test]
async fn a_model_that_does_not_return_json_fails_loudly_and_writes_nothing() {
    let t = TestDb::new("wiki-ingest-garbage");
    let root = wiki_root("ingest-garbage");
    store(&root).write_raw("note.md", "源文。").expect("铺源文");

    let provider = Arc::new(ScriptedProvider::new("抱歉，我不太确定该做什么。"));
    let app = app(&t, root.clone(), provider);

    let (status, body) = post(
        app,
        "/api/wiki/ingest",
        serde_json::json!({ "source": "note.md" }),
    )
    .await;
    assert!(
        status.is_server_error(),
        "模型没按约定回 JSON 必须是错误，实际 {status}：{body}"
    );
    assert!(
        body["error"]["next_step"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "错误必须带下一步：{body}"
    );
    // 一个字都不许写进去。
    let store = store(&root);
    assert_eq!(
        store.list_pages().expect("列页").len(),
        0,
        "失败不许留下半页"
    );
    assert!(
        store.read_index().expect("读索引").is_none(),
        "失败不许留下索引"
    );
}

#[tokio::test]
async fn a_page_that_does_not_parse_is_rejected_before_it_lands() {
    let t = TestDb::new("wiki-ingest-badpage");
    let root = wiki_root("ingest-badpage");
    store(&root).write_raw("note.md", "源文。").expect("铺源文");

    // 缺 frontmatter 的正文：`page_from_wire` 必须挡住它。
    let reply = serde_json::json!({
        "pages": [{ "path": "concepts/坏页.md", "content": "# 只有正文\n" }],
        "summary": "建了一页。",
    })
    .to_string();
    let provider = Arc::new(ScriptedProvider::new(&reply));
    let app = app(&t, root.clone(), provider);

    let (status, body) = post(
        app,
        "/api/wiki/ingest",
        serde_json::json!({ "source": "note.md" }),
    )
    .await;
    assert!(status.is_server_error(), "不合法页面必须报错：{body}");
    let store = store(&root);
    assert_eq!(store.list_pages().expect("列页").len(), 0, "坏页不许落盘");
}

#[tokio::test]
async fn querying_answers_from_the_candidate_pages() {
    let t = TestDb::new("wiki-query");
    let root = wiki_root("query");
    // 先造一页并建索引（走的是确定性那半，不经过模型）。
    let store = store(&root);
    store.ensure_layout().expect("建三层");
    store.write_page("concepts/入门.md", PAGE).expect("写页");
    let pages = store.load_all_pages().expect("读全部页");
    let idx = quill_wiki::ingest::build_index(&pages);
    store.write_index(&idx.render()).expect("写索引");

    let reply = serde_json::json!({
        "answer": "入门页讲的是入门概念。",
        "citations": ["concepts/入门.md"],
    })
    .to_string();
    let provider = Arc::new(ScriptedProvider::new(&reply));
    let app = app(&t, root, provider.clone());

    let (status, body) = post(
        app,
        "/api/wiki/query",
        serde_json::json!({ "question": "入门" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "问答应当成功：{body}");
    assert_eq!(body["answer"], "入门页讲的是入门概念。");
    assert_eq!(body["used_pages"], serde_json::json!(["concepts/入门.md"]));
    // 候选页正文必须进了提示词 —— 否则模型是在凭空作答。
    assert!(
        provider.prompts().contains("这是入门页。"),
        "候选页正文必须在提示词里"
    );
}

#[tokio::test]
async fn an_empty_question_is_rejected_before_calling_the_model() {
    let t = TestDb::new("wiki-query-empty");
    let root = wiki_root("query-empty");
    let provider = Arc::new(ScriptedProvider::new("{}"));
    let app = app(&t, root, provider.clone());

    let (status, _) = post(
        app,
        "/api/wiki/query",
        serde_json::json!({ "question": "  " }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "空白问题要 400");
    assert!(
        provider.seen.lock().expect("锁").is_empty(),
        "空问题不许白调一次模型"
    );
}
