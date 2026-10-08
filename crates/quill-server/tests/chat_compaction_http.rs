//! 上下文压缩的端到端证据（queue Q018）。
//!
//! 这条测试要回答的不是「有没有多一个字段」，而是：**`compaction_threshold_tokens`
//! 到底有没有人读**。所以它看三处真行为：
//!
//! 1. 超阈值时服务端**真的调了一次摘要模型**（用上游请求里那句摘要请求文案认出来），
//!    并且把摘要**真的喂给了这一轮对话**（最后那次请求里能看到摘要内容）；
//! 2. 没超阈值时**一次都不调**（否则「压缩」会变成每轮白花的钱）；
//! 3. 响应里的 `context` 数字来自真实估算，且 `compacted` 与上面两件事一致。
//!
//! 反向验证（改回坏样子应变红）：把 `prepare_history` 换成直接映射历史
//! （即不接线），`compacted` 永远是 false、`compaction_calls` 恒为 0，两条断言都红。

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_provider::{BoxFuture, ChatRequest, ChatResponse, ModelInfo, Provider, ProviderStream};
use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::chat_repo::{insert_message, NewMessage};
use quill_server::config::Config;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN_A: &str = "tok-a";

/// 与内核 `compaction.rs` 的 `SUMMARIZE_REQUEST_TEXT` 逐字相同。上游请求里出现它，
/// 就说明这次调用是「生成摘要」而不是普通对话。
const SUMMARIZE_REQUEST: &str =
    "Please summarize the conversation history provided in the system prompt.";

/// 摘要模型回一份结构化摘要（内核会从响应文本里找 JSON 块）。
const SUMMARY_JSON: &str = r#"{"user_intent":["把三千字的历史压成一段"],"current_work":"接线压缩","next_step":"继续对话"}"#;
/// 摘要里那个独有词 —— 用它证明「摘要真的进了下一轮请求」。
const SUMMARY_MARKER: &str = "把三千字的历史压成一段";

/// 阈值取配置下限（内核校验要求 ≥ 4001），这样测试只需造 4002 个 token 的历史。
const THRESHOLD: u32 = 4001;

#[derive(Debug, Default)]
struct CompactionProvider {
    seen: Mutex<Vec<ChatRequest>>,
    compaction_calls: AtomicUsize,
}

impl CompactionProvider {
    fn is_compaction(request: &ChatRequest) -> bool {
        request
            .messages
            .iter()
            .any(|m| m.text() == Some(SUMMARIZE_REQUEST))
    }

    /// 最后一次**非摘要**调用 —— 也就是这一轮真正的对话请求。
    fn last_chat(&self) -> ChatRequest {
        self.seen
            .lock()
            .expect("录制锁")
            .iter()
            .rev()
            .find(|r| !Self::is_compaction(r))
            .cloned()
            .expect("至少应有一次普通对话调用")
    }
}

impl Provider for CompactionProvider {
    fn name(&self) -> &str {
        "compaction"
    }

    fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
        self.seen.lock().expect("录制锁").push(request.clone());
        let compaction = Self::is_compaction(request);
        if compaction {
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
        }
        Box::pin(async move {
            Ok(ChatResponse {
                id: None,
                model: request.model.clone(),
                text: if compaction {
                    SUMMARY_JSON.to_string()
                } else {
                    "这一轮的回答。".to_string()
                },
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish_reason: None,
                usage: Default::default(),
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
        Box::pin(async {
            Err(quill_provider::ProviderError::NotConfigured {
                detail: "本测试只覆盖非流式 chat".into(),
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

fn state(t: &TestDb, provider: Arc<CompactionProvider>) -> AppState {
    let resolver = EnvTokenResolver::new(vec![(
        TOKEN_A.to_string(),
        AuthContext {
            user_id: user_id(),
            is_admin: true,
        },
    )]);
    let llm_config = quill_core::llm::LlmConfig {
        compaction_threshold_tokens: THRESHOLD,
        ..Default::default()
    };
    AppState {
        config: Config::from_env(),
        tokens: Arc::new(resolver),
        db: Some(t.bridge()),
        db_problem: None,
        llm: Arc::new(RwLock::new(Some(
            provider as quill_provider::SharedProvider,
        ))),
        llm_config: Arc::new(RwLock::new(llm_config)),
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
    serde_json::from_str(t).unwrap_or_else(|e| panic!("响应必须是 JSON（{e}）：{t}"))
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
            Some(serde_json::json!({ "title": "压缩测试" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK, "建会话应成功");
    let body = json(&text(resp).await);
    body["id"].as_str().expect("建会话必须回 id").to_string()
}

/// 直接往库里塞历史（不走 HTTP）：本测试要的是「历史已经很长」这个前提，
/// 用 POST 先聊几轮会把测试和对话循环的其它行为绑在一起。
fn seed_history(t: &TestDb, sid_hex: &str, turns: usize, chars_per_message: usize) {
    let sid = quill_domain::SessionId::parse(sid_hex)
        .expect("会话 id 必须是 16 字节十六进制")
        .as_bytes()
        .to_vec();
    let sid: [u8; 16] = sid.try_into().expect("会话 id 是 16 字节");
    let uid = user_id();
    // 中文一个字算一个 token（壳侧的估算规则），所以按字数造就能精确控制估算量。
    let long = "历史内容".repeat(chars_per_message / 4);
    for turn in 0..turns {
        for (role, text) in [("user", &long), ("assistant", &long)] {
            let seq = (turn * 2) as i64;
            let mut id = [0u8; 16];
            id[0] = (turn as u8).wrapping_add(1);
            id[1] = if role == "user" { 1 } else { 2 };
            insert_message(
                t.bridge().as_ref(),
                uid,
                sid,
                NewMessage {
                    id,
                    seq: seq + if role == "user" { 1 } else { 2 },
                    role: role.to_string(),
                    status: "complete".to_string(),
                    content: text.clone(),
                    reasoning: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    turn_ms: None,
                    created_at: seq,
                },
            )
            .expect("塞历史失败");
        }
    }
}

#[tokio::test]
async fn a_history_over_the_threshold_is_really_compacted() {
    let t = TestDb::new("compaction-over");
    seed_user(&t);
    let provider = Arc::new(CompactionProvider::default());
    let app = state(&t, Arc::clone(&provider));
    let sid = create_session(app.clone()).await;

    // 8 条 × 1000 字 = 8000 token（估算），远超阈值 4001。
    seed_history(&t, &sid, 4, 1000);

    let resp = build_router(app)
        .oneshot(req(
            "POST",
            &format!("/api/sessions/{sid}/messages"),
            Some(serde_json::json!({ "content": "接着刚才的继续说" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json(&text(resp).await);

    assert_eq!(
        body["context"]["compacted"],
        serde_json::json!(true),
        "历史估算超过阈值就必须真的压缩：{body}"
    );
    assert_eq!(
        provider.compaction_calls.load(Ordering::SeqCst),
        1,
        "超阈值必须**恰好**调一次摘要模型，不多不少"
    );
    let estimated = body["context"]["estimated_history_tokens"]
        .as_u64()
        .expect("估算值必须是数字");
    assert!(
        estimated as u32 > THRESHOLD,
        "报出来的估算值要真的超过阈值（{estimated} vs {THRESHOLD}）"
    );

    // 摘要必须真的进了这一轮请求：模型看到的是摘要，不是那 8000 token 原文。
    let sent = provider.last_chat();
    let saw_summary = sent
        .messages
        .iter()
        .any(|m| m.text().is_some_and(|t| t.contains(SUMMARY_MARKER)));
    assert!(
        saw_summary,
        "压缩后喂给模型的历史里必须含摘要内容：{:?}",
        sent.messages
            .iter()
            .filter_map(|m| m.text())
            .collect::<Vec<_>>()
    );
    // 压缩后不该再把 8 条原文都带上。**恰好留一条**是设计（goose 的
    // `CompactionMode::Auto` 会原样保留最近一条纯文本 user 消息，见内核
    // `compaction.rs` 的 `preserved_user`）—— 多于一条就说明没压住。
    let long_kept = sent
        .messages
        .iter()
        .filter(|m| m.text().is_some_and(|t| t.contains("历史内容历史内容")))
        .count();
    assert_eq!(
        long_kept,
        1,
        "压缩后只应保留最近那一条 user 原文，实际保留 {long_kept} 条：{:?}",
        sent.messages
            .iter()
            .filter_map(|m| m.text())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_short_history_is_left_alone_and_costs_no_summary_call() {
    let t = TestDb::new("compaction-under");
    seed_user(&t);
    let provider = Arc::new(CompactionProvider::default());
    let app = state(&t, Arc::clone(&provider));
    let sid = create_session(app.clone()).await;

    // 2 条 × 100 字 = 200 token，远低于阈值。
    seed_history(&t, &sid, 1, 100);

    let resp = build_router(app)
        .oneshot(req(
            "POST",
            &format!("/api/sessions/{sid}/messages"),
            Some(serde_json::json!({ "content": "你好" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json(&text(resp).await);

    assert_eq!(
        body["context"]["compacted"],
        serde_json::json!(false),
        "没超阈值不许压：{body}"
    );
    assert_eq!(
        provider.compaction_calls.load(Ordering::SeqCst),
        0,
        "没超阈值时一次摘要调用都不许有（否则每轮白花钱）"
    );
    assert_eq!(
        body["context"]["note"],
        serde_json::Value::Null,
        "没超阈值不是「失败」，note 必须是 null：{body}"
    );
    // 历史仍原样进上下文（100 字那条得能看到）。
    let sent = provider.last_chat();
    assert!(
        sent.messages
            .iter()
            .any(|m| m.text().is_some_and(|t| t.contains("历史内容"))),
        "未压缩时原历史必须照常带上"
    );
}
