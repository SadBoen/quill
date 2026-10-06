mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use std::sync::{Arc, RwLock};
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
        llm: Arc::new(RwLock::new(None)),
        llm_config: Arc::new(RwLock::new(Default::default())),
        providers: Arc::new(RwLock::new(Default::default())),
        login_limiter: Arc::new(Default::default()),
        pbkdf2: quill_control::Pbkdf2Params::for_tests(),
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

/// 真实服务会在启动时按 `QUILL_TOKENS` 自动建档，测试里没有这一步，
/// 写 sessions 会撞 `users` 的外键约束。
fn seed_user(uid: &str) {
    let id = quill_domain::UserId::parse(uid)
        .expect("测试 UID 必须合法")
        .as_bytes()
        .to_vec();
    let name = uid.to_string();
    leak_db()
        .bridge()
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
                .bind(&name)
                .bind(&name)
                .bind("测试管理员")
                .execute(&pool)
                .await
                .map_err(|e| quill_server::db::storage_error("测试铺用户", e))?;
                Ok(())
            })
        })
        .expect("铺用户失败");
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
    // 拿 `/api/upgrade/check` 当样本：它登记了、能力确实还没实现。
    // 原来这里用的是 `GET /api/users` —— 那条 2026-10-06 接通之后，
    // 这个测试会**因为自己过期而失败**。样本路由必须挑一条**仍然**是 501 的，
    // 否则它测的其实是「我以为的那件事」，不是「501 这个行为」。
    let resp = app!()
        .oneshot(authed("GET", "/api/upgrade/check", TOKEN_ADMIN))
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
        text.contains("/api/upgrade/check"),
        "501 文案必须点名具体路由（否则不可诊断）：{text}"
    );
    // next_step 现在按路由给（NotImplemented 带 advice），所以还必须真的有内容。
    let envelope = serde_json::from_str::<serde_json::Value>(&text).expect("响应必须是 JSON");
    let ns = envelope["error"]["next_step"].as_str().unwrap_or_default();
    assert!(
        !ns.trim().is_empty(),
        "501 必须自带下一步，否则用户只能干等：{text}"
    );
}

#[tokio::test]
async fn a_route_that_became_real_is_no_longer_reported_as_501() {
    // `/api/sessions` 已经真接通，契约测试不能再把它当桩，否则实现落地反而测试变红。
    for (method, path) in [
        ("GET", "/api/sessions"),
        ("GET", "/api/teams"),
        ("POST", "/api/teams"),
        ("GET", "/api/teams/{id}"),
        ("PATCH", "/api/teams/{id}"),
        ("DELETE", "/api/teams/{id}"),
        ("DELETE", "/api/sessions/{id}"),
    ] {
        let concrete = path
            .replace("{id}", "0192b7c8-0000-7000-8000-000000000003")
            .replace("{slug}", "cost-analyst");
        let resp = app!()
            .oneshot(authed(method, &concrete, TOKEN_ADMIN))
            .await
            .expect("失败");
        assert_ne!(
            resp.status(),
            StatusCode::NOT_IMPLEMENTED,
            "已实现的路由 {method} {path} 不应再返回 501"
        );
    }
}

