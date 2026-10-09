//! `POST /api/teams/{id}/dispatch/run` —— 派工**真执行**的契约。
//!
//! 钉住五件事，每一件都是「只记账」那一版做不到的：
//!
//! 1. 真的每个成员各调一次模型，正文进 `results`（不是 `executed:false`）；
//! 2. 幂等：同一轮重复提交，第二次不再执行，成员进 `skipped_as_duplicate`；
//! 3. 团队不存在 → 404，不是 200 加一份空结果；
//! 4. 派给**不属于这个团**的专家 → 4xx，且一个成员都没跑（名册校验不是摆设）；
//! 5. 成员的任务与产出写进**各自的会话**，缺的成员会话先建出来（Q025）。

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
use dispatch_seed::{member_session, seed, seed_limits};

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN_A: &str = "tok-a";

fn user_id() -> quill_domain::UserId {
    quill_domain::UserId::parse(UID_A).expect("测试 UID 必须合法")
}

fn expert(name: &str) -> quill_adapters::ExpertId {
    quill_adapters::ExpertId::parse(name).expect("测试专家名必须合法")
}

/// 每次调用回一段带序号的正文 —— 「调了几次、第几次是谁」一眼可判。
/// 同时记下每个请求里的 **system 与 user 正文**（Q043 的 guidelines 用例要看
/// 「准则有没有真的进提示」，只看调用次数证明不了这件事）。
#[derive(Debug)]
struct EchoProvider {
    calls: Mutex<Vec<String>>,
    prompts: Mutex<Vec<(String, String)>>,
}

impl EchoProvider {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            prompts: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("测试锁不该毒化").len()
    }

    /// 每个请求的 `(system, user)` 正文，按调用顺序。
    fn prompts(&self) -> Vec<(String, String)> {
        self.prompts.lock().expect("测试锁不该毒化").clone()
    }
}

