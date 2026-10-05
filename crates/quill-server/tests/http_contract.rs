mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_server::auth::EnvTokenResolver;
use quill_server::config::Config;
use quill_server::routes::{build_router, CONTRACT_ROUTES, EXTRA_ROUTES};
use quill_server::state::AppState;

use common::TestDb;

const UID_ADMIN: &str = "0192b7c8-0000-7000-8000-000000000001";
const UID_PLAIN: &str = "0192b7c8-0000-7000-8000-000000000002";
const TOKEN_ADMIN: &str = "tok-admin";
const TOKEN_PLAIN: &str = "tok-plain";

fn state_with(db: Option<&TestDb>) -> AppState {
    let resolver = EnvTokenResolver::new(vec![
        (
            TOKEN_ADMIN.to_string(),
            quill_server::auth::AuthContext {
                user_id: quill_domain::UserId::parse(UID_ADMIN).expect("测试 UID 必须合法"),
                is_admin: true,
            },
        ),
        (
            TOKEN_PLAIN.to_string(),
            quill_server::auth::AuthContext {
                user_id: quill_domain::UserId::parse(UID_PLAIN).expect("测试 UID 必须合法"),
                is_admin: false,
            },
        ),
    ]);
    AppState {
        config: Config::from_env(),
        tokens: std::sync::Arc::new(resolver),
        db: db.map(TestDb::bridge),

        db_problem: if db.is_some() {
            None
        } else {
            Some("测试注入：数据库不可用".to_string())
        },
    }
}

fn state() -> AppState {
    state_with(Some(leak_db()))
}

fn leak_db() -> &'static TestDb {
    use std::sync::OnceLock;
    static DB: OnceLock<TestDb> = OnceLock::new();
    DB.get_or_init(|| TestDb::new("http-contract"))
}

fn state_with_selftest(on: bool) -> AppState {
    let mut s = state();
    s.config.enable_selftest = on;
    s
}

macro_rules! app {
    () => {
        build_router(state())
    };
}

async fn body_text(resp: axum::response::Response) -> String {
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("读响应体")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("响应体必须是 UTF-8")
}

fn req(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .expect("构造请求失败")
}

fn authed(method: &str, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("构造请求失败")
}

#[tokio::test]
async fn healthz_returns_200_without_auth() {
    let resp = app!()
        .oneshot(req("GET", "/healthz"))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK, "/healthz 必须免鉴权可达");
    let text = body_text(resp).await;
    assert!(text.contains("\"status\":\"ok\""), "响应体应含状态：{text}");
}

#[tokio::test]
async fn auth_me_returns_200_with_resolved_identity() {
    let resp = app!()
        .oneshot(authed("GET", "/api/auth/me", TOKEN_ADMIN))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let text = body_text(resp).await;
    let admin_uid = quill_domain::UserId::parse(UID_ADMIN)
        .expect("合法")
        .to_compact_hex();
    assert!(
        text.contains(&admin_uid),
        "响应应回显解析出的 UserId：{text}"
    );
    assert!(
        text.contains("\"is_admin\":true"),
        "admin 标记应落上：{text}"
    );

    assert!(
        text.contains("\"approval_mode\":\"manual\""),
        "必须声明默认 manual 审批：{text}"
    );
}

#[tokio::test]
async fn version_returns_200_with_version_field() {
    let resp = app!()
        .oneshot(authed("GET", "/api/version", TOKEN_PLAIN))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let text = body_text(resp).await;
    assert!(text.contains("version"), "应返回版本号：{text}");
    assert!(!text.contains(".md"), "不应再指向任何已删除的文档：{text}");
}

#[tokio::test]
async fn missing_token_is_401_not_404_or_500() {
    let resp = app!()
        .oneshot(req("GET", "/api/sessions"))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers()
            .get(axum::http::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok()),
        Some("Bearer"),
        "401 必须带 WWW-Authenticate: Bearer"
    );
}

