//! `GET /api/sessions/{id}/metrics` 的 HTTP 契约。
//!
//! 这条接口是会话级 Token 统计条（照 octop 的 `TrajectoryMetricsBar` 迁移）的
//! 唯一数据源，所以它的**空值语义**必须被钉死：
//! `null` = quill 没记这一项；把它变成 0 就是凭空造一个从没被测量的数字。
//!
//! 具体那套口径在 `quill_server::session_metrics` 的单测里逐条验，
//! 这里只验「HTTP 层真的把 null 原样透传出去，没有在某处塌成 0」。

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use quill_server::routes::build_router;
use quill_server::state::AppState;

mod common;
use common::TestDb;

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN_A: &str = "test-token-a";

fn user_id() -> quill_domain::UserId {
    quill_domain::UserId::parse(UID_A).expect("测试 UID 必须合法")
}

fn state(t: &TestDb) -> AppState {
    use quill_control::Pbkdf2Params;
    use quill_server::auth::{AuthContext, EnvTokenResolver};
    let resolver = EnvTokenResolver::new(vec![(
        TOKEN_A.to_string(),
        AuthContext {
            user_id: user_id(),
            is_admin: true,
        },
    )]);
    AppState {
        config: quill_server::Config::from_env(),
        tokens: Arc::new(resolver),
        db: Some(t.bridge()),
        db_problem: None,
        llm: Arc::new(std::sync::RwLock::new(None)),
        llm_config: Arc::new(std::sync::RwLock::new(Default::default())),
        providers: Arc::new(std::sync::RwLock::new(Default::default())),
        login_limiter: Arc::new(Default::default()),
        pbkdf2: Pbkdf2Params::for_tests(),
    }
}

fn req(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN_A}"))
        .body(Body::empty())
        .expect("构造请求失败")
}

async fn get_json(app: AppState, path: &str) -> (StatusCode, Value) {
    let resp = build_router(app)
        .oneshot(req("GET", path))
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("读响应体").to_bytes();
    let body = String::from_utf8(bytes.to_vec()).expect("响应体必须是 UTF-8");
    let v: Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("响应必须是 JSON（{e}）：{body}"));
    (status, v)
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
                .bind("统计测试用户")
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
        .oneshot({
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("authorization", format!("Bearer {TOKEN_A}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"title":"统计测试"}"#))
                .expect("构造请求失败")
        })
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK, "建会话应成功");
    let bytes = resp.into_body().collect().await.expect("读响应体").to_bytes();
    let v: Value = serde_json::from_slice(&bytes).expect("必须是 JSON");
    v["id"].as_str().expect("应当返回会话 id").to_string()
}

/// 直接铺消息行，省掉起一个模型端点。
fn seed_messages(t: &TestDb, sid: &str, rows: &[(&str, i64, i64, Option<i64>, Option<i64>)]) {
    let uid = user_id().as_bytes().to_vec();
    let sid_v = quill_domain::SessionId::parse(sid).expect("会话 id 合法").as_bytes().to_vec();
    // 先全部拷成自有值再进循环：下面的闭包是 `move` + `'static` 的，
    // 直接从 `rows` 里取引用会借出一个逃不出本函数的借用。
    let owned: Vec<(i64, String, i64, i64, Option<i64>, Option<i64>)> = rows
        .iter()
        .enumerate()
        .map(|(i, (role, input, output, turn_ms, cache_read))| {
            (
                i as i64 + 1,
                role.to_string(),
                *input,
                *output,
                *turn_ms,
                *cache_read,
            )
        })
        .collect();
    for (seq, role, input, output, turn_ms, cache_read) in owned {
        // 每轮都要自己的副本：闭包是 move 的，跨轮复用会被上一轮吃掉。
        let uid_c = uid.clone();
        let sid_c = sid_v.clone();
        // 消息 id 必须**跨会话唯一**（主键是 user_id+id）。
        // 早先这里直接用 vec![seq; 16]，一个测试铺两个会话就撞主键了。
        let mut mid = [0u8; 16];
        mid[..12].copy_from_slice(&sid_v[..12]);
        mid[12..].copy_from_slice(&(seq as u32).to_be_bytes());
        t.bridge()
            .call(move |pool, _rt| {
                Box::pin(async move {
                    sqlx::query(
                        "INSERT INTO messages(user_id,id,session_id,seq,role,status,content,\
                         input_tokens,output_tokens,cache_read_tokens,turn_ms,created_at) \
                         VALUES(?,?,?,?,?,'complete','',?,?,?,?,0)",
                    )
                    .bind(uid_c.clone())
                    .bind(mid.to_vec())
                    .bind(sid_c.clone())
                    .bind(seq)
                    .bind(&role)
                    .bind(input)
                    .bind(output)
                    .bind(cache_read)
                    .bind(turn_ms)
                    .execute(&pool)
                    .await
                    .map_err(|e| quill_server::db::storage_error("铺消息", e))?;
                    Ok(())
                })
            })
            .expect("铺消息失败");
    }
}