fn text_of(m: &quill_provider::Message) -> String {
    match &m.content {
        quill_provider::MessageContent::Text { text } => text.clone(),
        other => format!("{other:?}"),
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
        {
            let sys = request
                .messages
                .iter()
                .find(|m| matches!(m.role, quill_provider::Role::System))
                .map(text_of)
                .unwrap_or_default();
            let user = request
                .messages
                .iter()
                .find(|m| matches!(m.role, quill_provider::Role::User))
                .map(text_of)
                .unwrap_or_default();
            self.prompts
                .lock()
                .expect("测试锁不该毒化")
                .push((sys, user));
        }
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

    // Q025：被挡下的一轮也不许往成员会话里再追加一对消息。
    assert_eq!(
        v2["member_messages_written"],
        serde_json::json!(0),
        "没真跑的成员没有产出可写：{v2}"
    );
    let sid = member_session(0x22, &expert("cost-analyst"));
    let (status, msgs) = call(
        &app,
        "GET",
        &format!("/api/sessions/{}/messages", sid.to_compact_hex()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{msgs}");
    assert_eq!(
        msgs["messages"].as_array().expect("数组").len(),
        2,
        "重复提交不许把同一轮的任务与产出写第二遍：{msgs}"
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

// ---------------------------------------------------------------- Q043：团队限制

/// 把 `body()` 的 round 换掉（限制用例要测 round 1+）。
fn body_round(
    f: &dispatch_seed::Fixture,
    round: u32,
    members: Vec<serde_json::Value>,
) -> serde_json::Value {
    let mut v = body(f, members);
    v["round"] = serde_json::json!(round);
    v
}

#[tokio::test]
async fn a_round_over_the_team_max_dispatch_is_400_and_calls_no_model() {
    let t = TestDb::new("dispatch-run-over-max-dispatch");
    // 团队限制：一轮最多 2 个成员；这一轮要派 3 个。
    let f = seed_limits(
        &t.bridge(),
        user_id(),
        0x31,
        &["cost-analyst", "growth-analyst", "risk-reviewer"],
        2,
        2,
        3,
        "",
    );
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));

    let members = vec![
        member_entry(0x31, "cost-analyst", 1),
        member_entry(0x31, "growth-analyst", 1),
        member_entry(0x31, "risk-reviewer", 1),
    ];
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(f.team_id)),
        Some(body(&f, members)),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "超过 max_dispatch 必须 400：{v}"
    );
    let text = v.to_string();
    assert!(text.contains("max_dispatch"), "要点名是哪条限制：{text}");
    assert!(
        text.contains('3') && text.contains('2'),
        "要带实际值与上限：{text}"
    );
    assert!(text.contains("下一步"), "必须给下一步：{text}");
    assert_eq!(
        provider.call_count(),
        0,
        "🔴 超限的一轮不许调模型（那是一次真实的钱与时间开销）"
    );

    // 台账里也不许留下任何记录：被拒的一轮不是「记了没跑」。
    let (status, listed) = call(
        &app,
        "GET",
        &format!(
            "/api/teams/{}/dispatch?room_id={}&round=0",
            team_hex(f.team_id),
            f.room_id
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(
        listed["count"],
        serde_json::json!(0),
        "🔴 被拒的一轮不许在台账留下记录：{listed}"
    );
}

#[tokio::test]
async fn booking_a_round_over_the_team_max_dispatch_is_400_without_ledger_rows() {
    // book（只记账那条）与 run 走**同一道**闸门：不能出现「记账放行、执行拒绝」
    // 这种两边口径不一致的岔子。
    let t = TestDb::new("dispatch-book-over-max-dispatch");
    let f = seed_limits(
        &t.bridge(),
        user_id(),
        0x32,
        &["cost-analyst", "growth-analyst", "risk-reviewer"],
        2,
        2,
        3,
        "",
    );
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));

    let book_members = serde_json::json!([
        {"expert": "cost-analyst", "member": "cost-analyst-1",
         "member_session_id": member_session(0x32, &expert("cost-analyst")).to_compact_hex()},
        {"expert": "growth-analyst", "member": "growth-analyst-1",
         "member_session_id": member_session(0x32, &expert("growth-analyst")).to_compact_hex()},
        {"expert": "risk-reviewer", "member": "risk-reviewer-1",
         "member_session_id": member_session(0x32, &expert("risk-reviewer")).to_compact_hex()},
    ]);
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch", team_hex(f.team_id)),
        Some(body(&f, book_members.as_array().expect("数组").clone())),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "book 也必须过 max_dispatch 闸门：{v}"
    );
    assert!(v.to_string().contains("max_dispatch"), "{v}");

    let (_, listed) = call(
        &app,
        "GET",
        &format!(
            "/api/teams/{}/dispatch?room_id={}&round=0",
            team_hex(f.team_id),
            f.room_id
        ),
        None,
    )
    .await;
    assert_eq!(listed["count"], serde_json::json!(0), "{listed}");
}

#[tokio::test]
async fn a_round_beyond_the_team_max_replan_is_400_and_calls_no_model() {
    let t = TestDb::new("dispatch-run-over-max-replan");
    // max_replan = 0：只允许首派（round 0），这一轮用 round 1。
    let f = seed_limits(&t.bridge(), user_id(), 0x33, &["cost-analyst"], 4, 0, 3, "");
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));

    // 先证明 round 0 在同一个团上是放行的（否则「拒绝 round 1」可能只是因为别的原因）。
    let (status, first) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(f.team_id)),
        Some(body(&f, vec![member_entry(0x33, "cost-analyst", 1)])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "round 0 是首派，必须放行：{first}");
    assert_eq!(provider.call_count(), 1);

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(f.team_id)),
        Some(body_round(
            &f,
            1,
            vec![member_entry(0x33, "cost-analyst", 1)],
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "超过 max_replan 必须 400：{v}"
    );
    let text = v.to_string();
    assert!(text.contains("max_replan"), "要点名是哪条限制：{text}");
    assert!(text.contains("下一步"), "{text}");
    assert_eq!(
        provider.call_count(),
        1,
        "🔴 被拒的重规划轮不许调模型（第 1 轮不该有任何调用）"
    );
}

