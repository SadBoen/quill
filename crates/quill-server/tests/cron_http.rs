//! 定时任务的 HTTP 与调度端到端证据（queue Q042）。
//!
//! 判据不是「路由返回 200」，而是四条：
//!   - 增删改查**真落库**，且线上的时间字段是前端能 `new Date()` 解析的 ISO 串；
//!   - 不支持的排期（cron 表达式）与过去的时刻**如实拒绝**，不猜成别的排期；
//!   - 调度器**真会投递**：到点的任务被送进它自己的会话，用户消息与模型回复都落了库；
//!   - 投递失败**不静默**：记下原因并改到 5 分钟后重试，而不是「跳过一次」。

mod common;

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_provider::{
    BoxFuture, ChatRequest, ChatResponse, FinishReason, ModelInfo, Provider, ProviderStream,
    TokenUsage,
};
use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::cron_repo::{self, CronJobRow, Schedule};
use quill_server::cron_scheduler;
use quill_server::db::now_ms;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN_A: &str = "tok-a";

#[derive(Debug)]
struct EchoProvider;

impl Provider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
        Box::pin(async move {
            Ok(ChatResponse {
                id: None,
                model: request.model.clone(),
                text: "定时任务跑完了。".to_string(),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish_reason: Some(FinishReason::Stop),
                usage: TokenUsage::new(Some(5), Some(5)),
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
        Box::pin(async {
            Err(quill_provider::ProviderError::Status {
                code: 501,
                body: "定时任务不走流式".to_string(),
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

/// 铺一条用户行。`cron_jobs.user_id` 与 `sessions.user_id` 都有指向 `users` 的外键，
/// 没有它连任务都塞不进去（这正是外键该有的样子）。
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
                .map_err(|e| quill_server::db::storage_error("铺用户", e))?;
                Ok(())
            })
        })
        .expect("铺用户失败");
}

/// 建一个测试实例。`with_model=false` 时**没有 provider** —— 用来测投递失败那条路。
fn state(t: &TestDb, with_model: bool) -> AppState {
    seed_user(t);
    let resolver = EnvTokenResolver::new(vec![(
        TOKEN_A.to_string(),
        AuthContext {
            user_id: user_id(),
            is_admin: true,
        },
    )]);
    let llm: Option<quill_provider::SharedProvider> = if with_model {
        Some(Arc::new(EchoProvider) as quill_provider::SharedProvider)
    } else {
        None
    };
    AppState {
        config: Config::from_env(),
        tokens: Arc::new(resolver),
        db: Some(t.bridge()),
        db_problem: None,
        llm: Arc::new(RwLock::new(llm)),
        llm_config: Arc::new(RwLock::new(Default::default())),
        providers: Arc::new(RwLock::new(Default::default())),
        login_limiter: Arc::new(Default::default()),
        member_control: Default::default(),
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

async fn call(
    app: AppState,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let resp = build_router(app)
        .oneshot(req(method, path, body))
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    (status, json(&text(resp).await))
}

/// 直接往库里塞一条**已经到点**的任务 —— 走 API 是做不到的（`every` 最小 60 秒、
/// `at` 不许是过去），而调度器要测的正是「到点了会怎样」。
fn seed_due_job(t: &TestDb, id: &str, schedule: Schedule, next_fire_at: i64) -> CronJobRow {
    let row = CronJobRow {
        id: id.to_string(),
        name: format!("任务 {id}"),
        message: "到点该说的话。".to_string(),
        schedule,
        tz: String::new(),
        session_id: None,
        last_fired_at: None,
        next_fire_at,
        fired_count: 0,
        last_error: None,
    };
    cron_repo::put(t.bridge().as_ref(), user_id(), row.clone(), now_ms()).expect("塞任务");
    row
}

/// 某个会话里的消息（role + content），按 seq 升序。
fn messages(t: &TestDb, sid_hex: &str) -> Vec<(String, String)> {
    let sid: Vec<u8> = (0..16)
        .map(|i| u8::from_str_radix(&sid_hex[i * 2..i * 2 + 2], 16).expect("hex"))
        .collect();
    let uid = user_id().as_bytes().to_vec();
    t.bridge()
        .call(move |pool, _rt| {
            Box::pin(async move {
                let rows = sqlx::query(
                    "SELECT role, content FROM messages WHERE user_id = ? AND session_id = ? \
                     ORDER BY seq ASC",
                )
                .bind(uid)
                .bind(sid)
                .fetch_all(&pool)
                .await
                .map_err(|e| quill_server::db::storage_error("读消息", e))?;
                Ok(rows
                    .into_iter()
                    .map(|r| {
                        (
                            sqlx::Row::get::<String, _>(&r, "role"),
                            sqlx::Row::get::<String, _>(&r, "content"),
                        )
                    })
                    .collect::<Vec<_>>())
            })
        })
        .expect("读消息失败")
}

fn every_body() -> serde_json::Value {
    serde_json::json!({
        "name": "每小时提醒",
        "message": "该做小结了。",
        "schedule": { "type": "every", "every_seconds": 3600 },
    })
}

#[tokio::test]
async fn creating_a_job_persists_it_and_the_list_shows_it() {
    let t = TestDb::new("cron-create");
    let app = state(&t, false);

    let (status, created) = call(app.clone(), "POST", "/api/cron", Some(every_body())).await;
    assert_eq!(status, StatusCode::CREATED, "新建应当 201：{created}");
    assert_eq!(created["name"], "每小时提醒");
    assert_eq!(created["message"], "该做小结了。");
    assert_eq!(created["schedule"]["type"], "every");
    assert_eq!(created["schedule"]["every_seconds"], 3600);
    assert_eq!(
        created["session_id"],
        serde_json::Value::Null,
        "还没投递过就没有会话"
    );
    assert_eq!(created["last_fired_at"], serde_json::Value::Null);
    // **线上是 ISO 串**（前端 `new Date(value)` 要能解析）—— 不是 epoch 数字。
    let next = created["next_fire_at"]
        .as_str()
        .expect("next_fire_at 必须是字符串");
    assert!(
        chrono::DateTime::parse_from_rfc3339(next).is_ok(),
        "next_fire_at 必须是 RFC3339，实际 {next:?}"
    );
    let id = created["id"].as_str().expect("要有 id").to_string();

    let (status, listed) = call(app.clone(), "GET", "/api/cron", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        listed["items"].as_array().map(Vec::len),
        Some(1),
        "{listed}"
    );
    assert_eq!(listed["items"][0]["id"], id.as_str());
    assert_eq!(listed["next_offset"], serde_json::Value::Null, "只有一页");

    let (status, one) = call(app.clone(), "GET", &format!("/api/cron/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["message"], "该做小结了。");

    // 改：PATCH 收的是完整 body，改完名字与排期都变。
    let mut patched_body = every_body();
    patched_body["name"] = serde_json::json!("改成两小时");
    patched_body["schedule"]["every_seconds"] = serde_json::json!(7200);
    let (status, patched) = call(
        app.clone(),
        "PATCH",
        &format!("/api/cron/{id}"),
        Some(patched_body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["name"], "改成两小时");
    assert_eq!(patched["schedule"]["every_seconds"], 7200);

    // 删：删完列表里就没有了，再读是 404。
    let (status, _) = call(app.clone(), "DELETE", &format!("/api/cron/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let (_status, listed) = call(app.clone(), "GET", "/api/cron", None).await;
    assert_eq!(
        listed["items"].as_array().map(Vec::len),
        Some(0),
        "{listed}"
    );
    let (status, _) = call(app, "GET", &format!("/api/cron/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_unsupported_schedule_and_a_past_moment_are_refused_with_a_next_step() {
    let t = TestDb::new("cron-refuse");
    let app = state(&t, false);

    // cron 表达式：明确拒绝（而不是猜成 every）。
    let body = serde_json::json!({
        "name": "工作日九点",
        "message": "早会。",
        "schedule": { "type": "cron", "cron_expr": "0 9 * * 1-5", "tz": "Asia/Shanghai" },
    });
    let (status, refused) = call(app.clone(), "POST", "/api/cron", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let detail = refused["error"]["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("cron 表达式"),
        "要点名是哪一种排期不支持：{detail}"
    );
    assert!(
        refused["error"]["next_step"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "拒绝必须带下一步：{refused}"
    );

    // 过去的时刻：一次性任务给一个已经过去的时间 → 拒绝（否则它会立刻触发或永不触发）。
    let body = serde_json::json!({
        "name": "昨天的提醒",
        "message": "晚了。",
        "schedule": { "type": "at", "at": "2020-01-01T00:00:00Z", "tz": "UTC" },
    });
    let (status, refused) = call(app.clone(), "POST", "/api/cron", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["error"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("已经过去"),
        "{refused}"
    );

    // 间隔越界（小于 60 秒）：拒绝，且说清区间。
    let mut too_fast = every_body();
    too_fast["schedule"]["every_seconds"] = serde_json::json!(5);
    let (status, refused) = call(app.clone(), "POST", "/api/cron", Some(too_fast)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    // 一条都不许落库。
    let (_, listed) = call(app, "GET", "/api/cron", None).await;
    assert_eq!(
        listed["items"].as_array().map(Vec::len),
        Some(0),
        "{listed}"
    );
}

#[tokio::test]
async fn the_scheduler_really_delivers_a_due_job_into_its_session() {
    let t = TestDb::new("cron-deliver");
    let app = state(&t, true);
    let now = now_ms();
    seed_due_job(
        &t,
        "job1",
        Schedule::Every {
            every_seconds: 3600,
        },
        now - 1000,
    );

    let delivered = cron_scheduler::run_due_once(&app, now)
        .await
        .expect("跑一轮");
    let after = cron_repo::get(t.bridge().as_ref(), user_id(), "job1".to_string())
        .expect("读任务")
        .expect("任务还在");
    assert_eq!(
        delivered, 1,
        "到点的任务应当被投递；调度器记下的失败原因是：{:?}",
        after.last_error
    );

    // 任务被绑到了一条会话上，而且**那次投递真的进了会话**（用户消息 + 模型回复）。
    let job = cron_repo::get(t.bridge().as_ref(), user_id(), "job1".to_string())
        .expect("读任务")
        .expect("任务还在");
    let sid = job.session_id.expect("投递后必须有会话").to_compact_hex();
    let msgs = messages(&t, &sid);
    assert_eq!(msgs.len(), 2, "应当有一条用户消息与一条助手回复：{msgs:?}");
    assert_eq!(msgs[0], ("user".to_string(), "到点该说的话。".to_string()));
    assert_eq!(
        msgs[1],
        ("assistant".to_string(), "定时任务跑完了。".to_string())
    );

    // 记账：fired 一次、下次触发推进到 now + 间隔、没有残留错误。
    assert_eq!(job.fired_count, 1);
    assert_eq!(job.last_fired_at, Some(now));
    assert_eq!(job.next_fire_at, now + 3600 * 1000);
    assert_eq!(job.last_error, None);

    // 再跑一轮：还没到点，不该重复投递（否则每次 tick 都会重发一遍）。
    let again = cron_scheduler::run_due_once(&app, now)
        .await
        .expect("再跑一轮");
    assert_eq!(again, 0, "没到点的任务不许重复投递");
    assert_eq!(messages(&t, &sid).len(), 2);
}

#[tokio::test]
async fn a_one_shot_job_is_removed_after_it_fires() {
    let t = TestDb::new("cron-one-shot");
    let app = state(&t, true);
    let now = now_ms();
    seed_due_job(&t, "once", Schedule::At { run_at: now - 500 }, now - 500);

    assert_eq!(
        cron_scheduler::run_due_once(&app, now)
            .await
            .expect("跑一轮"),
        1
    );
    // 一次性任务投递完就软删：列表里不该再有它（记录还在库里，供排障）。
    assert!(
        cron_repo::get(t.bridge().as_ref(), user_id(), "once".to_string())
            .expect("读")
            .is_none(),
        "一次性任务投递后应当从列表消失"
    );
}

#[tokio::test]
async fn a_failed_delivery_is_recorded_and_rescheduled_not_dropped() {
    let t = TestDb::new("cron-delivery-fails");
    // **没有 provider** —— 投递必然失败。
    let app = state(&t, false);
    let now = now_ms();
    seed_due_job(
        &t,
        "job-fail",
        Schedule::Every {
            every_seconds: 3600,
        },
        now - 1000,
    );

    let delivered = cron_scheduler::run_due_once(&app, now)
        .await
        .expect("跑一轮");
    assert_eq!(delivered, 0, "投递失败不该算成投递成功");

    let job = cron_repo::get(t.bridge().as_ref(), user_id(), "job-fail".to_string())
        .expect("读任务")
        .expect("失败的任务不许被丢掉");
    assert_eq!(job.fired_count, 0, "失败不算跑过");
    assert!(
        job.last_error.as_deref().is_some_and(|e| !e.is_empty()),
        "失败原因必须记下来，否则用户只看到「它没跑」"
    );
    assert_eq!(
        job.next_fire_at,
        now + cron_repo::FAILURE_RETRY_MS,
        "失败后应当 5 分钟后再试，而不是推进到正常的下一次"
    );
}