#[tokio::test]
async fn an_empty_session_reports_zero_turns_and_null_everything_else() {
    let t = TestDb::new("metrics-empty");
    seed_user(&t);
    let app = state(&t);
    let sid = create_session(app.clone()).await;

    let (status, v) = get_json(app, &format!("/api/sessions/{sid}/metrics")).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(v["turns"], serde_json::json!(0));
    assert_eq!(v["steps"], serde_json::json!(0));
    // 没消息 = 没有 token 数据。显示成 0 会让用户以为「这次聊天一点没花 token」。
    assert_eq!(v["input_tokens"], Value::Null, "没消息时入参必须是 null 不是 0");
    assert_eq!(v["output_tokens"], Value::Null);
    assert_eq!(v["llm_duration_ms"], Value::Null);
    assert_eq!(v["tok_per_s"], Value::Null);
    // 这两项 quill 压根不记录，恒为 null。
    assert_eq!(v["tool_duration_ms"], Value::Null);
    assert_eq!(v["ttft_avg_ms"], Value::Null);
    // 本地模型不报缓存 token 时，不能变成 0。
    assert_eq!(v["cache_hit_ratio"], Value::Null);
    assert_eq!(v["cache_read_tokens"], Value::Null);
}

#[tokio::test]
async fn turns_steps_tokens_and_speed_are_aggregated_from_real_rows() {
    let t = TestDb::new("metrics-agg");
    seed_user(&t);
    let app = state(&t);
    let sid = create_session(app.clone()).await;

    // 两轮对话：入参 1000+500，出参 100+50，模型耗时 2000+1000 毫秒。
    seed_messages(
        &t,
        &sid,
        &[
            ("user", 0, 0, None, None),
            ("assistant", 1000, 100, Some(2000), None),
            ("user", 0, 0, None, None),
            ("assistant", 500, 50, Some(1000), None),
        ],
    );

    let (status, v) = get_json(app, &format!("/api/sessions/{sid}/metrics")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["turns"], serde_json::json!(2));
    assert_eq!(v["steps"], serde_json::json!(2));
    assert_eq!(v["input_tokens"], serde_json::json!(1500));
    assert_eq!(v["output_tokens"], serde_json::json!(150));
    assert_eq!(v["llm_duration_ms"], serde_json::json!(3000));
    // 150 token / 3 秒
    assert_eq!(v["tok_per_s"], serde_json::json!(50.0));
}

#[tokio::test]
async fn a_reported_cache_hit_survives_the_round_trip_as_a_real_number() {
    let t = TestDb::new("metrics-cache");
    seed_user(&t);
    let app = state(&t);
    let sid = create_session(app.clone()).await;

    // cache_read 是 input 的子集（goose 的口径），所以入参仍是 1000 而不是 1900。
    seed_messages(
        &t,
        &sid,
        &[
            ("user", 0, 0, None, None),
            ("assistant", 1000, 20, Some(2000), Some(900)),
        ],
    );

    let (status, v) = get_json(app, &format!("/api/sessions/{sid}/metrics")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["cache_read_tokens"], serde_json::json!(900));
    assert_eq!(v["input_tokens"], serde_json::json!(1000), "缓存读不能重复计入入参");
    let ratio = v["cache_hit_ratio"].as_f64().expect("应当给出命中率");
    assert!((ratio - 0.9).abs() < 1e-6, "命中率应为 90%，实际 {ratio}");
}

#[tokio::test]
async fn a_null_cache_column_stays_null_all_the_way_to_the_response() {
    let t = TestDb::new("metrics-nocache");
    seed_user(&t);
    let app = state(&t);
    let sid = create_session(app.clone()).await;

    seed_messages(
        &t,
        &sid,
        &[
            ("user", 0, 0, None, None),
            ("assistant", 1000, 20, Some(2000), None),
        ],
    );

    let (status, v) = get_json(app, &format!("/api/sessions/{sid}/metrics")).await;
    assert_eq!(status, StatusCode::OK);
    // 这一条最容易被某次重构悄悄改成 0，然后界面显示「缓存命中 0.0%」。
    assert!(
        v["cache_read_tokens"].is_null(),
        "上游没上报时必须是 null，实际 {}",
        v["cache_read_tokens"]
    );
    assert!(v["cache_hit_ratio"].is_null());
}

