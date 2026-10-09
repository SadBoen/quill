//! 「不允许未选角色就聊天」这条规则的端到端验收。
//!
//! 过去的漏洞：`POST /api/sessions` 允许 `expert_id` 缺省，建出来的会话
//! `expert_id = null`，对话页把它归进「默认（未选角色）」分组 ——
//! 用户以为在跟某个角色说话，其实**没有任何人格在跑**，界面上也没说。

mod common;

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
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

/// 测试库必须先有这两行用户：`sessions.user_id` 对 `users` 有外键，
/// 没有用户行时建会话会被数据库拒掉（787 FOREIGN KEY constraint failed）。
///
/// 这条是踩过的：第一次写这个测试时忘了种用户，六条用例全挂在同一个外键错误上，
/// 看起来像是新功能坏了，其实是夹具没搭好。
fn seed_users(t: &TestDb) {
    let db = t.bridge();
    for (uid, name) in [(UID_A, "甲"), (UID_B, "乙")] {
        let sql = "INSERT INTO users (id, username, username_norm, display_name, \
                   password_hash, password_salt, password_algo, role, pwd_changed_at, \
                   created_at, updated_at) \
                   VALUES (?, ?, ?, ?, zeroblob(32), zeroblob(16), \
                   'pbkdf2-hmac-sha256$i=600000', 'owner', 0, 0, 0)"
            .to_string();
        let id: Vec<u8> = user_id(uid).as_bytes().to_vec();
        let uname = format!("u{uid}");
        db.call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(&sql)
                    .bind(id)
                    .bind(uname.clone())
                    .bind(uname)
                    .bind(name.to_string())
                    .execute(&pool)
                    .await
                    .map_err(|e| quill_server::db::storage_error("种测试用户", e))
            })
        })
        .unwrap_or_else(|e| panic!("种测试用户失败：{e}"));
    }
}

fn fixture(label: &str) -> TestDb {
    let t = TestDb::new(label);
    seed_users(&t);
    t
}

