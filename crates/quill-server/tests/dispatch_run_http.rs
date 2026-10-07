//! `POST /api/teams/{id}/dispatch/run` —— 派工**真执行**的契约。
//!
//! 钉住四件事，每一件都是「只记账」那一版做不到的：
//!
//! 1. 真的每个成员各调一次模型，正文进 `results`（不是 `executed:false`）；
//! 2. 幂等：同一轮重复提交，第二次不再执行，成员进 `skipped_as_duplicate`；
//! 3. 团队不存在 → 404，不是 200 加一份空结果；
//! 4. 派给**不属于这个团**的专家 → 4xx，且一个成员都没跑（名册校验不是摆设）。

mod common;
mod dispatch_seed;

use std::sync::{Arc, Mutex, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_provider::{
    BoxFuture, ChatRequest, ChatResponse, ModelInfo, Provider, ProviderError, ProviderStream,
};
use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;
use dispatch_seed::{member_session, seed};

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN_A: &str = "tok-a";

fn user_id() -> quill_domain::UserId {
    quill_domain::UserId::parse(UID_A).expect("测试 UID 必须合法")
}

fn expert(name: &str) -> quill_adapters::ExpertId {
    quill_adapters::ExpertId::parse(name).expect("测试专家名必须合法")
}

/// 每次调用回一段带序号的正文 —— 「调了几次、第几次是谁」一眼可判。
#[derive(Debug)]
struct EchoProvider {
    calls: Mutex<Vec<String>>,
}

impl EchoProvider {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("测试锁不该毒化").len()
    }
}

impl Provider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
        let n = {
            let mut c = self.calls.lock().expect("测试锁不该毒化");
            c.push(request.model.clone());
            c.len()
        };
        let model = request.model.clone();
        Box::pin(async move {
            Ok(ChatResponse {
                id: None,
                model,
                text: format!("成员产出 #{n}"),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish_reason: None,
                usage: Default::default(),
            })
        })
    }

    fn stream<'a>(&'a self, _r: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
        Box::pin(async {
            Err(ProviderError::NotConfigured {
                detail: "这条测试不走流式".to_string(),
            })
        })
    }

    fn models<'a>(&'a self) -> BoxFuture<'a, Vec<ModelInfo>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn state(t: &TestDb, provider: Arc<EchoProvider>) -> AppState {
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
        llm: Arc::new(RwLock::new(Some(provider as Arc<dyn Provider>))),
        llm_config: Arc::new(RwLock::new(quill_server::llm::LlmConfig {
            model: "stub-model".to_string(),
            base_url: "http://stub.invalid/v1".to_string(),
            ..Default::default()
        })),
        providers: Arc::new(RwLock::new(Default::default())),
        login_limiter: Arc::new(Default::default()),
        pbkdf2: quill_control::Pbkdf2Params::for_tests(),
    }
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

async fn call(
    app: &AppState,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN_A}"));
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    let request = b
        .body(match body {
            Some(v) => Body::from(v.to_string()),
            None => Body::empty(),
        })
        .expect("构造请求失败");
    let resp = build_router(app.clone())
        .oneshot(request)
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    (status, json(&text(resp).await))
}

fn team_hex(id: [u8; 16]) -> String {
    quill_domain::UserId::from_bytes(id).to_compact_hex()
}

fn member_entry(team_seed: u8, expert_name: &str, seq: u32) -> serde_json::Value {
    let e = expert(expert_name);
    let member = quill_adapters::MemberId::for_expert(&e, seq).expect("成员标识合法");
    serde_json::json!({
        "expert": expert_name,
        "member": member.as_str(),
        "title": format!("分析 {expert_name}"),
        "instructions": "给出三点结论",
        "member_session_id": member_session(team_seed, &e).to_compact_hex(),
    })
}

fn body(f: &dispatch_seed::Fixture, members: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "room_id": f.room_id,
        "round": 0,
        "leader_session_id": f.leader_session.to_compact_hex(),
        "members": members,
    })
}