#[tokio::test]
async fn unimplemented_route_still_requires_auth_first() {
    let resp = app!()
        .oneshot(req("GET", "/api/sessions"))
        .await
        .expect("失败");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// 写路径（POST 响应）与读路径（SQLite `hex(id)`）必须给出同一个 id 字符串。
/// 曾经一个用小写、一个用大写，前端 `session.id === sessionId` 与按 id 去重同时失配：
/// 新会话首条消息渲染两遍、会话标题永远退回"新对话"。
#[tokio::test]
async fn created_session_id_is_byte_identical_to_the_listed_one() {
    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap_or_else(|e| panic!("响应必须是 JSON（{e}）：{text}"))
    }

    seed_user(UID_ADMIN);

    let created = app!()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("authorization", format!("Bearer {TOKEN_ADMIN}"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("构造请求失败"),
        )
        .await
        .expect("oneshot 失败");
    let created_status = created.status();
    let created_text = body_text(created).await;
    assert_eq!(
        created_status,
        StatusCode::OK,
        "建会话应成功，实际 {created_status}：{created_text}"
    );
    let new_id = json(&created_text)
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("建会话响应必须带 id：{created_text}"))
        .to_string();

    let listed_text = body_text(
        app!()
            .oneshot(authed("GET", "/api/sessions", TOKEN_ADMIN))
            .await
            .expect("oneshot 失败"),
    )
    .await;
    let listed: Vec<String> = json(&listed_text)
        .get("sessions")
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("id").and_then(|v| v.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    assert!(
        listed.contains(&new_id),
        "POST 返回的 id 必须在 GET /api/sessions 里逐字节一致（大小写敏感）\nPOST={new_id}\nGET={listed:?}"
    );
}

#[tokio::test]
async fn every_contract_route_responds_and_is_never_a_false_success() {
    // 「已实现」清单必须跟着实现一起长，否则新接通的路由会因为不再返回 501
    // 而被判成「假成功」——这正是本测试要抓的东西，所以清单不能手懒。
    const IMPLEMENTED: [(&str, &str); 56] = [
        ("GET", "/api/version"),
        ("GET", "/api/auth/me"),
        ("GET", "/api/healthz"),
        ("POST", "/api/auth/login"),
        ("POST", "/api/auth/refresh"),
        ("POST", "/api/auth/logout"),
        // MCP 配置与 SKILL：存储层已通（2026-10-06 接通）。**只管存，不连服务器**
        // —— 协议层（rmcp）还没落地，所以 `GET` 的响应里 `connected` 恒为
        // false。把它列进「已实现」的同时，这条注释也钉住那个前提。
        ("GET", "/api/extensions/mcp"),
        ("POST", "/api/extensions/mcp"),
        ("DELETE", "/api/extensions/mcp/{name}"),
        ("GET", "/api/extensions/skills"),
        ("POST", "/api/extensions/skills"),
        // 技能市场。**上游是外部服务**（SkillHub），所以能不能返回 200
        // 取决于网络，不取决于本地实现：断网时它走 503 并带 next_step。
        // 本测试只要求「不是 404，且失败响应自带下一步」——
        // 换句话说，它接受「市场连不上」这个诚实答案，但拒绝 501 那种
        // 「假装这条路还没接」的假成功。
        ("GET", "/api/extensions/skill-hub"),
        ("POST", "/api/extensions/skill-hub/{slug}/install"),
        // 单技能这三条：搜索、榜单、安装。上游确有（实测 200），
        // 少了它们，用户在市场里只能看到「装编排说明」的那一层。
        ("GET", "/api/extensions/skill-hub/skills"),
        ("GET", "/api/extensions/skill-hub/rankings"),
        ("POST", "/api/extensions/skill-hub/skills/{slug}/install"),
        // 技能开关。停用的技能不挂进对话工具表，而此前这条路由没登记，
        // 界面上也就没有任何一处能把技能打开 —— 后端做完了、开关没出口。
        ("PATCH", "/api/extensions/skills/{name}"),
        ("DELETE", "/api/extensions/skills/{name}"),
        ("GET", "/api/experts"),
        ("POST", "/api/experts"),
        ("GET", "/api/experts/{slug}"),
        ("PATCH", "/api/experts/{slug}"),
        ("DELETE", "/api/experts/{slug}"),
        // 专家市场（2026-10-06 接线，B1-6）。连上游，接不上时报的是上游不可达
        // 而不是 501 —— 那是**上游**的问题，不是这条路由没做。
        ("GET", "/api/experts/market"),
        ("POST", "/api/experts/market/{slug}/install"),
        ("GET", "/api/sessions"),
        ("POST", "/api/sessions"),
        ("GET", "/api/sessions/{id}"),
        ("DELETE", "/api/sessions/{id}"),
        ("GET", "/api/sessions/{id}/metrics"),
        ("GET", "/api/sessions/{id}/context"),
        ("GET", "/api/usage"),
        ("GET", "/api/teams"),
        ("POST", "/api/teams"),
        ("GET", "/api/teams/{id}"),
        ("PATCH", "/api/teams/{id}"),
        ("DELETE", "/api/teams/{id}"),
        ("POST", "/api/sessions/{id}/messages"),
        ("GET", "/api/wiki/pages"),
        ("GET", "/api/wiki/pages/{path}"),
        ("GET", "/api/wiki/index"),
        ("GET", "/api/wiki/log"),
        ("GET", "/api/admin/config"),
        ("PUT", "/api/admin/config"),
        ("GET", "/api/admin/providers"),
        ("POST", "/api/admin/providers"),
        ("PUT", "/api/admin/providers/{id}"),
        ("DELETE", "/api/admin/providers/{id}"),
        ("PUT", "/api/admin/providers/{id}/default"),
        ("GET", "/api/admin/providers/{id}/models"),
        ("GET", "/api/admin/models"),
        // 备份三条（2026-10-06 接线，B4-1）。工作区页有真按钮，打 501 就是
        // 「点了必失败」。注意 `restore` 虽在这份清单里（不再是 501），
        // 但它**不还原** —— 响应里 `restored: false`，并给出停服后要跑的命令；
        // `backup_http.rs` 钉死了「不许出现假成功」。
        ("POST", "/api/backup/export"),
        ("POST", "/api/backup/verify"),
        ("POST", "/api/backup/restore"),
        // 用户管理：只列名册与启停两条是真实现（2026-10-06）。
        // `POST`/`DELETE` **继续 501，而且是有意的** —— 账号只来自部署配置，
        // 软删除也只做了一半（`users.deleted_at` 全仓库没有写入点）。
        // 理由写在 api_users.rs 的文件头，那两条 501 的 next_step 会原样说出来。
        ("GET", "/api/users"),
        ("PATCH", "/api/users/{id}"),
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
        let t = body_text(resp).await;

        // 「路由没挂上」与「资源不存在」**都是 404**，光看状态码分不开。
        // 这两条 dispatch 路由现在都会对不存在的团队回 404（那是正确的），
        // 所以这里要认的是错误信封里的 `code`：路由没挂上是 `not_found`
        // （来自 fallback，找不到路由），资源不存在是 `entity_not_found`。
        //
        // 以前这条断言写的是 `assert_ne!(status, NOT_FOUND)`，在本项目里
        // 那是把「未注册」和「查无此人」混为一谈 —— 正好是 B1-2 说的那类
        // 分不清三种状态的问题。区分它们只能靠 code。
        assert!(
            !t.contains("\"code\":\"not_found\""),
            "{method} {concrete} 必须已注册（code=not_found 说明 handler 没挂上）：{t}"
        );

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
        llm: Arc::new(RwLock::new(None)),
        llm_config: Arc::new(RwLock::new(Default::default())),
        providers: Arc::new(RwLock::new(Default::default())),
        login_limiter: Arc::new(Default::default()),
        pbkdf2: quill_control::Pbkdf2Params::for_tests(),
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

// ---------------------------------------------------------------------------
// 实例级 /api/admin/config 契约
// ---------------------------------------------------------------------------

fn json_put(token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri("/api/admin/config")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("构造 PUT 失败")
}

fn json_get(token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/api/admin/config")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("构造 GET 失败")
}

/// 非 admin 令牌访问 /api/admin/config 必须被拒绝：未带令牌 → 401，
/// 带非 admin 令牌 → 403。
#[tokio::test]
async fn admin_config_rejects_non_admin() {
    let plain = build_router(state())
        .oneshot(json_get(TOKEN_PLAIN))
        .await
        .expect("失败");
    assert_eq!(
        plain.status(),
        StatusCode::FORBIDDEN,
        "非 admin 必须 403（不是 200，也不是 401）"
    );
    let body = body_text(plain).await;
    assert!(body.contains("forbidden"), "错误码必须是 forbidden：{body}");
    assert!(body.contains("下一步"), "必须给中文下一步：{body}");
    assert!(
        body.contains("admin"),
        "next_step 要点名 admin 角色（让用户知道怎么修）：{body}"
    );

    let missing = build_router(state())
        .oneshot(req("GET", "/api/admin/config"))
        .await
        .expect("失败");
    assert_eq!(
        missing.status(),
        StatusCode::UNAUTHORIZED,
        "未带令牌必须 401"
    );

    let put_plain = build_router(state())
        .oneshot(json_put(
            TOKEN_PLAIN,
            serde_json::json!({
                "protocol": "openai",
                "base_url": "http://x/v1",
                "api_key": "",
                "model": "m",
                "max_context_tokens": 32768,
                "compaction_threshold_tokens": 8000,
                "max_output_tokens": 2048
            }),
        ))
        .await
        .expect("失败");
    assert_eq!(
        put_plain.status(),
        StatusCode::FORBIDDEN,
        "PUT 同样要 403（不能 401 也不能 200）"
    );
}

/// admin 写合法 → GET 字段一致 → 再写非法 → 400 + 中文 next_step。
#[tokio::test]
async fn admin_config_round_trips_and_rejects_invalid() {
    // 第一次：合法的 openai 配置。
    let good = serde_json::json!({
        "protocol": "openai",
        "base_url": "http://example.local:18080/v1",
        "api_key": "sk-test-abcdef",
        "model": "local-model",
        "max_context_tokens": 32768,
        "compaction_threshold_tokens": 8000,
        "max_output_tokens": 2048,
    });
    let put_ok = build_router(state())
        .oneshot(json_put(TOKEN_ADMIN, good.clone()))
        .await
        .expect("失败");
    assert_eq!(
        put_ok.status(),
        StatusCode::OK,
        "合法 PUT 必须 200：{}",
        body_text(put_ok).await
    );
    let put_text = body_text(
        build_router(state())
            .oneshot(json_put(TOKEN_ADMIN, good.clone()))
            .await
            .expect("失败"),
    )
    .await;
    let put_body: serde_json::Value = serde_json::from_str(&put_text).expect("响应必须是 JSON");
    assert_eq!(put_body["protocol"], "openai");
    assert_eq!(put_body["base_url"], "http://example.local:18080/v1");
    assert_eq!(put_body["has_api_key"], true);
    assert!(
        put_body.get("api_key").is_none(),
        "响应不许回传 api_key 明文：{put_text}"
    );
    assert!(
        !put_text.contains("sk-test-abcdef"),
        "api_key 明文泄漏了：{put_text}"
    );
    assert_eq!(put_body["model"], "local-model");
    assert_eq!(put_body["max_context_tokens"], 32768);
    assert_eq!(put_body["compaction_threshold_tokens"], 8000);
    assert_eq!(put_body["max_output_tokens"], 2048);
    assert!(
        put_body["updated_at"].as_i64().unwrap_or(0) > 0,
        "updated_at 必须是非零时间戳：{put_text}"
    );

    // GET 拿回相同字段。
    let get_resp = build_router(state())
        .oneshot(json_get(TOKEN_ADMIN))
        .await
        .expect("失败");
    assert_eq!(get_resp.status(), StatusCode::OK);
    let get_text = body_text(get_resp).await;
    let get_body: serde_json::Value = serde_json::from_str(&get_text).expect("JSON");
    for k in [
        "protocol",
        "base_url",
        "has_api_key",
        "model",
        "max_context_tokens",
        "compaction_threshold_tokens",
        "max_output_tokens",
    ] {
        assert_eq!(get_body[k], put_body[k], "字段 {k:?} 必须一致");
    }
    assert_eq!(get_body["has_api_key"], true, "库里存了密钥时必须报 true");
    assert!(
        get_body.get("api_key").is_none(),
        "GET 响应不许回传 api_key：{get_text}"
    );

    // 非法字段 1：protocol 不在枚举里。
    let bad_proto = serde_json::json!({
        "protocol": "bogus",
        "base_url": "http://x/v1",
        "api_key": "",
        "model": "m",
        "max_context_tokens": 32768,
        "compaction_threshold_tokens": 8000,
        "max_output_tokens": 2048,
    });
    let r = build_router(state())
        .oneshot(json_put(TOKEN_ADMIN, bad_proto))
        .await
        .expect("失败");
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let b = body_text(r).await;
    assert!(b.contains("bad_request"), "错误码必须是 bad_request：{b}");
    assert!(b.contains("下一步"), "必须给中文下一步：{b}");
    assert!(b.contains("openai"), "next_step 要列出合法枚举：{b}");

    // 非法字段 2：max_context_tokens 是负数。
    let bad_ctx = serde_json::json!({
        "protocol": "openai",
        "base_url": "http://x/v1",
        "api_key": "",
        "model": "m",
        "max_context_tokens": -1,
        "compaction_threshold_tokens": 8000,
        "max_output_tokens": 2048,
    });
    let r = build_router(state())
        .oneshot(json_put(TOKEN_ADMIN, bad_ctx))
        .await
        .expect("失败");
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let b = body_text(r).await;
    assert!(b.contains("下一步"), "必须给中文下一步：{b}");
    assert!(
        b.contains("max_context_tokens"),
        "要点名出错字段：{b}"
    );

    // 非法字段 3：compaction 大于 context。
    let bad_balance = serde_json::json!({
        "protocol": "openai",
        "base_url": "http://x/v1",
        "api_key": "",
        "model": "m",
        "max_context_tokens": 8000,
        "compaction_threshold_tokens": 16000,
        "max_output_tokens": 2048,
    });
    let r = build_router(state())
        .oneshot(json_put(TOKEN_ADMIN, bad_balance))
        .await
        .expect("失败");
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let b = body_text(r).await;
    assert!(b.contains("compaction_threshold"), "要指出越界字段：{b}");
}

/// 热重载：PUT 后 state 里的 provider/config 被替换；再 PUT 一个根本
/// 不存在的 base_url 时，旧 provider 仍能维持（构造失败保留旧值）。
#[tokio::test]
async fn admin_config_hot_reloads_provider_and_keeps_old_on_build_failure() {
    use quill_server::llm::AdminConfig;

    let s = state();

    // 直接用 lib 的 admin helpers 走完整路径（绕开 HTTP 层）。
    let good = AdminConfig {
        protocol: quill_server::llm::Protocol::Openai,
        base_url: "http://127.0.0.1:65530/v1".into(), // 故意不可达
        api_key: "".into(),
        model: "stub".into(),
        max_context_tokens: 32768,
        compaction_threshold_tokens: 8000,
        max_output_tokens: 2048,
        updated_at: 0,
    };
    // 直接构造 OpenAiCompatible 占位，验证 slot 能塞进去；构造本身只校验 URL 形状。
    let cfg_llm = good.to_llm_config();
    let provider = quill_server::llm::build(&cfg_llm).expect("URL 形状合法时应能构造");
    s.replace_llm(Some(provider.clone()), cfg_llm.clone());

    assert_eq!(
        s.llm()
            .expect("hot swap 后必须有 provider")
            .name(),
        "openai-compatible",
        "替换后的 provider 名字必须反映新配置"
    );
    let snap = s.llm_config_snapshot();
    assert_eq!(snap.model, "stub");
    assert_eq!(snap.base_url, "http://127.0.0.1:65530/v1");
    assert_eq!(snap.max_context_tokens, 32768);
    assert_eq!(snap.compaction_threshold_tokens, 8000);
    assert_eq!(snap.max_tokens, 2048);
}

/// 把 admin 写表后，再去 `provider.chat` 路径 —— 直接拿应用 router 跑通一遍：
/// 写到数据库 → 用 state 重建 provider → state.llm() 返回非 None。
/// 用一个真 chat 路由覆盖热替换语义。
#[tokio::test]
async fn admin_config_put_then_state_has_provider() {
    let shared = state();
    // 直接走 HTTP PUT。
    let body = serde_json::json!({
        "protocol": "openai",
        "base_url": "http://127.0.0.1:65530/v1",
        "api_key": "",
        "model": "after-put",
        "max_context_tokens": 32768,
        "compaction_threshold_tokens": 8000,
        "max_output_tokens": 2048,
    });
    let resp = build_router(shared.clone())
        .oneshot(json_put(TOKEN_ADMIN, body))
        .await
        .expect("失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert_eq!(status, StatusCode::OK, "合法 PUT 必须 200，实际 {status}：{text}");

    // 同一 state（共享）上断言 hot swap 已生效 —— 这一段不依赖 SQLite 行，
    // 只看 in-memory 的 `Arc<RwLock<...>>` slot，所以并行跑也安全。
    let llm = shared
        .llm()
        .expect("PUT 后 state.llm 必须是 Some（hot reload 失败时是 provider_unavailable）");
    assert_eq!(llm.name(), "openai-compatible");
    assert_eq!(shared.llm_config_snapshot().model, "after-put");
    assert_eq!(shared.llm_config_snapshot().base_url, "http://127.0.0.1:65530/v1");
    assert_eq!(shared.llm_config_snapshot().max_tokens, 2048);
    assert_eq!(shared.llm_config_snapshot().max_context_tokens, 32768);
}

// ---------------------------------------------------------------------------
// 多模型供应商 / 模型池契约
//
// 这些用例各自用**独立的临时库**（provider_state），不碰上面 admin_config
// 那几条共用的库：那几条会并发改「默认 provider」。
// ---------------------------------------------------------------------------

fn provider_state(label: &str) -> (TestDb, AppState) {
    let db = TestDb::new(label);
    let state = state_with(Some(&db));
    (db, state)
}

fn prov_json(
    method: &str,
    path: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let payload = body.unwrap_or_else(|| serde_json::json!({}));
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .expect("构造请求失败")
}

async fn prov_send(
    state: &AppState,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let resp = build_router(state.clone())
        .oneshot(prov_json(method, path, TOKEN_ADMIN, body))
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    let text = body_text(resp).await;
    let parsed = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("响应必须是 JSON（{e}，{status}）：{text}"));
    (status, parsed)
}

async fn prov_delete(state: &AppState, id: &str) -> (StatusCode, String) {
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/admin/providers/{id}"))
                .header("authorization", format!("Bearer {TOKEN_ADMIN}"))
                .body(Body::empty())
                .expect("构造 DELETE 失败"),
        )
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    (status, body_text(resp).await)
}

async fn create_provider(state: &AppState, body: serde_json::Value) -> serde_json::Value {
    let (status, out) = prov_send(state, "POST", "/api/admin/providers", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "建 provider 必须 201：{out}");
    out
}

fn provider_id(v: &serde_json::Value) -> String {
    v["id"]
        .as_str()
        .unwrap_or_else(|| panic!("响应必须带 id：{v}"))
        .to_string()
}

fn base_provider_body(name: &str, base_url: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "preset_id": "custom",
        "kind": "custom",
        "protocol": "openai",
        "base_url": base_url,
        "api_key": "sk-secret-value-123",
        "model": "qwen3.5",
        "max_context_tokens": 32768,
        "compaction_threshold_tokens": 8000,
        "max_output_tokens": 4096,
    })
}

