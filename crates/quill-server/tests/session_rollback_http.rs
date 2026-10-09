//! `POST /api/sessions/{id}/rollback` 的契约：真回滚、计数跟着变、跨会话/跨用户隔离、
//! 非法入参被拒（queue Q021）。
//!
//! **为什么单独一个文件**：这是 quill 第一条**会真删消息**的路由。它最容易悄悄坏掉的
//! 地方不是「删不删」，而是**删多删少**：`seq >= 边界` 写成 `>` 就会把被回滚的那一条
//! 留下（用户以为回到了某轮之前，其实那一轮还在）；漏了 `session_id`/`user_id` 谓词
//! 就会删到别人的会话。这两类错在界面上都看不出来，所以在这里逐条钉住。

mod common;

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::db::{storage_error, DbBridge};
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const UID_B: &str = "0192b7c8-0000-7000-8000-000000000002";
const TOKEN_A: &str = "tok-a";
const TOKEN_B: &str = "tok-b";

fn user_id(uid: &str) -> quill_domain::UserId {
    quill_domain::UserId::parse(uid).expect("测试 UID 必须合法")
}

fn state(t: &TestDb) -> AppState {
    let resolver = EnvTokenResolver::new(vec![
        (
            TOKEN_A.to_string(),
            AuthContext {
                user_id: user_id(UID_A),
                is_admin: true,
            },
        ),
        (
            TOKEN_B.to_string(),
            AuthContext {
                user_id: user_id(UID_B),
                is_admin: false,
            },
        ),
    ]);
    AppState {
        config: Config::from_env(),
        tokens: Arc::new(resolver),
        db: Some(t.bridge()),
        db_problem: None,
        llm: Arc::new(RwLock::new(None)),
        llm_config: Arc::new(RwLock::new(Default::default())),
        providers: Arc::new(RwLock::new(Default::default())),
        login_limiter: Arc::new(Default::default()),
        member_control: Default::default(),
        pbkdf2: quill_control::Pbkdf2Params::for_tests(),
    }
}

fn seed_user(db: &Arc<DbBridge>, uid: &str) {
    let id = user_id(uid).as_bytes().to_vec();
    let name = uid.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "INSERT OR IGNORE INTO users (id, username, username_norm, display_name, \
                 password_hash, password_salt, password_algo, role, pwd_changed_at, \
                 created_at, updated_at) \
                 VALUES (?,?,?,?,zeroblob(32),zeroblob(16),'pbkdf2-hmac-sha256$i=600000',\
                 'owner',0,0,0)",
            )
            .bind(id)
            .bind(&name)
            .bind(&name)
            .bind("测试用户")
            .execute(&pool)
            .await
            .map_err(|e| storage_error("铺测试用户", e))?;
            Ok(())
        })
    })
    .expect("铺测试用户失败");
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
    token: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
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

async fn new_session(app: &AppState, title: &str) -> String {
    let (status, v) = call(
        app,
        "POST",
        "/api/sessions",
        TOKEN_A,
        Some(serde_json::json!({ "title": title })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "建会话应成功：{v}");
    v["id"].as_str().expect("建会话回包必须带 id").to_string()
}

/// 一条测试消息的 16 字节 id。**必须带上会话盐** —— 主键是 `(user_id, id)`，
/// 两个会话用同一套 id 会撞 `UNIQUE`。
fn msg_id(salt: u8, seq: u8) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[0] = salt;
    id[1] = seq;
    id
}

fn hex16(id: &[u8; 16]) -> String {
    quill_adapters::to_hex_upper(id)
}