#[tokio::test]
async fn running_a_round_really_calls_the_model_and_returns_the_output() {
    let t = TestDb::new("dispatch-run-exec");
    let f = seed(&t.bridge(), user_id(), 0x21, &["cost-analyst"]);
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(f.team_id)),
        Some(body(&f, vec![member_entry(0x21, "cost-analyst", 1)])),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "真执行应 200：{v}");
    assert_eq!(v["executed"], serde_json::json!(true), "{v}");
    assert_eq!(v["delivered"], serde_json::json!(1), "{v}");
    assert_eq!(v["failed"], serde_json::json!(0), "{v}");
    assert_eq!(
        v["results"][0]["output"],
        serde_json::json!("成员产出 #1"),
        "成员的模型产出必须进 results（这正是「只记账」那一版给不出的东西）：{v}"
    );
    assert_eq!(
        v["results"][0]["scope"],
        serde_json::json!("分析 cost-analyst"),
        "{v}"
    );
    assert_eq!(provider.call_count(), 1, "必须真的调了一次模型");
}

#[tokio::test]
async fn running_the_same_round_twice_only_executes_once() {
    let t = TestDb::new("dispatch-run-idempotent");
    let f = seed(&t.bridge(), user_id(), 0x22, &["cost-analyst"]);
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));
    let req_body = body(&f, vec![member_entry(0x22, "cost-analyst", 1)]);
    let path = format!("/api/teams/{}/dispatch/run", team_hex(f.team_id));

    let (s1, v1) = call(&app, "POST", &path, Some(req_body.clone())).await;
    assert_eq!(s1, StatusCode::OK, "{v1}");
    assert_eq!(v1["delivered"], serde_json::json!(1), "{v1}");

    let (s2, v2) = call(&app, "POST", &path, Some(req_body)).await;
    assert_eq!(s2, StatusCode::OK, "重复提交同一轮不该报错：{v2}");
    assert_eq!(
        v2["delivered"],
        serde_json::json!(0),
        "第二次不该再执行：{v2}"
    );
    assert_eq!(
        v2["skipped_as_duplicate"],
        serde_json::json!(["cost-analyst-1"]),
        "被台账挡下的成员要如实点名：{v2}"
    );
    assert_eq!(
        provider.call_count(),
        1,
        "🔴 幂等闸门没拦住第二次模型调用——那是重复烧钱"
    );
}

#[tokio::test]
async fn running_against_a_missing_team_is_404_not_an_empty_success() {
    let t = TestDb::new("dispatch-run-missing-team");
    let f = seed(&t.bridge(), user_id(), 0x23, &["cost-analyst"]);
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));

    // 用同一个 room/round/成员，但团队 id 换成一个不存在的。
    let mut bogus = [0u8; 16];
    bogus[0] = 0xEE;
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(bogus)),
        Some(body(&f, vec![member_entry(0x23, "cost-analyst", 1)])),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "不存在的团队必须 404：{v}");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("entity_not_found"),
        "{v}"
    );
    assert_eq!(provider.call_count(), 0, "团队都不存在，一个成员都不该跑");
}

#[tokio::test]
async fn dispatching_to_an_expert_outside_the_team_runs_nobody() {
    let t = TestDb::new("dispatch-run-not-a-member");
    let f = seed(&t.bridge(), user_id(), 0x24, &["cost-analyst"]);
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));

    // `ghost-analyst` 不在这个团的名册里。名册校验若被架空（拿请求里的专家去建团），
    // 它会照跑——那条测试就会红。
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(f.team_id)),
        Some(body(&f, vec![member_entry(0x24, "ghost-analyst", 1)])),
    )
    .await;

    assert!(
        status.is_client_error(),
        "派给非本团专家必须是客户端错误，实际 {status}：{v}"
    );
    assert_eq!(
        provider.call_count(),
        0,
        "🔴 名册校验没拦住：不属于这个团的专家被真的执行了"
    );
}