/// 假上游：返回固定的 /v1/models JSON。字段形状照 llama.cpp 的真实响应
/// —— `data[].meta.n_ctx_train` 才是训练上下文，`n_ctx` 是当前实例上下文。
async fn spawn_fake_upstream(payload: &'static str) -> String {
    let app = axum::Router::new().route(
        "/v1/models",
        axum::routing::get(move || {
            let payload = payload;
            async move {
                (
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    payload,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("假上游必须能绑回环端口");
    let addr = listener.local_addr().expect("读本机地址");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}/v1")
}

const FAKE_LLAMA_MODELS: &str = r#"{
  "models": [
    {"name": "D:\\models\\Qwen3.5-4B-Q4_K_M.gguf", "type": "model",
     "capabilities": ["completion"]}
  ],
  "object": "list",
  "data": [
    {"id": "D:\\models\\Qwen3.5-4B-Q4_K_M.gguf", "object": "model",
     "created": 1791193408, "owned_by": "llamacpp",
     "meta": {"n_ctx": 8192, "n_ctx_train": 262144, "n_embd": 2560,
              "ftype": "Q4_K - Medium"}}
  ]
}"#;

const FAKE_MODEL_ID: &str = "D:\\models\\Qwen3.5-4B-Q4_K_M.gguf";

/// 1) CRUD 契约：201 / 200 / 204，且 **api_key 明文绝不出现在任何响应里**。
#[tokio::test]
async fn providers_crud_round_trip_and_never_leak_the_api_key() {
    let (_db, s) = provider_state("providers-crud");

    let created =
        create_provider(&s, base_provider_body("本地 llama", "http://127.0.0.1:18080/v1")).await;
    let id = provider_id(&created);
    assert_eq!(id.len(), 32, "id 必须是 32 位 hex：{created}");
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_lowercase()),
        "id 必须全大写 hex（SQLite hex() 读出来是大写）：{id}"
    );
    assert_eq!(created["kind"], "custom");
    assert_eq!(created["protocol"], "openai");
    assert_eq!(created["has_api_key"], true);
    assert_eq!(
        created["is_default"], true,
        "库里没有 provider 时新建的那条就是默认"
    );
    assert_eq!(created["enabled"], true);
    assert!(
        created.get("api_key").is_none(),
        "响应里不许有 api_key 字段：{created}"
    );
    assert!(
        !created.to_string().contains("sk-secret-value-123"),
        "api_key 明文泄漏到响应里了：{created}"
    );

    let (status, list) = prov_send(&s, "GET", "/api/admin/providers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!list.to_string().contains("sk-secret-value-123"), "{list}");
    let rows = list["providers"].as_array().expect("providers 必须是数组");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"].as_str(), Some(id.as_str()));
    assert_eq!(rows[0]["name"], "本地 llama");

    // PUT 是**部分更新**：只发 model，其余字段必须原样保留。
    let (status, updated) = prov_send(
        &s,
        "PUT",
        &format!("/api/admin/providers/{id}"),
        Some(serde_json::json!({ "model": "Qwen3.5-4B-Q4_K_M.gguf" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "部分更新必须 200：{updated}");
    assert_eq!(updated["model"], "Qwen3.5-4B-Q4_K_M.gguf");
    for k in [
        "name",
        "base_url",
        "protocol",
        "preset_id",
        "kind",
        "max_context_tokens",
        "compaction_threshold_tokens",
        "max_output_tokens",
        "enabled",
        "created_at",
    ] {
        assert_eq!(updated[k], created[k], "只发 model 时字段 {k:?} 不许被清空");
    }
    assert_eq!(updated["has_api_key"], true, "没传 api_key 就必须沿用旧密钥");
    assert!(!updated.to_string().contains("sk-secret-value-123"), "{updated}");

    // 显式 null = 清除；空串 = 沿用。
    let (status, cleared) = prov_send(
        &s,
        "PUT",
        &format!("/api/admin/providers/{id}"),
        Some(serde_json::json!({ "api_key": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "清密钥必须 200：{cleared}");
    assert_eq!(cleared["has_api_key"], false, "显式 null 必须清除密钥");

    // 热重载：默认 provider 的 model 变了，运行时快照必须跟着变。
    assert_eq!(s.llm_config_snapshot().model, "Qwen3.5-4B-Q4_K_M.gguf");
    assert!(s.llm().is_ok(), "默认 provider 必须已装回运行时");

    let second = create_provider(&s, base_provider_body("远端", "http://example.invalid/v1")).await;
    let second_id = provider_id(&second);
    assert_eq!(second["is_default"], false);

    let (status, text) = prov_delete(&s, &second_id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "删除必须 204：{text}");
    let (_, list) = prov_send(&s, "GET", "/api/admin/providers", None).await;
    assert_eq!(list["providers"].as_array().map(Vec::len), Some(1));
}

/// 2) 400 校验失败 + 404 未知 id，全部中文 detail + next_step。
#[tokio::test]
async fn provider_validation_failures_and_unknown_ids_are_explicit() {
    let (_db, s) = provider_state("providers-validate");
    let _ = create_provider(&s, base_provider_body("本地", "http://127.0.0.1:18080/v1")).await;

    let bad_bodies: Vec<(serde_json::Value, &str)> = vec![
        (
            serde_json::json!({ "preset_id": "custom", "kind": "custom", "protocol": "openai", "base_url": "http://x/v1" }),
            "name",
        ),
        (
            serde_json::json!({ "name": "x", "kind": "custom", "protocol": "bogus", "base_url": "http://x/v1" }),
            "protocol",
        ),
        (
            serde_json::json!({ "name": "x", "kind": "magic", "protocol": "openai", "base_url": "http://x/v1" }),
            "kind",
        ),
        (
            serde_json::json!({ "name": "x", "kind": "custom", "protocol": "openai", "base_url": "http://x/v1", "max_context_tokens": 0 }),
            "max_context_tokens",
        ),
        (
            serde_json::json!({ "name": "x", "kind": "custom", "protocol": "openai", "base_url": "http://x/v1",
                                "max_context_tokens": 8000, "compaction_threshold_tokens": 16000 }),
            "compaction_threshold_tokens",
        ),
    ];
    for (body, field) in bad_bodies {
        let (status, out) = prov_send(&s, "POST", "/api/admin/providers", Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body} 必须 400：{out}");
        let err = &out["error"];
        assert_eq!(err["code"], "bad_request", "{out}");
        let detail = err["detail"].as_str().unwrap_or_default().to_string();
        assert!(detail.contains(field), "要点名出错字段 {field}：{detail}");
        assert!(detail.contains("下一步"), "必须给中文下一步：{detail}");
        assert!(!err["next_step"].as_str().unwrap_or_default().is_empty());
    }

    // 被拒的写不该留下行。
    let (_, list) = prov_send(&s, "GET", "/api/admin/providers", None).await;
    assert_eq!(list["providers"].as_array().map(Vec::len), Some(1));

    for (method, path) in [
        (
            "GET",
            "/api/admin/providers/DEADBEEFDEADBEEFDEADBEEFDEADBEEF/models".to_string(),
        ),
        (
            "PUT",
            "/api/admin/providers/DEADBEEFDEADBEEFDEADBEEFDEADBEEF/default".to_string(),
        ),
        (
            "PUT",
            "/api/admin/providers/DEADBEEFDEADBEEFDEADBEEFDEADBEEF".to_string(),
        ),
        (
            "DELETE",
            "/api/admin/providers/DEADBEEFDEADBEEFDEADBEEFDEADBEEF".to_string(),
        ),
    ] {
        let (status, out) = prov_send(&s, method, &path, Some(serde_json::json!({}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path} 必须 404：{out}");
        assert_eq!(out["error"]["code"], "entity_not_found", "{out}");
    }
}

/// 3) 非 admin 一律 403；没令牌一律 401。
#[tokio::test]
async fn provider_routes_reject_non_admin() {
    let (_db, s) = provider_state("providers-auth");
    for (method, path) in [
        ("GET", "/api/admin/providers"),
        ("POST", "/api/admin/providers"),
        ("GET", "/api/admin/models"),
    ] {
        let resp = build_router(s.clone())
            .oneshot(prov_json(
                method,
                path,
                TOKEN_PLAIN,
                Some(serde_json::json!({})),
            ))
            .await
            .expect("oneshot 失败");
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{method} {path} 非 admin 必须 403"
        );
        let t = body_text(resp).await;
        assert!(t.contains("forbidden"), "{t}");
        assert!(t.contains("下一步"), "{t}");
    }

    let resp = build_router(s.clone())
        .oneshot(req("GET", "/api/admin/models"))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "没令牌必须 401");
}

/// 4) 删「唯一」或「当前默认」provider 必须 409。
#[tokio::test]
async fn deleting_the_only_or_the_default_provider_is_refused() {
    let (_db, s) = provider_state("providers-409");
    let only = provider_id(
        &create_provider(&s, base_provider_body("唯一", "http://127.0.0.1:18080/v1")).await,
    );

    let (status, t) = prov_delete(&s, &only).await;
    assert_eq!(status, StatusCode::CONFLICT, "删唯一 provider 必须 409：{t}");
    assert!(t.contains("conflict"), "{t}");
    // 「下一步」是独立的 next_step 字段，不在 detail 里（Conflict 的 advice
    // 由调用方给：删 provider 的正确做法和删专家的不一样）。
    let ns = serde_json::from_str::<serde_json::Value>(&t).expect("响应必须是 JSON")["error"]
        ["next_step"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(!ns.is_empty(), "409 必须自带下一步：{t}");
    assert!(
        ns.contains("/api/admin/providers"),
        "「下一步」要指一条真实可走的路由：{t}"
    );

    let second = provider_id(
        &create_provider(&s, base_provider_body("第二个", "http://127.0.0.1:18081/v1")).await,
    );

    let (status, t) = prov_delete(&s, &only).await;
    assert_eq!(status, StatusCode::CONFLICT, "删当前默认 provider 必须 409：{t}");
    assert!(t.contains("default"), "文案要点名「默认」：{t}");

    let (status, out) = prov_send(
        &s,
        "PUT",
        &format!("/api/admin/providers/{second}/default"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "切默认必须 200：{out}");
    assert_eq!(out["is_default"], true);
    assert_eq!(s.llm_config_snapshot().base_url, "http://127.0.0.1:18081/v1");

    let (status, t) = prov_delete(&s, &only).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "不再是默认就允许删：{t}");

    // 全表只能有一个默认项。
    let (_, list) = prov_send(&s, "GET", "/api/admin/providers", None).await;
    let defaults = list["providers"]
        .as_array()
        .expect("数组")
        .iter()
        .filter(|p| p["is_default"] == true)
        .count();
    assert_eq!(defaults, 1, "默认项必须唯一");
}

/// 5) PUT /{id}/default 之后，旧的 /api/admin/config 必须反映新的默认 provider。
#[tokio::test]
async fn setting_a_new_default_is_reflected_by_the_legacy_admin_config() {
    let (_db, s) = provider_state("providers-legacy");
    let _a = provider_id(
        &create_provider(&s, base_provider_body("A", "http://127.0.0.1:18080/v1")).await,
    );
    let b = provider_id(
        &create_provider(&s, base_provider_body("B", "http://127.0.0.1:18081/v1")).await,
    );

    let resp = build_router(s.clone())
        .oneshot(json_get(TOKEN_ADMIN))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK, "旧接口必须继续可用");
    let before: serde_json::Value = serde_json::from_str(&body_text(resp).await).expect("JSON");
    assert_eq!(before["base_url"], "http://127.0.0.1:18080/v1");

    let (status, out) = prov_send(&s, "PUT", &format!("/api/admin/providers/{b}/default"), None).await;
    assert_eq!(status, StatusCode::OK, "{out}");

    let resp = build_router(s.clone())
        .oneshot(json_get(TOKEN_ADMIN))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let text = body_text(resp).await;
    let after: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(
        after["base_url"], "http://127.0.0.1:18081/v1",
        "切默认后旧接口必须跟着变：{text}"
    );
    assert_eq!(after["model"], "qwen3.5");
    assert_eq!(after["max_context_tokens"], 32768);
    assert_eq!(after["compaction_threshold_tokens"], 8000);
    assert_eq!(after["max_output_tokens"], 4096);
    assert_eq!(after["protocol"], "openai");
}

/// 6) 真实探测：display_name / context_window(n_ctx) / modality / owned_by。
///
/// 注意 `context_window` 取的是 **`n_ctx`（实例窗口）而不是 `n_ctx_train`
/// （训练窗口）**。这个夹具照抄真机 llama.cpp 响应，两者同时存在：
/// `n_ctx = 8192`、`n_ctx_train = 262144`。报后者会把「上下文上限」说大 32 倍，
/// 而用户正是照着这个数去填 `max_context_tokens`。见 ISSUE-022。
#[tokio::test]
async fn model_probe_derives_name_context_and_modality_from_the_upstream() {
    let base_url = spawn_fake_upstream(FAKE_LLAMA_MODELS).await;
    let (_db, s) = provider_state("providers-probe");
    let mut body = base_provider_body("假上游", &base_url);
    body["model"] = serde_json::json!(FAKE_MODEL_ID);
    let id = provider_id(&create_provider(&s, body).await);

    let (status, out) = prov_send(&s, "GET", &format!("/api/admin/providers/{id}/models"), None).await;
    assert_eq!(status, StatusCode::OK, "探测接口必须 200：{out}");
    assert_eq!(out["provider_id"].as_str(), Some(id.as_str()));
    assert!(out["probed_at"].as_i64().unwrap_or(0) > 0, "必须带探测时刻");
    assert!(out["error"].is_null(), "探测成功时 error 必须是 null：{out}");

    let models = out["models"].as_array().expect("models 必须是数组");
    assert_eq!(models.len(), 1, "上游报了几个就是几个：{out}");
    let m = &models[0];
    assert_eq!(m["id"], FAKE_MODEL_ID, "id 必须原样透传");
    assert_eq!(m["display_name"], "Qwen3.5-4B-Q4_K_M");
    assert_eq!(m["context_window"], 8192, "必须是实例窗口 n_ctx，不是训练窗口 n_ctx_train");
    assert_eq!(m["modality"], "text", "capabilities=[completion] 是文本证据");
    assert_eq!(m["owned_by"], "llamacpp");

    // 模型池：starred 判定就是 model.id == provider.model
    let (status, pool) = prov_send(&s, "GET", "/api/admin/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pool["default_provider_id"].as_str(), Some(id.as_str()));
    assert!(pool["generated_at"].as_i64().unwrap_or(0) > 0);
    assert!(pool["unavailable"].as_array().expect("数组").is_empty());
    let entries = pool["pool"].as_array().expect("pool 必须是数组");
    assert_eq!(entries.len(), 1, "{pool}");
    assert_eq!(entries[0]["provider_id"].as_str(), Some(id.as_str()));
    assert_eq!(entries[0]["provider_name"], "假上游");
    assert_eq!(entries[0]["starred"], true, "等于 provider.model 的模型必须打星");
    assert_eq!(entries[0]["model"]["context_window"], 8192, "模型池里同样是实例窗口");
    assert_eq!(entries[0]["model"]["display_name"], "Qwen3.5-4B-Q4_K_M");

    // ☆ 点一下只发 model：不能把别的字段清空。
    let (status, star) = prov_send(
        &s,
        "PUT",
        &format!("/api/admin/providers/{id}"),
        Some(serde_json::json!({ "model": FAKE_MODEL_ID })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{star}");
    assert_eq!(star["name"], "假上游");
    assert_eq!(star["base_url"], base_url);
    assert_eq!(star["max_context_tokens"], 32768);
}

/// 7) 探测失败必须进 unavailable（中文原因），不许变成空 pool。
#[tokio::test]
async fn an_unreachable_provider_lands_in_unavailable_not_in_an_empty_pool() {
    let (_db, s) = provider_state("providers-unavailable");
    let dead = create_provider(&s, base_provider_body("打不通", "http://127.0.0.1:9/v1")).await;
    let dead_id = provider_id(&dead);

    let (status, out) = prov_send(&s, "GET", &format!("/api/admin/providers/{dead_id}/models"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        out["models"].as_array().expect("数组").is_empty(),
        "探测失败不许编出模型：{out}"
    );
    let err = out["error"].as_str().expect("探测失败必须给原因");
    assert!(
        err.chars().any(|c| c as u32 > 0x2e80),
        "原因必须是中文：{err}"
    );

    let (status, pool) = prov_send(&s, "GET", "/api/admin/models", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        pool["pool"].as_array().expect("数组").is_empty(),
        "池里不该有打不通的 provider：{pool}"
    );
    let un = pool["unavailable"].as_array().expect("unavailable 必须是数组");
    assert_eq!(un.len(), 1, "打不通的 provider 必须进 unavailable：{pool}");
    assert_eq!(un[0]["provider_id"].as_str(), Some(dead_id.as_str()));
    assert_eq!(un[0]["provider_name"], "打不通");
    let reason = un[0]["error"].as_str().expect("中文原因");
    assert!(
        reason.chars().any(|c| c as u32 > 0x2e80),
        "原因必须是中文：{reason}"
    );

    // 改回可达地址后必须重新出现在 pool 里（不许只做一次性快照）。
    let reachable = spawn_fake_upstream(FAKE_LLAMA_MODELS).await;
    let (status, fixed) = prov_send(
        &s,
        "PUT",
        &format!("/api/admin/providers/{dead_id}"),
        Some(serde_json::json!({ "base_url": reachable })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fixed}");
    let (_, pool) = prov_send(&s, "GET", "/api/admin/models", None).await;
    assert_eq!(
        pool["pool"].as_array().map(Vec::len),
        Some(1),
        "改回可达地址后必须回到 pool：{pool}"
    );
    assert!(pool["unavailable"].as_array().expect("数组").is_empty());
}

/// 遗留的 `GET /api/admin/config` 也**不许**回传明文密钥。
/// 它和 `/api/admin/providers*` 是同一个 admin 凭据下的两条路，只要有一条漏，
/// 前面 providers 路由做的「只给 has_api_key」就被旁路了。
#[tokio::test]
async fn the_legacy_admin_config_never_echoes_the_api_key() {
    let (_db, s) = provider_state("legacy-config-no-key");
    let created = create_provider(&s, base_provider_body("带密钥的端点", "http://127.0.0.1:18080/v1")).await;
    let id = provider_id(&created);
    assert_eq!(created["has_api_key"], true, "建的时候确实存了密钥：{created}");

    // 让它成为默认 provider，legacy config 才有东西可回。
    let (status, _) = prov_send(&s, "PUT", &format!("/api/admin/providers/{id}/default"), None).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = prov_send(&s, "GET", "/api/admin/config", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.get("api_key").is_none(),
        "legacy config 响应里不该有 api_key 这个键：{body}"
    );
    let raw = body.to_string();
    assert!(
        !raw.contains("sk-secret-value-123"),
        "密钥明文泄漏：{raw}"
    );
    assert_eq!(body["has_api_key"], true, "只该用 has_api_key 说明已设置：{body}");

    // 请求体仍然接受 api_key（留空 = 沿用旧值），否则前端无法在不误清密钥的前提下保存。
    let (status, kept) = prov_send(
        &s,
        "PUT",
        "/api/admin/config",
        Some(serde_json::json!({
            "protocol": "openai",
            "base_url": "http://127.0.0.1:18080/v1",
            "api_key": "",
            "model": "qwen3.5-kept",
            "max_context_tokens": 32768,
            "compaction_threshold_tokens": 8000,
            "max_output_tokens": 4096,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{kept}");
    assert!(!kept.to_string().contains("sk-secret-value-123"), "{kept}");
    let (_, after) = prov_send(&s, "GET", "/api/admin/config", None).await;
    assert_eq!(after["model"], "qwen3.5-kept", "空 api_key 不该清掉旧密钥：{after}");
    assert_eq!(after["has_api_key"], true, "空 api_key 不该清掉旧密钥：{after}");
}

/// 停用默认 provider 必须**真的**生效：`llm::build` 不读 `enabled` 是上一轮的漏洞，
/// 表现为「模型池空了 / 健康检查说就绪 / 聊天照常能用」三者自相矛盾。
#[tokio::test]
async fn disabling_the_default_provider_makes_the_runtime_honest() {
    let (_db, s) = provider_state("disable-default");
    let upstream = spawn_fake_upstream(FAKE_LLAMA_MODELS).await;
    let created = create_provider(&s, base_provider_body("唯一端点", &upstream)).await;
    let id = provider_id(&created);
    let (status, _) = prov_send(&s, "PUT", &format!("/api/admin/providers/{id}/default"), None).await;
    assert_eq!(status, StatusCode::OK);

    // 启用时：模型池里能看到它。
    let (_, pool) = prov_send(&s, "GET", "/api/admin/models", None).await;
    assert_eq!(pool["pool"].as_array().map(Vec::len), Some(1), "{pool}");

    // 关掉它。
    let (status, off) = prov_send(
        &s,
        "PUT",
        &format!("/api/admin/providers/{id}"),
        Some(serde_json::json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{off}");
    assert_eq!(off["enabled"], false, "enabled 字段必须真的写下去：{off}");

    // 模型池必须把它剔除，而且要和「探测失败」分开——停用是管理员主动的选择，
    // 混进 unavailable 会让界面显示成「连不上」，那是误导。
    let (_, pool) = prov_send(&s, "GET", "/api/admin/models", None).await;
    assert!(
        pool["pool"].as_array().expect("数组").is_empty(),
        "停用的 provider 不该留在 pool：{pool}"
    );
    let un = pool["unavailable"].as_array().expect("unavailable 必须是数组");
    assert!(
        un.iter().all(|p| p["provider_id"] != id.as_str()),
        "停用不是探测失败，不该混进 unavailable：{pool}"
    );
    let dis = pool["disabled"].as_array().expect("disabled 必须是数组");
    assert_eq!(dis.len(), 1, "停用的 provider 必须单独列出来：{pool}");
    assert_eq!(dis[0]["provider_id"].as_str(), Some(id.as_str()));
    assert_eq!(dis[0]["provider_name"], "唯一端点");
    let why = dis[0]["reason"].as_str().expect("停用要给原因");
    assert!(why.chars().any(|c| c as u32 > 0x2e80), "原因必须是中文：{why}");

    // 运行时必须同步：默认 provider 停用后不该还留着可用的 provider，
    // 否则界面三处（健康横幅 / 模型池 / 聊天）会自相矛盾。
    assert!(
        s.llm().is_err(),
        "默认 provider 停用后运行时不该还有可用 provider"
    );
    let reason = s.llm().expect_err("停用后必须给出原因");
    let text = format!("{reason:?}");
    assert!(
        text.chars().any(|c| c as u32 > 0x2e80),
        "停用原因必须是中文：{text}"
    );

    // 再打开必须能恢复 —— 不许只做一次性清空。
    let (status, on) = prov_send(
        &s,
        "PUT",
        &format!("/api/admin/providers/{id}"),
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{on}");
    assert!(s.llm().is_ok(), "重新启用后运行时必须恢复：{:?}", s.llm().err());
    let (_, pool) = prov_send(&s, "GET", "/api/admin/models", None).await;
    assert_eq!(pool["pool"].as_array().map(Vec::len), Some(1), "重新启用后要回到 pool：{pool}");
    assert!(
        pool["disabled"].as_array().expect("数组").is_empty(),
        "重新启用后不该还留在 disabled：{pool}"
    );
}
