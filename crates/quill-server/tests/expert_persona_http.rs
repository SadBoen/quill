//! 专家人格的 HTTP 契约 + 人格注入的端到端证据。
//!
//! 注入测试不看 HTTP 状态码，而是把服务端真正发给 provider 的 `ChatRequest`
//! 抓下来逐条断言 —— 界面暗示了「选专家」而服务端没读 `sessions.expert_id`
//! 这种假接线，只有断言到 provider 收到的请求体才抓得住。

mod common;

use std::sync::{Arc, Mutex, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_provider::{
    BoxFuture, ChatRequest, ChatResponse, ModelInfo, Provider, ProviderStream,
};
use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN_A: &str = "tok-a";

const PERSONA: &str = "你是一名严谨的成本分析师：先问清口径再算数，结论必须带算式。";

/// 会把每次收到的请求原样留档的假 provider。
#[derive(Debug, Default)]
struct RecordingProvider {
    seen: Mutex<Vec<ChatRequest>>,
}

impl RecordingProvider {
    fn last(&self) -> ChatRequest {
        self.seen
            .lock()
            .expect("录制锁不应被毒化")
            .last()
            .cloned()
            .expect("provider 至少应被调用过一次")
    }
}

impl Provider for RecordingProvider {
    fn name(&self) -> &str {
        "recording"
    }

    fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
        self.seen
            .lock()
            .expect("录制锁不应被毒化")
            .push(request.clone());
        Box::pin(async move {
            Ok(ChatResponse {
                id: Some("chatcmpl-stub".into()),
                model: request.model.clone(),
                text: "（桩回复）".into(),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish_reason: None,
                usage: Default::default(),
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
        // 流式路径本文件不覆盖；这里直接判错，免得为了造一个空 stream
        // 往 dev-dependencies 里加 futures。
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

fn state(t: &TestDb, provider: Option<Arc<RecordingProvider>>) -> AppState {
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
        llm: Arc::new(RwLock::new(
            provider.map(|p| p as quill_provider::SharedProvider),
        )),
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
    serde_json::from_str(t).unwrap_or_else(|e| panic!("响应必须是 JSON（{e}）：{t}"))
}

/// 建一个用户（sessions 有外键指向 users，测试里没有播种这一步）。
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

async fn create_session(app: AppState, expert_id: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::json!({ "title": "人格测试" });
    if let Some(e) = expert_id {
        body["expert_id"] = serde_json::json!(e);
    }
    let resp = build_router(app)
        .oneshot(req("POST", "/api/sessions", Some(body)))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK, "建会话应成功");
    json(&text(resp).await)
}

#[tokio::test]
async fn post_with_persona_returns_201_with_both_new_fields() {
    let t = TestDb::new("persona-create");
    let app = state(&t, None);

    let resp = build_router(app)
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(serde_json::json!({
                "id": "cost-analyst",
                "display_name": "成本分析师",
                "description": "算清本月成本",
                "instructions": PERSONA,
                "model": "qwen3-max"
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::CREATED, "创建应 201");
    let v = json(&text(resp).await);
    assert_eq!(v["instructions"], serde_json::json!(PERSONA));
    assert_eq!(v["model"], serde_json::json!("qwen3-max"));
    assert!(
        v.as_object().expect("必须是对象").contains_key("model"),
        "model 键必须存在（缺失会被前端当成后端没做这个字段）"
    );

    let resp = build_router(state(&t, None))
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(serde_json::json!({
                "id": "plain-expert",
                "display_name": "普通专家",
                "description": "没有人格"
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = json(&text(resp).await);
    assert_eq!(v["instructions"], serde_json::json!(""), "缺省必须是空串");
    assert!(
        v["model"].is_null(),
        "缺省的 model 必须是 JSON null（= 跟随实例默认模型），实际 {}",
        v["model"]
    );
}

#[tokio::test]
async fn patching_instructions_keeps_model_and_null_clears_it() {
    let t = TestDb::new("persona-patch");
    let app = state(&t, None);
    build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(serde_json::json!({
                "id": "cost-analyst",
                "display_name": "成本分析师",
                "description": "算清本月成本",
                "instructions": PERSONA,
                "model": "qwen3-max"
            })),
        ))
        .await
        .expect("oneshot 失败");

    let resp = build_router(app.clone())
        .oneshot(req(
            "PATCH",
            "/api/experts/cost-analyst",
            Some(serde_json::json!({ "instructions": "只改人格，不碰模型" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json(&text(resp).await);
    assert_eq!(v["instructions"], serde_json::json!("只改人格，不碰模型"));
    assert_eq!(
        v["model"],
        serde_json::json!("qwen3-max"),
        "🔴 只传 instructions 时 model 必须沿用原值（PATCH 是部分更新，不是覆盖）"
    );

    let resp = build_router(app.clone())
        .oneshot(req(
            "PATCH",
            "/api/experts/cost-analyst",
            Some(serde_json::json!({ "model": null })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json(&text(resp).await);
    assert!(
        v["model"].is_null(),
        "显式 model: null 必须清除偏好模型，实际 {}",
        v["model"]
    );
    assert_eq!(
        v["instructions"],
        serde_json::json!("只改人格，不碰模型"),
        "清 model 不得顺手把人格也清掉"
    );

    let resp = build_router(app)
        .oneshot(req(
            "GET",
            "/api/experts/cost-analyst",
            None,
        ))
        .await
        .expect("oneshot 失败");
    let v = json(&text(resp).await);
    assert!(
        v["model"].is_null(),
        "清除必须真的落库（重启后也得是 null）"
    );
}

#[tokio::test]
async fn unknown_persona_field_is_rejected_and_over_long_instructions_is_400_with_next_step() {
    let t = TestDb::new("persona-badinput");
    let app = state(&t, None);

    let resp = build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(serde_json::json!({
                "id": "cost-analyst",
                "display_name": "成本分析师",
                "description": "算清成本",
                "persona": "把人格塞到另一个字段名里"
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "🔴 人格字段名拼错必须判红（静默忽略会让人以为人格生效了）"
    );
    let body = text(resp).await;
    assert!(body.contains("persona"), "应点名是哪个字段：{body}");
    assert!(body.contains("instructions"), "应列出可接受字段：{body}");

    let resp = build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(serde_json::json!({
                "id": "cost-analyst",
                "display_name": "成本分析师",
                "description": "算清成本",
                "instructions": "人".repeat(20_001)
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "超长人格必须 400");
    let body = text(resp).await;
    assert!(body.contains("下一步"), "错误必须带修复方向：{body}");
    assert!(body.contains("20000"), "应给出上限：{body}");
    assert!(body.contains("next_step"), "响应体结构必须带下一步字段：{body}");

    let resp = build_router(app)
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(serde_json::json!({
                "id": "cost-analyst",
                "display_name": "成本分析师",
                "description": "算清成本",
                "model": "   "
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "空模型名必须 400");
    let body = text(resp).await;
    assert!(body.contains("下一步"), "错误必须带修复方向：{body}");
}

#[tokio::test]
async fn bound_experts_persona_reaches_the_provider_as_a_system_message() {
    let t = TestDb::new("persona-inject");
    seed_user(&t);
    let rec = Arc::new(RecordingProvider::default());
    let app = state(&t, Some(Arc::clone(&rec)));

    build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(serde_json::json!({
                "id": "cost-analyst",
                "display_name": "成本分析师",
                "description": "算清本月成本",
                "instructions": PERSONA
            })),
        ))
        .await
        .expect("oneshot 失败");

    let session = create_session(app.clone(), Some("cost-analyst")).await;
    assert_eq!(
        session["persona_applied"],
        serde_json::json!(true),
        "建会话时就该体检出人格可用：{session}"
    );

    let sid = session["id"].as_str().expect("建会话必须回 id");
    let resp = build_router(app)
        .oneshot(req(
            "POST",
            &format!("/api/sessions/{sid}/messages"),
            Some(serde_json::json!({ "content": "本月云成本多少" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json(&text(resp).await);
    assert_eq!(body["persona_applied"], serde_json::json!(true));

    let sent = rec.last();
    let system: Vec<&str> = sent
        .messages
        .iter()
        .filter(|m| m.role == quill_provider::Role::System)
        .filter_map(|m| m.text())
        .collect();
    assert_eq!(
        system,
        vec![PERSONA],
        "🔴 绑了带人格的专家就必须把那条 instructions 作为 system 消息发给模型：{:?}",
        sent.messages
    );
    assert_eq!(
        sent.messages.first().map(|m| m.role),
        Some(quill_provider::Role::System),
        "人格必须排在历史之前"
    );
    assert_eq!(
        sent.messages.last().and_then(|m| m.text()),
        Some("本月云成本多少"),
        "本轮用户消息必须仍在末尾"
    );
}

#[tokio::test]
async fn an_expert_without_instructions_sends_no_system_message_at_all() {
    let t = TestDb::new("persona-empty");
    seed_user(&t);
    let rec = Arc::new(RecordingProvider::default());
    let app = state(&t, Some(Arc::clone(&rec)));

    build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(serde_json::json!({
                "id": "plain-expert",
                "display_name": "普通专家",
                "description": "没有人格正文",
                "instructions": "   "
            })),
        ))
        .await
        .expect("oneshot 失败");

    let session = create_session(app.clone(), Some("plain-expert")).await;
    assert_eq!(
        session["persona_applied"],
        serde_json::json!(false),
        "空白人格不算人格：{session}"
    );

    let sid = session["id"].as_str().expect("建会话必须回 id");
    let resp = build_router(app)
        .oneshot(req(
            "POST",
            &format!("/api/sessions/{sid}/messages"),
            Some(serde_json::json!({ "content": "你好" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);

    let sent = rec.last();
    assert!(
        sent.messages.iter().all(|m| m.role != quill_provider::Role::System),
        "🔴 空人格不得注入空 system 消息（那是给模型发噪声）：{:?}",
        sent.messages
    );
    assert_eq!(sent.messages.len(), 1, "只应有本轮用户消息");
}

#[tokio::test]
async fn a_deleted_or_missing_expert_degrades_to_default_persona_and_says_so() {
    let t = TestDb::new("persona-deleted");
    seed_user(&t);
    let rec = Arc::new(RecordingProvider::default());
    let app = state(&t, Some(Arc::clone(&rec)));

    build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(serde_json::json!({
                "id": "doomed-expert",
                "display_name": "将被删除的专家",
                "description": "用来验证降级",
                "instructions": PERSONA
            })),
        ))
        .await
        .expect("oneshot 失败");

    let session = create_session(app.clone(), Some("doomed-expert")).await;
    let sid = session["id"].as_str().expect("建会话必须回 id").to_string();

    build_router(app.clone())
        .oneshot(req("DELETE", "/api/experts/doomed-expert", None))
        .await
        .expect("oneshot 失败");

    let resp = build_router(app.clone())
        .oneshot(req(
            "POST",
            &format!("/api/sessions/{sid}/messages"),
            Some(serde_json::json!({ "content": "还在吗" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK, "专家没了不该让对话直接瘫掉");
    let body = json(&text(resp).await);
    assert_eq!(body["persona_applied"], serde_json::json!(false));
    let notice = body["expert_notice"]
        .as_str()
        .unwrap_or_else(|| panic!("🔴 专家不可用必须在响应里明说，不能静默降级：{body}"));
    assert!(notice.contains("doomed-expert"), "提示要点名是哪个专家：{notice}");
    assert!(notice.contains("下一步"), "提示必须带修复方向：{notice}");

    let sent = rec.last();
    assert!(
        sent.messages.iter().all(|m| m.role != quill_provider::Role::System),
        "🔴 专家已删时不得把旧人格继续注入：{:?}",
        sent.messages
    );
    assert_eq!(sent.messages.len(), 1);

    let resp = build_router(app)
        .oneshot(req(
            "POST",
            "/api/sessions",
            Some(serde_json::json!({ "title": "选了不存在的专家", "expert_id": "ghost-expert" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json(&text(resp).await);
    let notice = body["expert_notice"]
        .as_str()
        .unwrap_or_else(|| panic!("建会话时就绑了不存在的专家也必须提示：{body}"));
    assert!(notice.contains("ghost-expert"), "{notice}");
    assert_eq!(body["persona_applied"], serde_json::json!(false));
}