#[tokio::test]
async fn team_guidelines_reach_every_member_prompt() {
    let t = TestDb::new("dispatch-run-guidelines");
    let f = seed_limits(
        &t.bridge(),
        user_id(),
        0x34,
        &["cost-analyst", "growth-analyst"],
        4,
        2,
        3,
        "结论必须给出数据来源；不确定的地方要标注假设",
    );
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(f.team_id)),
        Some(body(
            &f,
            vec![
                member_entry(0x34, "cost-analyst", 1),
                member_entry(0x34, "growth-analyst", 1),
            ],
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "1 个成员在默认上限内，必须放行：{v}"
    );
    assert_eq!(v["guidelines_applied"], serde_json::json!(true), "{v}");
    assert_eq!(
        v["guidelines_chars"],
        serde_json::json!("结论必须给出数据来源；不确定的地方要标注假设"
            .chars()
            .count()),
        "{v}"
    );
    assert_eq!(
        v["team_limits"]["max_dispatch"],
        serde_json::json!(4),
        "{v}"
    );

    let prompts = provider.prompts();
    assert_eq!(prompts.len(), 2, "两个成员各调一次模型");
    for (system, user) in &prompts {
        let g_at = user
            .find("结论必须给出数据来源")
            .unwrap_or_else(|| panic!("🔴 团队准则没进成员提示：sys={system} user={user}"));
        let t_at = user
            .find("给出三点结论")
            .unwrap_or_else(|| panic!("任务正文不见了：{user}"));
        assert!(g_at < t_at, "准则必须排在任务正文之前：{user}");
    }
}

#[tokio::test]
async fn a_team_without_guidelines_keeps_the_member_prompt_clean() {
    let t = TestDb::new("dispatch-run-no-guidelines");
    let f = seed(&t.bridge(), user_id(), 0x35, &["cost-analyst"]);
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(f.team_id)),
        Some(body(&f, vec![member_entry(0x35, "cost-analyst", 1)])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["guidelines_applied"], serde_json::json!(false), "{v}");
    let prompts = provider.prompts();
    assert!(
        !prompts[0].1.contains("团队准则"),
        "没有准则时不许拼一个空标题：{}",
        prompts[0].1
    );
}

// ---------------------------------------------------------------- Q025：成员会话

/// 与 `member_entry` 同形，但 `member_session_id` 可以指定 ——「会话不在就建」
/// 与「指错了类型」两条用例都要一条 seed 没铺过的 id。
fn entry_with_sid(expert_name: &str, sid: &quill_adapters::SessionId) -> serde_json::Value {
    let e = expert(expert_name);
    let member = quill_adapters::MemberId::for_expert(&e, 1).expect("成员标识合法");
    serde_json::json!({
        "expert": expert_name,
        "member": member.as_str(),
        "title": format!("分析 {expert_name}"),
        "instructions": "给出三点结论",
        "member_session_id": sid.to_compact_hex(),
    })
}

/// book 的 members[] 只认「派给谁」——`title` / `instructions` 是 run 才认的字段，
/// 带上它们 `only_keys` 会 400。
fn book_entry(expert_name: &str, sid: &quill_adapters::SessionId) -> serde_json::Value {
    let mut v = entry_with_sid(expert_name, sid);
    let obj = v.as_object_mut().expect("对象");
    obj.remove("title");
    obj.remove("instructions");
    v
}