#[tokio::test]
async fn three_auth_failure_modes_produce_byte_identical_responses() {
    let missing = app!()
        .oneshot(req("GET", "/api/sessions"))
        .await
        .expect("失败");
    let wrong = app!()
        .oneshot(authed("GET", "/api/sessions", "no-such-token"))
        .await
        .expect("失败");
    let malformed = app!()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/sessions")
                .header("authorization", TOKEN_ADMIN)
                .body(Body::empty())
                .expect("构造失败"),
        )
        .await
        .expect("失败");

    let (a, b, c) = (
        body_text(missing).await,
        body_text(wrong).await,
        body_text(malformed).await,
    );
    assert_eq!(
        a, b,
        "缺头与错令牌的响应体必须逐字节相同\n缺头：{a}\n错令牌：{b}"
    );
    assert_eq!(b, c, "错令牌与格式错的响应体必须逐字节相同\n格式错：{c}");
}

#[tokio::test]
async fn unauth_body_is_chinese_with_actionable_next_step() {
    let resp = app!()
        .oneshot(req("GET", "/api/sessions"))
        .await
        .expect("失败");
    let text = body_text(resp).await;
    assert!(text.contains("next_step"), "错误体必须带下一步：{text}");
    assert!(
        text.contains("Authorization"),
        "下一步应告知如何携带令牌：{text}"
    );
}

#[tokio::test]
async fn unregistered_path_is_404_with_chinese_body() {
    let resp = app!()
        .oneshot(authed("GET", "/api/definitely-not-a-route", TOKEN_ADMIN))
        .await
        .expect("失败");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let text = body_text(resp).await;
    assert!(
        text.contains("not_found"),
        "错误码应可被前端分支处理：{text}"
    );
    assert!(text.contains("next_step"), "404 也必须自诊断：{text}");
}

#[tokio::test]
async fn extra_routes_list_does_not_leak_into_404_claim() {
    for &(_, path) in EXTRA_ROUTES {
        let concrete = path.replace("{path}", "a/b").replace("{name}", "n");
        let resp = app!().oneshot(req("GET", &concrete)).await.expect("失败");
        assert_ne!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "EXTRA_ROUTES 声明的 {path} 实际应存在（401/501 也算存在）"
        );
    }
}

#[tokio::test]
async fn registered_but_unimplemented_route_returns_501_with_route_name() {
    let resp = app!()
        .oneshot(authed("GET", "/api/sessions", TOKEN_ADMIN))
        .await
        .expect("失败");
    assert_eq!(
        resp.status(),
        StatusCode::NOT_IMPLEMENTED,
        "已登记未实现的路由必须是 501，不许返回假成功 200"
    );
    let text = body_text(resp).await;
    assert!(
        text.contains("not_implemented"),
        "错误码应可被前端分支：{text}"
    );
    assert!(
        text.contains("/api/sessions"),
        "501 文案必须点名具体路由（否则不可诊断）：{text}"
    );
}

#[tokio::test]
async fn unimplemented_route_still_requires_auth_first() {
    let resp = app!()
        .oneshot(req("GET", "/api/sessions"))
        .await
        .expect("失败");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn every_contract_route_responds_and_is_never_a_false_success() {
    const IMPLEMENTED: [(&str, &str); 8] = [
        ("GET", "/api/version"),
        ("GET", "/api/auth/me"),
        ("GET", "/api/healthz"),
        ("GET", "/api/experts"),
        ("POST", "/api/experts"),
        ("GET", "/api/experts/{slug}"),
        ("PATCH", "/api/experts/{slug}"),
        ("DELETE", "/api/experts/{slug}"),
    ];

    for &(method, path) in CONTRACT_ROUTES {
        let concrete = path
            .replace("{id}", "0192b7c8-0000-7000-8000-000000000003")
            .replace("{slug}", "cost-analyst")
            .replace("{name}", "some-mcp")
            .replace("{path}", "a/b");

        let resp = app!()
            .oneshot(authed(method, &concrete, TOKEN_ADMIN))
            .await
            .unwrap_or_else(|e| panic!("{method} {concrete} oneshot 失败：{e}"));

        let status = resp.status();

        let body = body_text(resp).await;
        if status == StatusCode::NOT_FOUND {
            assert!(
                body.contains("entity_not_found"),
                "契约路由 {method} {concrete} 返回 404 且错误码是 not_found：\
                 说明路由**没登记**，而不是资源不存在：{body}"
            );
        }

        let implemented = IMPLEMENTED.iter().any(|(m, p)| *m == method && *p == path);
        if implemented {
            assert_ne!(
                status,
                StatusCode::NOT_IMPLEMENTED,
                "契约路由 {method} {concrete} 已实现，不得退回 501"
            );
        } else {
            assert_eq!(
                status,
                StatusCode::NOT_IMPLEMENTED,
                "契约路由 {method} {concrete} 未实现，必须返回 501 而非 {status}（假成功）"
            );
        }
    }
}

#[tokio::test]
async fn dispatch_routes_are_registered_and_answer_in_chinese() {
    for (method, declared, concrete) in [
        (
            "GET",
            "/api/teams/{id}/dispatch",
            "/api/teams/0192b7c8-0000-7000-8000-000000000003/dispatch?room_id=r&round=0",
        ),
        (
            "POST",
            "/api/teams/{id}/dispatch",
            "/api/teams/0192b7c8-0000-7000-8000-000000000003/dispatch",
        ),
        ("GET", "/api/dispatch/inflight", "/api/dispatch/inflight"),
    ] {
        assert!(
            EXTRA_ROUTES.contains(&(method, declared)),
            "{method} {declared} 必须在 EXTRA_ROUTES 里显式登记（豁免必须登记）"
        );
        let resp = app!()
            .oneshot(authed(method, concrete, TOKEN_ADMIN))
            .await
            .unwrap_or_else(|e| panic!("{method} {concrete} oneshot 失败：{e}"));
        let status = resp.status();
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {concrete} 必须已注册（404 说明 handler 没挂上）"
        );
        let t = body_text(resp).await;

        if !status.is_success() {
            assert!(
                t.contains("next_step"),
                "失败响应必须自诊断（{method} {concrete}，{status}）：{t}"
            );
        }
    }
}