/// 直接铺 `count` 条消息（一问一答交替），返回它们的 hex id 列表。
/// **不走 HTTP 发消息**：那要真调模型；这里测的是回滚，不是对话。
/// `salt` 让同一个用例里的两个会话拿到互不相同的消息 id。
fn seed_messages(db: &Arc<DbBridge>, uid: &str, sid_hex: &str, salt: u8, count: u8) -> Vec<String> {
    let uid = user_id(uid).as_bytes().to_vec();
    let sid = quill_domain::SessionId::parse(sid_hex)
        .expect("会话 id 必须合法")
        .as_bytes()
        .to_vec();
    let mut ids = Vec::new();
    for seq in 1..=count {
        let id = msg_id(salt, seq);
        ids.push(hex16(&id));
        let role = if seq % 2 == 1 { "user" } else { "assistant" };
        let content = format!("第 {seq} 条");
        let sid2 = sid.clone();
        let uid2 = uid.clone();
        db.call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(
                    "INSERT INTO messages(user_id,id,session_id,seq,role,status,content,created_at) \
                     VALUES(?,?,?,?,?,'complete',?,?)",
                )
                .bind(uid2)
                .bind(id.to_vec())
                .bind(sid2)
                .bind(i64::from(seq))
                .bind(role)
                .bind(content)
                .bind(i64::from(seq))
                .execute(&pool)
                .await
                .map_err(|e| storage_error("铺测试消息", e))?;
                Ok(())
            })
        })
        .expect("铺测试消息失败");
    }
    ids
}

async fn list_seqs(app: &AppState, sid: &str) -> Vec<i64> {
    let (status, v) = call(
        app,
        "GET",
        &format!("/api/sessions/{sid}/messages"),
        TOKEN_A,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "列消息应成功：{v}");
    v["messages"]
        .as_array()
        .expect("必须是数组")
        .iter()
        .map(|m| m["seq"].as_i64().expect("seq 必须是整数"))
        .collect()
}

async fn message_count(app: &AppState, sid: &str) -> i64 {
    let (status, list) = call(app, "GET", "/api/sessions", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK);
    list["sessions"]
        .as_array()
        .expect("必须是数组")
        .iter()
        .find(|s| s["id"] == serde_json::json!(sid))
        .map(|s| s["message_count"].as_i64().expect("message_count 是整数"))
        .unwrap_or_else(|| panic!("会话 {sid} 不在列表里：{list}"))
}

fn fixture(label: &str) -> (TestDb, AppState) {
    let t = TestDb::new(label);
    seed_user(&t.bridge(), UID_A);
    seed_user(&t.bridge(), UID_B);
    let app = state(&t);
    (t, app)
}