/// 铺一条 solo 会话（「指错了类型」用例需要一条**不是**成员会话的会话）。
fn seed_solo_session(
    db: &std::sync::Arc<quill_server::db::DbBridge>,
    owner: quill_domain::UserId,
    id: [u8; 16],
) {
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO sessions (user_id, id, kind, room_id, workspace_path, \
                 created_at, updated_at, last_active_at) \
                 VALUES (?, ?, 'solo', 'r-solo', '.quill-test-ws/solo', 0, 0, 0)",
            )
            .bind(owner.as_bytes().to_vec())
            .bind(id.to_vec())
            .execute(&pool)
            .await
            .map_err(|e| quill_server::db::storage_error("铺 solo 会话", e))?;
            Ok(())
        })
    })
    .unwrap_or_else(|e| panic!("铺 solo 会话失败：{e}"));
}

/// run 之后成员的「任务 + 产出」真的在它自己的会话里 —— 经真实 HTTP 路由读回，
/// 并钉住「写进去的正文与成员实际收到的 prompt 是同一份」。
#[tokio::test]
async fn a_round_writes_task_and_output_into_the_member_session() {
    let t = TestDb::new("dispatch-run-member-session");
    let f = seed(&t.bridge(), user_id(), 0x41, &["cost-analyst"]);
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));
    let sid = member_session(0x41, &expert("cost-analyst"));

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(f.team_id)),
        Some(body(&f, vec![member_entry(0x41, "cost-analyst", 1)])),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["member_messages_written"], serde_json::json!(1), "{v}");
    assert_eq!(v["member_persist_error"], serde_json::Value::Null, "{v}");

    let (status, msgs) = call(
        &app,
        "GET",
        &format!("/api/sessions/{}/messages", sid.to_compact_hex()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{msgs}");
    let msgs = msgs["messages"].as_array().expect("消息数组").clone();
    assert_eq!(msgs.len(), 2, "一轮成员回合是两条（任务 + 产出）：{msgs:?}");

    let seen = provider.prompts();
    assert_eq!(seen.len(), 1, "只有一个成员跑过");
    assert_eq!(msgs[0]["role"], serde_json::json!("user"), "{msgs:?}");
    assert_eq!(msgs[0]["seq"], serde_json::json!(1), "{msgs:?}");
    assert_eq!(
        msgs[0]["content"],
        serde_json::json!(seen[0].1),
        "会话里记的任务正文必须与成员实际收到的 prompt 是同一份：{msgs:?}"
    );
    assert_eq!(msgs[1]["role"], serde_json::json!("assistant"), "{msgs:?}");
    assert_eq!(msgs[1]["seq"], serde_json::json!(2), "{msgs:?}");
    assert_eq!(
        msgs[1]["content"],
        serde_json::json!("成员产出 #1"),
        "产出必须原样进它自己的会话：{msgs:?}"
    );
    assert_eq!(msgs[1]["status"], serde_json::json!("complete"), "{msgs:?}");
}

/// book 在记账之前把缺的成员会话建出来（外键要求它先在），且归属三项齐全 ——
/// 建出来的是真正的成员会话，不是随手塞一条空会话。
#[tokio::test]
async fn booking_creates_the_member_session_it_records() {
    let t = TestDb::new("dispatch-book-creates-member-session");
    let f = seed(&t.bridge(), user_id(), 0x42, &["cost-analyst"]);
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));
    // seed 铺的是 member_session(0x42, …)；这里换一个前缀 ——
    // 「缺则建」那条分支只有在这一步才真的走。
    let fresh = member_session(0x99, &expert("cost-analyst"));

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch", team_hex(f.team_id)),
        Some(body(&f, vec![book_entry("cost-analyst", &fresh)])),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "book 应 202：{v}");
    assert_eq!(provider.call_count(), 0, "book 只记账，不许调模型");

    // 结构：kind 与归属三项（schema 的 CHECK 只管「非空」，指向谁要这里验）。
    let db = t.bridge();
    let (kind, team, parent, expert_id): (String, String, String, String) = db
        .call({
            let uid = user_id().as_bytes().to_vec();
            let sid = fresh.as_bytes().to_vec();
            move |pool, _rt| {
                Box::pin(async move {
                    let row = sqlx::query_as::<_, (String, String, String, String)>(
                        "SELECT kind, hex(team_id), hex(parent_session_id), expert_id \
                         FROM sessions WHERE user_id = ? AND id = ?",
                    )
                    .bind(uid)
                    .bind(sid)
                    .fetch_one(&pool)
                    .await
                    .map_err(|e| quill_server::db::storage_error("读成员会话归属", e))?;
                    Ok(row)
                })
            }
        })
        .expect("读成员会话归属失败");
    assert_eq!(kind, "team_member", "建出来的必须是成员会话");
    assert_eq!(team.to_lowercase(), team_hex(f.team_id), "要指向本团");
    assert_eq!(
        parent.to_lowercase(),
        f.leader_session.to_compact_hex(),
        "要指回主持人会话"
    );
    assert_eq!(expert_id, "cost-analyst", "要记名册里那个专家");

    // 出口一：全量列表（团队页 / 用量页）看得见它。
    let (status, all) = call(&app, "GET", "/api/sessions?exclude_kind=team_leader", None).await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let top = all["sessions"]
        .as_array()
        .expect("会话数组")
        .iter()
        .find(|s| {
            s["id"]
                .as_str()
                .unwrap_or_default()
                .eq_ignore_ascii_case(&fresh.to_compact_hex())
        })
        .unwrap_or_else(|| panic!("新建的成员会话必须在全量列表里：{all}"));
    assert_eq!(top["kind"], serde_json::json!("team_member"), "{all}");
    assert_eq!(
        top["title"],
        serde_json::json!("cost-analyst"),
        "book 没有任务标题，会话标题退回专家名：{all}"
    );

    // 出口二：侧栏那条请求（前端实际发的参数）看不见它。
    let (status, hidden) = call(
        &app,
        "GET",
        "/api/sessions?exclude_kind=team_leader,team_member",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{hidden}");
    assert!(
        !hidden["sessions"]
            .as_array()
            .expect("会话数组")
            .iter()
            .any(|s| s["id"]
                .as_str()
                .unwrap_or_default()
                .eq_ignore_ascii_case(&fresh.to_compact_hex())),
        "成员会话不该出现在侧栏的会话列表里：{hidden}"
    );
}