fn req(
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
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

async fn call(
    app: &AppState,
    method: &str,
    path: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> serde_json::Value {
    let resp = build_router(app.clone())
        .oneshot(req(method, path, Some(token), body))
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    let body_text = text(resp).await;
    let v: serde_json::Value = serde_json::from_str(&body_text)
        .unwrap_or_else(|e| panic!("响应必须是 JSON（{e}）：{body_text}"));
    assert!(
        status.is_success(),
        "{method} {path} 应成功，实际 {status}：{v}"
    );
    v
}

/// 没传 expert_id 时，会话必须已经挂上通用专家，而不是留空。
#[tokio::test]
async fn a_session_created_without_an_expert_lands_on_the_general_one() {
    let t = fixture("general-expert-default");
    let app = state(&t);

    let v = call(
        &app,
        "POST",
        "/api/sessions",
        TOKEN_A,
        Some(serde_json::json!({})),
    )
    .await;

    assert_eq!(
        v["expert_id"], "general",
        "没指定角色就建出来的会话必须挂通用专家，不能是 null：{v}"
    );
    assert_eq!(
        v["persona_applied"], true,
        "通用专家是有真人格的，persona_applied 必须为 true：{v}"
    );
    assert!(
        v["expert_notice"].is_null(),
        "挂上了就不该再有「没生效」的提示：{v}"
    );
}

/// 显式传空白等同于没传 —— 不能靠传 `""` 或 `"   "` 绕开规则。
#[tokio::test]
async fn an_explicitly_blank_expert_id_is_not_a_way_out() {
    let t = fixture("general-expert-blank");
    let app = state(&t);

    let v = call(
        &app,
        "POST",
        "/api/sessions",
        TOKEN_A,
        Some(serde_json::json!({ "expert_id": "   " })),
    )
    .await;

    assert_eq!(
        v["expert_id"], "general",
        "空白 expert_id 不该造出「未选角色」的会话：{v}"
    );
}

/// 显式指定别的专家时以调用方为准 —— 兜底不许反过来覆盖用户的选择。
#[tokio::test]
async fn an_explicitly_chosen_expert_is_respected() {
    let t = fixture("general-expert-explicit");
    let app = state(&t);

    call(
        &app,
        "POST",
        "/api/experts",
        TOKEN_A,
        Some(serde_json::json!({
            "id": "cost-analyst",
            "display_name": "成本分析师",
            "description": "算清本月成本"
        })),
    )
    .await;

    let v = call(
        &app,
        "POST",
        "/api/sessions",
        TOKEN_A,
        Some(serde_json::json!({ "expert_id": "cost-analyst" })),
    )
    .await;

    assert_eq!(
        v["expert_id"], "cost-analyst",
        "用户选了哪个角色就用哪个，兜底不许覆盖：{v}"
    );
}

/// 用户第一次用就是直接开对话，不必先去专家页点一次。
#[tokio::test]
async fn creating_a_session_also_materialises_the_general_expert() {
    let t = fixture("general-expert-materialise");
    let app = state(&t);

    call(
        &app,
        "POST",
        "/api/sessions",
        TOKEN_A,
        Some(serde_json::json!({})),
    )
    .await;

    let v = call(&app, "GET", "/api/experts", TOKEN_A, None).await;
    let list = v["experts"].as_array().expect("experts 必须是数组");
    assert_eq!(
        list.len(),
        1,
        "没建过任何专家的用户，建完会话后应当恰好有通用专家：{v}"
    );
    assert_eq!(list[0]["id"], "general");
}

/// ensure 是幂等的：反复建会话不产生第二份，也不覆盖用户改过的内容。
#[tokio::test]
async fn the_general_expert_is_not_duplicated_or_overwritten() {
    let t = fixture("general-expert-idempotent");
    let app = state(&t);

    for _ in 0..3 {
        call(
            &app,
            "POST",
            "/api/sessions",
            TOKEN_A,
            Some(serde_json::json!({})),
        )
        .await;
    }

    call(
        &app,
        "PATCH",
        "/api/experts/general",
        TOKEN_A,
        Some(serde_json::json!({ "instructions": "我改过的正文。" })),
    )
    .await;

    call(
        &app,
        "POST",
        "/api/sessions",
        TOKEN_A,
        Some(serde_json::json!({})),
    )
    .await;

    let v = call(&app, "GET", "/api/experts/general", TOKEN_A, None).await;
    assert_eq!(
        v["instructions"], "我改过的正文。",
        "用户改过通用专家之后，后续的自动补齐不许覆盖它：{v}"
    );

    let v = call(&app, "GET", "/api/experts", TOKEN_A, None).await;
    assert_eq!(
        v["experts"].as_array().expect("数组").len(),
        1,
        "反复建会话不该把通用专家建出第二份：{v}"
    );
}

/// 通用专家是**这个用户**的，不是全局共享的。
/// 这类自动补齐最常见的错就是漏掉 user_id，于是 A 改了 B 也跟着变。
#[tokio::test]
async fn each_user_gets_their_own_general_expert() {
    let t = fixture("general-expert-isolation");
    let app = state(&t);

    call(
        &app,
        "POST",
        "/api/sessions",
        TOKEN_A,
        Some(serde_json::json!({})),
    )
    .await;
    call(
        &app,
        "PATCH",
        "/api/experts/general",
        TOKEN_A,
        Some(serde_json::json!({ "instructions": "A 的正文" })),
    )
    .await;

    call(
        &app,
        "POST",
        "/api/sessions",
        TOKEN_B,
        Some(serde_json::json!({})),
    )
    .await;

    let v = call(&app, "GET", "/api/experts/general", TOKEN_B, None).await;
    assert_ne!(
        v["instructions"], "A 的正文",
        "B 不该看到 A 改过的通用专家 —— 自动补齐必须按 user_id 隔离：{v}"
    );
}

/// `StatusCode` 只在断言里用到，显式引一次免得 unused 警告。
#[allow(dead_code)]
const _STATUS_CODE_IN_USE: StatusCode = StatusCode::OK;
