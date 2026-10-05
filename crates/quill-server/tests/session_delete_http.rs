//! DELETE /api/sessions/{id} 的契约：软删、幂等、跨用户隔离。
//!
//! 覆盖四件最容易悄悄坏掉的事：删完不再出现在列表里、二次删除幂等（不是 404）、
//! 别人的会话删不动、消息记录仍然保留（软删不是硬删）。

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

use common::{scalar_i64, TestDb};

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

async fn new_session(app: &AppState) -> String {
    let (status, v) = call(
        app,
        "POST",
        "/api/sessions",
        TOKEN_A,
        Some(serde_json::json!({ "title": "待删除的会话" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "建会话应成功：{v}");
    v["id"]
        .as_str()
        .expect("建会话回包必须带 id")
        .to_string()
}

fn fixture(label: &str) -> (TestDb, AppState) {
    let t = TestDb::new(label);
    seed_user(&t.bridge(), UID_A);
    seed_user(&t.bridge(), UID_B);
    let app = state(&t);
    (t, app)
}

#[tokio::test]
async fn deleting_a_session_soft_deletes_it_and_is_idempotent() {
    let (_t, app) = fixture("session-delete");
    let id = new_session(&app).await;

    let (status, v) = call(&app, "DELETE", &format!("/api/sessions/{id}"), TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK, "删除应 200：{v}");
    assert_eq!(v["id"], serde_json::json!(id));
    assert_eq!(v["deleted"], serde_json::json!(true));
    assert!(v["note"].as_str().is_some_and(|n| !n.is_empty()), "{v}");

    // 软删后：单读 404、列表里没有。
    let (status, v) = call(&app, "GET", &format!("/api/sessions/{id}"), TOKEN_A, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "删掉的会话必须 404：{v}");
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));
    assert!(
        v["error"]["next_step"].as_str().is_some(),
        "404 必须带下一步指引：{v}"
    );

    let (status, list) = call(&app, "GET", "/api/sessions", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = list["sessions"]
        .as_array()
        .expect("必须是数组")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&id.as_str()),
        "🔴 软删的会话不得再出现在列表里：{list}"
    );

    // 二次删除必须幂等：200 + deleted:false，而不是 404。
    let (status, v) = call(&app, "DELETE", &format!("/api/sessions/{id}"), TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK, "二次删除必须幂等 200：{v}");
    assert_eq!(v["deleted"], serde_json::json!(false));
    assert!(
        v["note"].as_str().is_some_and(|n| n.contains("幂等")),
        "幂等说明要写进 note：{v}"
    );
}

#[tokio::test]
async fn deleting_a_session_keeps_its_messages() {
    let (t, app) = fixture("session-delete-keeps-messages");
    let id = new_session(&app).await;

    // 直接铺一条消息，验证软删没有连带删掉消息（硬删会触发 CASCADE 丢数据）。
    let sid = quill_domain::SessionId::parse(&id)
        .expect("测试 id 必须合法")
        .as_bytes()
        .to_vec();
    let uid = user_id(UID_A).as_bytes().to_vec();
    t.bridge()
        .call({
            let sid = sid.clone();
            move |pool, _rt| {
                Box::pin(async move {
                    sqlx::query(
                        "INSERT INTO messages (user_id, id, session_id, seq, role, content, \
                         created_at) VALUES (?, x'02020202020202020202020202020202', ?, 1, \
                         'user', '你好', 0)",
                    )
                    .bind(uid)
                    .bind(sid)
                    .execute(&pool)
                    .await
                    .map_err(|e| storage_error("铺消息", e))?;
                    Ok(())
                })
            }
        })
        .expect("铺消息失败");

    let (status, _) = call(&app, "DELETE", &format!("/api/sessions/{id}"), TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        scalar_i64(&t.bridge(), "SELECT count(*) AS c FROM messages"),
        1,
        "🔴 软删不得连带丢消息（那是硬删的行为，且不可逆）"
    );
    assert_eq!(
        scalar_i64(&t.bridge(), "SELECT count(*) AS c FROM sessions"),
        1,
        "软删后这一行仍在库里（只是 deleted_at 不为空）"
    );
}

#[tokio::test]
async fn another_users_session_cannot_be_deleted() {
    let (_t, app) = fixture("session-delete-isolation");
    let id = new_session(&app).await;

    let (status, v) = call(&app, "DELETE", &format!("/api/sessions/{id}"), TOKEN_B, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "删别人的会话按「不存在」口径：{v}");
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));
    assert!(v.to_string().contains("下一步"), "{v}");

    // 属主自己的会话还在（别人的删除请求没碰它）。
    let (status, v) = call(&app, "GET", &format!("/api/sessions/{id}"), TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK, "我的会话不该被别人删掉：{v}");
}

#[tokio::test]
async fn deleting_an_unknown_or_malformed_session_is_reported_honestly() {
    let (_t, app) = fixture("session-delete-missing");

    let (status, v) = call(
        &app,
        "DELETE",
        "/api/sessions/0192b7c8-0000-7000-8000-00000000ffff",
        TOKEN_A,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));
    assert!(v.to_string().contains("下一步"), "{v}");

    let (status, v) = call(&app, "DELETE", "/api/sessions/not-an-id", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "非法 id 必须 400：{v}");
    assert!(v.to_string().contains("下一步"), "{v}");
}