/// `member_session_id` 指向一条**别的类型**的会话 → 400，一个成员都不跑、
/// 台账不留记录：把成员的消息写进用户的聊天会话是数据污染，不是「顺手复用」。
#[tokio::test]
async fn a_member_session_id_of_the_wrong_kind_is_400_and_runs_nobody() {
    let t = TestDb::new("dispatch-run-wrong-kind-member-session");
    let f = seed(&t.bridge(), user_id(), 0x43, &["cost-analyst"]);
    let provider = Arc::new(EchoProvider::new());
    let app = state(&t, Arc::clone(&provider));

    let mut solo = [0u8; 16];
    solo[0] = 0x44;
    seed_solo_session(&t.bridge(), user_id(), solo);
    let solo = quill_adapters::SessionId::from_bytes(solo);

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/teams/{}/dispatch/run", team_hex(f.team_id)),
        Some(body(&f, vec![entry_with_sid("cost-analyst", &solo)])),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "类型不对必须 400：{v}");
    assert_eq!(v["error"]["code"], serde_json::json!("bad_request"), "{v}");
    assert!(
        v.to_string().contains("不是团队成员的会话"),
        "要点名是类型不对：{v}"
    );
    assert_eq!(
        provider.call_count(),
        0,
        "🔴 会话类型不对就不许起成员（跑出来的产出没有可写的地方）"
    );

    let (_, listed) = call(
        &app,
        "GET",
        &format!(
            "/api/teams/{}/dispatch?room_id={}&round=0",
            team_hex(f.team_id),
            f.room_id
        ),
        None,
    )
    .await;
    assert_eq!(
        listed["count"],
        serde_json::json!(0),
        "被拒的一轮不许在台账留下记录：{listed}"
    );
}