#[tokio::test]
async fn rollback_deletes_the_boundary_message_and_everything_after() {
    let (t, app) = fixture("session-rollback");
    let id = new_session(&app, "回滚").await;
    let ids = seed_messages(&t.bridge(), UID_A, &id, 1, 5);
    assert_eq!(list_seqs(&app, &id).await, vec![1, 2, 3, 4, 5]);

    // 回滚到第 3 条**之前** → 第 3、4、5 条都没了，剩 1、2。
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/sessions/{id}/rollback"),
        TOKEN_A,
        Some(serde_json::json!({ "message_id": ids[2] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "回滚应 200：{v}");
    assert_eq!(
        v["deleted"],
        serde_json::json!(3),
        "应删掉含边界在内的 3 条：{v}"
    );
    assert_eq!(
        list_seqs(&app, &id).await,
        vec![1, 2],
        "🔴 边界那一条也必须删掉（goose 的 `seq >= 边界` 口径）——留一条就是把语义做错了"
    );
}

#[tokio::test]
async fn rollback_rebases_the_session_message_count() {
    let (t, app) = fixture("session-rollback-count");
    let id = new_session(&app, "计数").await;
    let ids = seed_messages(&t.bridge(), UID_A, &id, 1, 4);

    // 会话列表读的是 `sessions.message_count`（存着的列），所以先把它摆成 4
    // —— 真实路径里它是 `touch_session` 一轮 +2 累出来的，这里直接对齐。
    let sid = quill_domain::SessionId::parse(&id)
        .expect("id 合法")
        .as_bytes()
        .to_vec();
    let db = t.bridge();
    {
        let sid = sid.clone();
        db.call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query("UPDATE sessions SET message_count = 4 WHERE id = ?")
                    .bind(sid)
                    .execute(&pool)
                    .await
                    .map_err(|e| storage_error("摆计数", e))?;
                Ok(())
            })
        })
        .expect("摆计数失败");
    }
    assert_eq!(message_count(&app, &id).await, 4);

    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/sessions/{id}/rollback"),
        TOKEN_A,
        Some(serde_json::json!({ "message_id": ids[1] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        message_count(&app, &id).await,
        1,
        "🔴 回滚后 message_count 必须按剩下的消息重算 —— 不重算侧栏会一直显示 4"
    );
}

#[tokio::test]
async fn rollback_to_the_first_message_empties_the_conversation() {
    let (t, app) = fixture("session-rollback-empty");
    let id = new_session(&app, "清空").await;
    let ids = seed_messages(&t.bridge(), UID_A, &id, 1, 3);

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/sessions/{id}/rollback"),
        TOKEN_A,
        Some(serde_json::json!({ "message_id": ids[0] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["deleted"], serde_json::json!(3));
    assert!(
        list_seqs(&app, &id).await.is_empty(),
        "回到第一条之前 = 会话空了"
    );

    // 空会话还能继续写：`next_seq` 是 `MAX(seq)+1` 现算的，回滚不会把它卡住。
    seed_messages(&t.bridge(), UID_A, &id, 2, 1);
    assert_eq!(list_seqs(&app, &id).await, vec![1]);
}

#[tokio::test]
async fn a_message_from_another_session_is_a_404_and_deletes_nothing() {
    let (t, app) = fixture("session-rollback-wrong-session");
    let a = new_session(&app, "甲").await;
    let b = new_session(&app, "乙").await;
    seed_messages(&t.bridge(), UID_A, &a, 1, 3);
    let b_ids = seed_messages(&t.bridge(), UID_A, &b, 2, 3);

    // 拿乙会话的消息去回滚甲会话：边界在甲里找不到 → 一条都不许删。
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/sessions/{a}/rollback"),
        TOKEN_A,
        Some(serde_json::json!({ "message_id": b_ids[1] })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "别的会话的消息按「不在本会话」口径报 404：{v}"
    );
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));
    assert!(v["error"]["next_step"].is_string(), "404 必须带下一步：{v}");

    assert_eq!(
        list_seqs(&app, &a).await,
        vec![1, 2, 3],
        "甲会话一条都不该少"
    );
    assert_eq!(list_seqs(&app, &b).await, vec![1, 2, 3], "乙会话也不该被动");
}

#[tokio::test]
async fn another_users_session_cannot_be_rolled_back() {
    let (t, app) = fixture("session-rollback-isolation");
    let id = new_session(&app, "我的").await;
    let ids = seed_messages(&t.bridge(), UID_A, &id, 1, 3);

    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/sessions/{id}/rollback"),
        TOKEN_B,
        Some(serde_json::json!({ "message_id": ids[0] })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "别人的会话按「不存在」口径：{v}"
    );
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));

    assert_eq!(
        list_seqs(&app, &id).await,
        vec![1, 2, 3],
        "🔴 跨用户请求不许删到属主的消息"
    );
}

#[tokio::test]
async fn rollback_requires_an_explicit_valid_message_id() {
    let (t, app) = fixture("session-rollback-validation");
    let id = new_session(&app, "校验").await;
    seed_messages(&t.bridge(), UID_A, &id, 1, 3);

    // 缺 message_id：拒（没有「默认删到某处」这种入口）。
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/sessions/{id}/rollback"),
        TOKEN_A,
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "缺 message_id 必须 400：{v}"
    );
    assert!(v["error"]["next_step"].is_string(), "{v}");

    // 非法 id：拒。
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/sessions/{id}/rollback"),
        TOKEN_A,
        Some(serde_json::json!({ "message_id": "not-a-hex-id" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "非法 message_id 必须 400：{v}"
    );
    assert!(v["error"]["next_step"].is_string(), "{v}");

    // 未知字段：拒（免得前端以为传了什么都行）。
    let (status, v) = call(
        &app,
        "POST",
        &format!("/api/sessions/{id}/rollback"),
        TOKEN_A,
        Some(serde_json::json!({ "message_id": hex16(&msg_id(0, 1)), "seq": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "未知字段必须 400：{v}");

    // 非法会话 id：400。
    let (status, v) = call(
        &app,
        "POST",
        "/api/sessions/not-an-id/rollback",
        TOKEN_A,
        Some(serde_json::json!({ "message_id": hex16(&msg_id(0, 1)) })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "非法会话 id 必须 400：{v}");

    // 一路被拒之后，会话内容一条没动。
    assert_eq!(list_seqs(&app, &id).await, vec![1, 2, 3]);
}
