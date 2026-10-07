//! `/api/channels` 的契约。
//!
//! **最重要的一条是凭据绝不外泄**：`GET /api/channels` 会把每个通道的
//! 完整配置读出来往外送，而微信的 `accounts[].token` 是长期凭据。
//! 一旦它进了响应，任何一个列表请求就等于把 token 交出去。
//! `store::to_public` 是唯一出口，本文件钉的就是「它只挑非密字段」。
//!
//! 第二个重点是 `dm_policy` 的默认值：`allowlist` 而**不是** `open`。
//! 配好通道就让陌生人能跟智能体说话，这个故障比「机器人不理我」
//! 难发现得多（表现为「怎么有人找上门」），所以默认必须关着。
//!
//! 扫码那两步不进真 iLink（会打真实网络），只验请求体校验与路由挂线。

mod common;

use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{json, Value};
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

/// 错误响应的文案在 `error.detail` 里（信封形状，见 `ApiError`）。
/// 直接读 `b["detail"]` 永远读到 null —— 断言会莫名其妙地失败，
/// 而那是**判据自己读错了**，不是被测行为有问题。
fn detail_of(body: &Value) -> Option<&str> {
    body.pointer("/error/detail").and_then(Value::as_str)
}

async fn call(
    app: axum::Router,
    token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if !token.is_empty() {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let req = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.oneshot(req).await.expect("请求应得到响应");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

/// 建一条带凭据的微信通道。
async fn make_weixin(app: &axum::Router, token: &str) -> Value {
    let (s, b) = call(
        app.clone(),
        token,
        "POST",
        "/api/channels",
        Some(json!({
            "kind": "weixin",
            "name": "我的微信",
            "enabled": false,
            "config": {
                "dm_policy": "allowlist",
                "accounts": [{
                    "account_id": "acc-1@im.bot",
                    "account_name": "小王",
                    "token": "SUPER-SECRET-TOKEN",
                    "base_url": "https://ilinkai.weixin.qq.com",
                }],
            },
        })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "建通道失败：{b}");
    b
}

#[tokio::test]
async fn create_returns_200_without_echoing_the_token() {
    let t = TestDb::new("ch-create");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    let b = make_weixin(&app, TOKEN_A).await;

    let raw = b.to_string();
    assert!(
        !raw.contains("SUPER-SECRET-TOKEN"),
        "创建响应里回显了凭据：{raw}"
    );
    assert_eq!(b["kind"], "weixin");
    assert_eq!(b["configured"], true, "应如实说「已配置」");
    assert_eq!(b["accounts"][0]["account_id"], "acc-1@im.bot");
    assert_eq!(b["dm_policy"], "allowlist");
}

#[tokio::test]
async fn list_never_returns_the_token() {
    let t = TestDb::new("ch-list");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    make_weixin(&app, TOKEN_A).await;

    let (s, b) = call(app, TOKEN_A, "GET", "/api/channels", None).await;
    assert_eq!(s, StatusCode::OK);
    let raw = b.to_string();
    assert!(
        !raw.contains("SUPER-SECRET-TOKEN"),
        "列表把凭据端出去了：{raw}"
    );
    // 也不能靠 config 字段绕过去
    assert!(b["channels"][0].get("config").is_none());
}

#[tokio::test]
async fn get_one_never_returns_the_token_either() {
    let t = TestDb::new("ch-get");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    make_weixin(&app, TOKEN_A).await;

    let (s, b) = call(app, TOKEN_A, "GET", "/api/channels/weixin-main", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(!b.to_string().contains("SUPER-SECRET-TOKEN"));
}

#[tokio::test]
async fn channels_are_scoped_to_their_owner() {
    let t = TestDb::new("ch-scope");
    seed_user(&t.bridge(), UID_A);
    seed_user(&t.bridge(), UID_B);
    let app = build_router(state(&t));
    make_weixin(&app, TOKEN_A).await;

    let (_, b) = call(app.clone(), TOKEN_A, "GET", "/api/channels", None).await;
    assert_eq!(b["channels"].as_array().unwrap().len(), 1);

    let (_, b) = call(app.clone(), TOKEN_B, "GET", "/api/channels", None).await;
    assert_eq!(
        b["channels"].as_array().unwrap().len(),
        0,
        "B 不该看到 A 的通道"
    );

    // 直接按 id 取也取不到
    let (s, _) = call(app, TOKEN_B, "GET", "/api/channels/weixin-main", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_kind_is_rejected_with_the_supported_list() {
    let t = TestDb::new("ch-kind");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    let (s, b) = call(
        app,
        TOKEN_A,
        "POST",
        "/api/channels",
        Some(json!({ "kind": "telegram" })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let detail = detail_of(&b).unwrap_or_default();
    assert!(
        detail.contains("weixin"),
        "报错要点名本实例支持什么：{detail}"
    );
}

#[tokio::test]
async fn bad_dm_policy_is_rejected() {
    let t = TestDb::new("ch-policy");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    let (s, b) = call(
        app,
        TOKEN_A,
        "POST",
        "/api/channels",
        Some(json!({
            "kind": "weixin",
            "config": { "dm_policy": "everyone" },
        })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let detail = detail_of(&b).unwrap_or_default();
    assert!(detail.contains("allowlist"), "报错要列出合法取值：{detail}");
}

#[tokio::test]
async fn omitted_policy_defaults_to_allowlist_not_open() {
    let t = TestDb::new("ch-default");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    let (s, b) = call(
        app,
        TOKEN_A,
        "POST",
        "/api/channels",
        Some(json!({ "kind": "weixin", "config": {} })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        b["dm_policy"], "allowlist",
        "默认必须是不放行任何人，不是 open"
    );
    assert_eq!(b["configured"], false);
}

#[tokio::test]
async fn partial_update_keeps_the_token() {
    // 只改名字不该把凭据抹掉 —— 抹掉的表现是「机器人突然不理我了」
    let t = TestDb::new("ch-partial");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    make_weixin(&app, TOKEN_A).await;

    let (s, b) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/channels",
        Some(json!({ "kind": "weixin", "name": "改名了" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["name"], "改名了");
    assert_eq!(b["configured"], true, "只改名字不该弄丢凭据");
}

#[tokio::test]
async fn delete_removes_the_channel() {
    let t = TestDb::new("ch-del");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    make_weixin(&app, TOKEN_A).await;

    let (s, _) = call(
        app.clone(),
        TOKEN_A,
        "DELETE",
        "/api/channels/weixin-main",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (_, b) = call(app, TOKEN_A, "GET", "/api/channels", None).await;
    assert_eq!(b["channels"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn one_kind_per_owner_is_enforced() {
    // 表上有 UNIQUE(owner_user_id, kind)：同名 kind 只能有一条。
    // 不然前端存不住选中哪条，重复行在列表里也看不出区别。
    let t = TestDb::new("ch-unique");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    make_weixin(&app, TOKEN_A).await;

    let (s, b) = call(
        app,
        TOKEN_A,
        "POST",
        "/api/channels",
        Some(json!({ "kind": "weixin", "channel_id": "weixin-2" })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "同一 kind 不该能有第二条；且冲突要报 409 而不是 503 存储不可用"
    );
    let detail = detail_of(&b).unwrap_or_default();
    assert!(
        detail.contains("weixin"),
        "报错要点名是哪一条冲突：{detail}"
    );
}

#[tokio::test]
async fn qr_poll_rejects_a_missing_token() {
    // 不打真网络：只验请求体校验。真扫码要人拿手机扫，判据里验不了。
    let t = TestDb::new("ch-qr");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    let (s, b) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/channels/weixin/qrcode/poll",
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(
        detail_of(&b).unwrap_or_default().contains("qrcode_token"),
        "要说清缺哪个字段：{b}"
    );
}

#[tokio::test]
async fn qr_poll_rejects_unknown_keys() {
    let t = TestDb::new("ch-qr-extra");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    let (s, _) = call(
        app,
        TOKEN_A,
        "POST",
        "/api/channels/weixin/qrcode/poll",
        Some(json!({ "qrcode_token": "q", "evil": 1 })),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "未知字段要报错 —— 拼错的字段名会静默丢掉"
    );
}

#[tokio::test]
async fn channel_routes_require_a_token() {
    let t = TestDb::new("ch-auth");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));
    let (s, _) = call(app, "", "GET", "/api/channels", None).await;
    assert_ne!(s, StatusCode::OK, "没令牌不该能列通道");
}
