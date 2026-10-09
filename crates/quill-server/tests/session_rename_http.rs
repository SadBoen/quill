//! `PATCH /api/sessions/{id}` 的契约：真改名、跨用户隔离、空名被拒、软删的改不动。
//!
//! **为什么单独一个文件**：这条路由此前只挂 GET/DELETE，PATCH 落进 405 —— 界面上
//! 「重命名」没有任何后端出口（queue Q051）。这里钉住四件最容易悄悄坏掉的事：
//! 改名真的落库且列表能看到、**`last_active_at` 不动**（动了侧栏排序会凭空跳）、
//! 别人的会话改不动、空标题被拒而超长标题是截断不是报错。

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

fn fixture(label: &str) -> (TestDb, AppState) {
    let t = TestDb::new(label);
    seed_user(&t.bridge(), UID_A);
    seed_user(&t.bridge(), UID_B);
    let app = state(&t);
    (t, app)
}

#[tokio::test]
async fn renaming_round_trips_and_the_list_shows_the_new_title() {
    let (_t, app) = fixture("session-rename");
    let id = new_session(&app, "旧名字").await;

    let (status, v) = call(
        &app,
        "PATCH",
        &format!("/api/sessions/{id}"),
        TOKEN_A,
        Some(serde_json::json!({ "title": "新名字" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "改名应 200（此前是 405）：{v}");
    assert_eq!(v["title"], serde_json::json!("新名字"));
    assert_eq!(v["renamed"], serde_json::json!(true));

    // `GET /api/sessions/{id}` 只是**存在性探针**（回 `{id, found:true}`，不含标题），
    // 所以「改名真落库」要看列表 —— 侧栏读的也正是列表的 `title`。
    let (status, v) = call(&app, "GET", &format!("/api/sessions/{id}"), TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["found"], serde_json::json!(true));

    // 列表里是新名字（侧栏读的就是这个字段）。
    let (status, list) = call(&app, "GET", "/api/sessions", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK);
    let titles: Vec<&str> = list["sessions"]
        .as_array()
        .expect("必须是数组")
        .iter()
        .filter_map(|r| r["title"].as_str())
        .collect();
    assert!(
        titles.contains(&"新名字"),
        "改名必须真落库（列表里看到新名字）：{list}"
    );
    assert!(!titles.contains(&"旧名字"), "旧名字不该还留着：{list}");
}

#[tokio::test]
async fn renaming_does_not_touch_last_active_at_so_the_sidebar_does_not_jump() {
    let (t, app) = fixture("session-rename-active-at");
    let id = new_session(&app, "排序测试").await;

    let sid = quill_domain::SessionId::parse(&id)
        .expect("测试 id 必须合法")
        .as_bytes()
        .to_vec();
    let before = scalar_i64(
        &t.bridge(),
        &format!(
            "SELECT last_active_at AS c FROM sessions WHERE id = x'{}'",
            sid.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ),
    );

    let (status, _) = call(
        &app,
        "PATCH",
        &format!("/api/sessions/{id}"),
        TOKEN_A,
        Some(serde_json::json!({ "title": "改完不该跳" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let after = scalar_i64(
        &t.bridge(),
        &format!(
            "SELECT last_active_at AS c FROM sessions WHERE id = x'{}'",
            sid.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ),
    );
    assert_eq!(
        before, after,
        "🔴 改名不是「用过它」：刷新 last_active_at 会让会话在侧栏里凭空跳到最上面"
    );
}

#[tokio::test]
async fn another_users_session_cannot_be_renamed() {
    let (_t, app) = fixture("session-rename-isolation");
    let id = new_session(&app, "我的会话").await;

    let (status, v) = call(
        &app,
        "PATCH",
        &format!("/api/sessions/{id}"),
        TOKEN_B,
        Some(serde_json::json!({ "title": "被改掉" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "改别人的会话按「不存在」口径：{v}"
    );
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));
    assert!(
        v["error"]["next_step"].is_string(),
        "404 必须带下一步指引：{v}"
    );

    // 属主的标题没被别人的请求改掉（看列表，理由同前：单读不含标题）。
    let (status, list) = call(&app, "GET", "/api/sessions", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK);
    let titles: Vec<&str> = list["sessions"]
        .as_array()
        .expect("必须是数组")
        .iter()
        .filter_map(|r| r["title"].as_str())
        .collect();
    assert!(
        titles.contains(&"我的会话"),
        "别人的改名请求不许生效：{list}"
    );
}

#[tokio::test]
async fn a_blank_title_is_rejected_but_a_long_one_is_truncated() {
    let (_t, app) = fixture("session-rename-validation");
    let id = new_session(&app, "校验").await;

    // 空白标题：拒。侧栏里一行没有名字的会话等于让人认不出来。
    let (status, v) = call(
        &app,
        "PATCH",
        &format!("/api/sessions/{id}"),
        TOKEN_A,
        Some(serde_json::json!({ "title": "   " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "空白标题必须 400：{v}");
    assert!(
        v["error"]["next_step"].is_string(),
        "400 必须带下一步指引：{v}"
    );

    // 超长标题：按**字符**截到 64，不是报错，也不是按字节截（按字节会把中文截成半个字）。
    let long = "字".repeat(70);
    let (status, v) = call(
        &app,
        "PATCH",
        &format!("/api/sessions/{id}"),
        TOKEN_A,
        Some(serde_json::json!({ "title": long })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "超长标题应截断而不是拒绝：{v}");
    assert_eq!(
        v["title"]
            .as_str()
            .expect("title 必须是字符串")
            .chars()
            .count(),
        64,
        "截断口径必须与建会话一致（64 个字符）：{v}"
    );

    // 未知字段：拒（免得前端以为传了什么都能被悄悄忽略）。
    let (status, v) = call(
        &app,
        "PATCH",
        &format!("/api/sessions/{id}"),
        TOKEN_A,
        Some(serde_json::json!({ "title": "好名字", "kind": "solo" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "未知字段必须 400：{v}");
}

#[tokio::test]
async fn a_soft_deleted_or_malformed_session_is_reported_honestly() {
    let (_t, app) = fixture("session-rename-soft-deleted");
    let id = new_session(&app, "删了再改").await;

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("/api/sessions/{id}"),
        TOKEN_A,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, v) = call(
        &app,
        "PATCH",
        &format!("/api/sessions/{id}"),
        TOKEN_A,
        Some(serde_json::json!({ "title": "改不动" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "软删的会话改不动（与读口径一致）：{v}"
    );
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));

    let (status, v) = call(
        &app,
        "PATCH",
        "/api/sessions/not-an-id",
        TOKEN_A,
        Some(serde_json::json!({ "title": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "非法 id 必须 400：{v}");
    assert!(v["error"]["next_step"].is_string(), "{v}");
}