#[tokio::test]
async fn a_missing_session_is_404_not_an_empty_statistics_object() {
    let t = TestDb::new("metrics-404");
    seed_user(&t);
    let app = state(&t);

    let (status, v) = get_json(app, "/api/sessions/0123456789ABCDEF0123456789ABCDEF/metrics").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "找不到会话不能回一份全 0 的统计");
    assert!(v.get("turns").is_none(), "错误信封里不该混进统计字段：{v}");
}

#[tokio::test]
async fn usage_lists_every_session_with_its_own_metrics() {
    let t = TestDb::new("usage-list");
    seed_user(&t);
    let app = state(&t);

    // 两个会话，一个有对话、一个没有。
    let a = create_session(app.clone()).await;
    seed_messages(
        &t,
        &a,
        &[
            ("user", 0, 0, None, None),
            ("assistant", 1000, 100, Some(2000), Some(900)),
        ],
    );
    let b = create_session(app.clone()).await;

    let (status, v) = get_json(app.clone(), "/api/usage").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["session_count"], serde_json::json!(2));
    assert_eq!(v["truncated"], serde_json::json!(false));
    assert_eq!(v["limit"], serde_json::json!(200));

    let list = v["sessions"].as_array().expect("sessions 应是数组");
    assert_eq!(list.len(), 2, "两个会话都要列出来，包括那个空的");

    // 找到有数据的那一行。
    let row = list
        .iter()
        .find(|s| s["id"] == serde_json::json!(a))
        .expect("应当能找到刚才那个会话");
    let m = &row["metrics"];
    assert_eq!(m["turns"], serde_json::json!(1));
    assert_eq!(m["input_tokens"], serde_json::json!(1000));
    assert_eq!(m["cache_read_tokens"], serde_json::json!(900));
    assert_eq!(m["tool_duration_ms"], Value::Null, "quill 不记录工具耗时");

    // 没有消息的会话：一行真实的 0 轮次，但 token 是 null。
    let empty = list
        .iter()
        .find(|s| s["id"] == serde_json::json!(b))
        .expect("空会话也要在列表里");
    assert_eq!(empty["metrics"]["turns"], serde_json::json!(0));
    assert_eq!(
        empty["metrics"]["input_tokens"],
        Value::Null,
        "没有消息就没有 token，显示 0 会让用户以为「这次聊天一点没花」"
    );
}

#[tokio::test]
async fn usage_totals_equal_the_sum_of_the_rows() {
    let t = TestDb::new("usage-totals");
    seed_user(&t);
    let app = state(&t);

    let a = create_session(app.clone()).await;
    seed_messages(
        &t,
        &a,
        &[
            ("user", 0, 0, None, None),
            ("assistant", 1000, 100, Some(2000), Some(400)),
        ],
    );
    let b = create_session(app.clone()).await;
    seed_messages(
        &t,
        &b,
        &[
            ("user", 0, 0, None, None),
            ("assistant", 500, 50, Some(1000), Some(100)),
        ],
    );

    let (_, v) = get_json(app, "/api/usage").await;
    let totals = &v["totals"];
    assert_eq!(totals["turns"], serde_json::json!(2));
    assert_eq!(totals["input_tokens"], serde_json::json!(1500));
    assert_eq!(totals["output_tokens"], serde_json::json!(150));
    assert_eq!(totals["llm_duration_ms"], serde_json::json!(3000));
    assert_eq!(totals["cache_read_tokens"], serde_json::json!(500));

    let rows: Vec<i64> = v["sessions"]
        .as_array()
        .expect("数组")
        .iter()
        .map(|s| s["metrics"]["input_tokens"].as_i64().expect("数字"))
        .collect();
    let sum: i64 = rows.iter().sum();
    assert_eq!(
        sum,
        totals["input_tokens"].as_i64().expect("数字"),
        "合计必须等于逐行相加，否则界面上两处数字会互相打架"
    );
}

#[tokio::test]
async fn usage_on_a_fresh_account_is_an_empty_report_not_a_zeroed_one() {
    let t = TestDb::new("usage-empty");
    seed_user(&t);
    let app = state(&t);

    let (status, v) = get_json(app, "/api/usage").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["session_count"], serde_json::json!(0));
    assert_eq!(v["sessions"].as_array().map(Vec::len), Some(0));
    assert_eq!(v["totals"]["input_tokens"], Value::Null, "没有任何数据就是 null");
    assert_eq!(v["totals"]["turns"], serde_json::json!(0), "轮次是 0，这是真值");
}