#[tokio::test]
async fn wrong_method_on_existing_path_is_405_not_404() {
    let resp = app!()
        .oneshot(authed("PUT", "/api/sessions", TOKEN_ADMIN))
        .await
        .expect("失败");
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    let text = body_text(resp).await;
    assert!(text.contains("method_not_allowed"), "错误码应明确：{text}");
}

#[tokio::test]
async fn panic_in_handler_becomes_500_and_leaks_nothing_internally() {
    let resp = build_router(state_with_selftest(true))
        .oneshot(req("GET", "/__selftest__/panic"))
        .await
        .expect("oneshot 失败");

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "panic 必须被转成 500，而不是让连接任务崩掉"
    );

    let has_request_id = resp.headers().contains_key("x-quill-request-id");
    let text = body_text(resp).await;
    assert!(
        !text.contains("自检用内部信息"),
        "内部细节泄漏到响应体了：{text}"
    );
    assert!(
        !text.contains("panicked at"),
        "panic 位置/栈泄漏到响应体了：{text}"
    );
    assert!(text.contains("next_step"), "500 也必须自诊断：{text}");
    assert!(
        has_request_id,
        "500 必须带请求 ID，用户才能在日志里定位（铁律四）"
    );
}

#[tokio::test]
async fn selftest_panic_route_is_absent_unless_explicitly_enabled() {
    let resp = app!()
        .oneshot(req("GET", "/__selftest__/panic"))
        .await
        .expect("失败");
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "自检 panic 路由默认必须不存在（否则是自我 DoS 开关）"
    );
}

#[tokio::test]
async fn successful_responses_also_carry_request_id_for_traceability() {
    let resp = app!().oneshot(req("GET", "/healthz")).await.expect("失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let id = resp
        .headers()
        .get("x-quill-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    assert!(id.is_some(), "所有响应都应带请求 ID");
    assert!(id.unwrap().starts_with('r'));
}

#[tokio::test]
async fn healthz_surfaces_startup_warnings_so_silent_fallback_is_impossible() {
    let bad = AppState {
        config: {
            let mut c = Config::from_env();
            c.web_dir = std::path::PathBuf::from("/definitely/not/here");
            c
        },
        tokens: state().tokens,
        db: state().db,
        db_problem: None,
    };
    let resp = build_router(bad)
        .oneshot(req("GET", "/healthz"))
        .await
        .expect("失败");

    assert_eq!(resp.status(), StatusCode::OK);
    let text = body_text(resp).await;
    assert!(
        text.contains("\"ui_assets_available\":false"),
        "必须显式报出前端产物不可用：{text}"
    );
}
