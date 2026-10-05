//! HTTP 契约集成测试 —— 直接打路由，**不起真实端口**。
//!
//! # 为什么不起真实端口（铁律二十八：闸门不能替代实测的反面）
//!
//! 这里用 `tower::ServiceExt::oneshot` 把请求直接喂进 axum `Router`：
//! 走的是**真实的**路由匹配、真实的中间件链、真实的 extractor、
//! 真实的 `IntoResponse` —— 唯一没走的是 TCP 与 hyper。
//! 换来的是：测试不占端口、可并行、不受 CI 网络环境影响。
//! 真实端口的启动验证由 `bash scripts/` 冒烟与交付前手工 `curl` 覆盖。
//!
//! # 覆盖的五条路径（任务硬要求）
//!
//! | 路径 | 用例 |
//! |---|---|
//! | 200 | `/healthz` 免鉴权可达 |
//! | 401 | 缺令牌 / 错令牌 / 格式错 → **三种情形响应逐字节相同** |
//! | 404 | 未登记路径 |
//! | 501 | 已登记但未实现的契约路由 |
//! | 500 | handler panic → 中文说明 + 无内部细节泄漏 |
//!
//! # 存储说明
//!
//! 专家与派工路由已接真库，因此本文件的状态**带一个临时 SQLite 库**；
//! 打真库的行为断言在 `tests/expert_repo_sqlite.rs` /
//! `tests/dispatch_ledger_sqlite.rs` / `tests/expert_http.rs`，
//! 本文件只管「状态码契约」。

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

/// 测试用户 ID（合法 36 字符 uuid 形态）。
const UID_ADMIN: &str = "0192b7c8-0000-7000-8000-000000000001";
const UID_PLAIN: &str = "0192b7c8-0000-7000-8000-000000000002";
const TOKEN_ADMIN: &str = "tok-admin";
const TOKEN_PLAIN: &str = "tok-plain";

/// 构造带两个已知令牌的状态（**不读环境变量**）。
///
/// ⚠️ **不读环境变量**：测试必须与运行环境的 `QUILL_TOKENS` 无关，
/// 否则本地能过的测试在 CI 上会因环境不同而变红（铁律十四：
/// 「这个『通过』是因为真的检查了，还是因为根本没跑？」）。
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
        // ⚠️ `db_problem` 只在**没有**库时才有值：两者同时非空会让
        //    `AppState::db` 的语义变模糊（到底哪个是真的？）。
        db_problem: if db.is_some() {
            None
        } else {
            Some("测试注入：数据库不可用".to_string())
        },
    }
}

/// 默认状态：带一个临时真库（专家/派工路由需要它）。
fn state() -> AppState {
    state_with(Some(leak_db()))
}

/// 一个**进程内共享**的临时库。
///
/// ⚠️ 为什么共享而不是每次新建：`state()` 被本文件二十多个用例各调一次，
/// 每次建库 + 跑 44 条迁移会把测试拖慢一个数量级；
/// 而这些用例互不写数据（只断言状态码），共享一个空库是安全的。
/// `TestDb` 本身不需要被保留（`state` 已经拿到了 `Arc<DbBridge>`），
/// 目录清理交给操作系统回收 —— 这也是为什么这里刻意 `mem::forget`。
fn leak_db() -> &'static TestDb {
    use std::sync::OnceLock;
    static DB: OnceLock<TestDb> = OnceLock::new();
    DB.get_or_init(|| TestDb::new("http-contract"))
}

/// 状态 + 显式开关自检路由。
///
/// ⚠️ 走**配置字段**而不是环境变量：环境变量是进程全局的，在并行测试里
/// 改它会互相污染（一个用例开了自检，另一个就以为它默认开着）。
fn state_with_selftest(on: bool) -> AppState {
    let mut s = state();
    s.config.enable_selftest = on;
    s
}

/// 把「状态 + 路由」打包成可 `oneshot` 的 service。
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

// ─────────────────────────── 200 ───────────────────────────

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
    // 这条同时证明：鉴权中间件真的解析出了 UserId，而不是"放行所有人"。
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
    // 约束 5-a：普通用户默认 manual 审批模式。
    assert!(
        text.contains("\"approval_mode\":\"manual\""),
        "必须声明默认 manual 审批：{text}"
    );
}

#[tokio::test]
async fn version_returns_200_with_contract_pointer() {
    let resp = app!()
        .oneshot(authed("GET", "/api/version", TOKEN_PLAIN))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let text = body_text(resp).await;
    assert!(
        text.contains("PHASE2_CONTRACT.md"),
        "应指明契约出处：{text}"
    );
}

// ─────────────────────────── 401 ───────────────────────────

#[tokio::test]
async fn missing_token_is_401_not_404_or_500() {
    // ⚠️ 路由是存在的：401 而不是 404 证明"路由已登记，只是未认证"。
    //    若是 404，说明路由表根本没建起来 —— 那是另一种失败。
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
    // 🔴 铁律级断言：缺头 / 错令牌 / 格式错 三者**逐字节相同**。
    // 只要三者之一文案不同，攻击者就能用它区分"用户不存在"与"令牌错误"，
    // 从而把登录接口变成用户枚举器。
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
                // 缺 "Bearer " 前缀
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

// ─────────────────────────── 404 ───────────────────────────

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
    // 反向断言（铁律十二：豁免之外必须有反向断言）：
    // EXTRA_ROUTES 里列出的路径**确实存在**，因此不能被这条 404 用例覆盖。
    // 若有人把 /healthz 误列入 EXTRA_ROUTES 后又从路由表删掉，本用例不会发现；
    // 这条用例保证的是"清单与路由表一致"的另一半。
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

// ─────────────────────────── 501 ───────────────────────────

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
    // ⚠️ 顺序断言：未认证请求打 501 路由必须先吃 401。
    // 反过来（先返回 501 再鉴权）等于把"路由存在但没实现"泄露给未认证方。
    let resp = app!()
        .oneshot(req("GET", "/api/sessions"))
        .await
        .expect("失败");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn every_contract_route_responds_and_is_never_a_false_success() {
    // 🔴 遍历契约 §5.1 全量路由：每条都必须**有响应**，
    // 且**绝不能**是 200 假成功（已实现的那几条除外）。
    // 漏登记的路由会在这里变成 404 → 立刻红。
    //
    // ⚠️ IMPLEMENTED 里新增了专家 CRUD 五条中的四条：它们已接真库
    //    （quill-agent 领域 + quill-store 表），因此**不得**再要求 501。
    //    加进来的那一刻起，这条断言就变成「别让已实现的路由退回 501」的守门人。
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
        // 把契约里的 `{id}` 占位换成合法实例值。
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
        // 🔴 404 有**两种**：路由没登记（not_found）与资源不存在（entity_not_found）。
        //    对已实现的路由，404 本身不能证明「没登记」—— 得看错误码。
        let body = body_text(resp).await;
        if status == StatusCode::NOT_FOUND {
            assert!(
                body.contains("entity_not_found"),
                "契约路由 {method} {concrete} 返回 404 且错误码是 not_found：\
                 说明路由**没登记**，而不是资源不存在：{body}"
            );
        }
        // ⚠️ 按**契约模板路径**精确匹配（不是 concrete）：
        //    `concrete` 已把 `{slug}` 替成实例值，与清单里的模板永远不会相等。
        let implemented = IMPLEMENTED.iter().any(|(m, p)| *m == method && *p == path);
        if implemented {
            // 已实现的路由必须**真的**干活：绝不能退回 501。
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

/// 反向断言：派工三条**契约外**路由必须真的挂上（不能只登记在清单里）。
///
/// ⚠️ 与 `extra_routes_list_does_not_leak_into_404_claim` 配对：
/// 那条保证「清单里的路径存在」，这条保证「这三条确实是我新加的、
/// 且都在清单里」—— 只做前者的话，摘掉 handler 而清单留着仍会绿。
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
        // ⚠️ 只有**失败**响应才必须自诊断；成功响应带 next_step 反而是噪声。
        //    POST 那条走的是 400（缺 members），因此同样要求 next_step。
        if !status.is_success() {
            assert!(
                t.contains("next_step"),
                "失败响应必须自诊断（{method} {concrete}，{status}）：{t}"
            );
        }
    }
}

// ─────────────────────────── 405 ───────────────────────────

#[tokio::test]
async fn wrong_method_on_existing_path_is_405_not_404() {
    // 契约表里没有 PUT /api/sessions；路径存在但方法不对。
    // 若返回 404，客户端会误以为"路由不存在"而放弃重试。
    let resp = app!()
        .oneshot(authed("PUT", "/api/sessions", TOKEN_ADMIN))
        .await
        .expect("失败");
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    let text = body_text(resp).await;
    assert!(text.contains("method_not_allowed"), "错误码应明确：{text}");
}

// ─────────────────────────── 500（panic 兜底）───────────────────────────

#[tokio::test]
async fn panic_in_handler_becomes_500_and_leaks_nothing_internally() {
    // 🔴 铁律：panic 必须被兜住，且**不得**把内部细节发给客户端。
    //
    // ⚠️ 走**生产代码里的自检路由**（`Config::enable_selftest` 开启），
    //    而不是在这里自己拼一个 Router —— 只有这样测的才是**真实装配结果**
    //    （含 `build_router` 内部的 layer 顺序），不会与真实行为漂移。
    //    这一点是被实测纠正过的：自建 Router 的初版因为 axum 的
    //    `.layer()` 只覆盖它之前注册的路由，而**漏掉了兜底层**。
    let resp = build_router(state_with_selftest(true))
        .oneshot(req("GET", "/__selftest__/panic"))
        .await
        .expect("oneshot 失败");

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "panic 必须被转成 500，而不是让连接任务崩掉"
    );
    // ⚠️ 先取头再读体：读体会消费掉 response（实测 E0382）。
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
    // 反向断言（铁律十二：豁免必须有反向断言）：
    // 默认（`enable_selftest = false`）时该路由**必须不存在**，
    // 否则等于给外部留了一个可触发的自我 DoS 开关。
    // ⚠️ 与上一条构成配对：那条证明「开着会被兜住」，这条证明「默认关着」。
    //    缺任一条，该机制都只剩一半可证。
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

// ─────────────────────────── 配置降级（铁律七）───────────────────────────

#[tokio::test]
async fn healthz_surfaces_startup_warnings_so_silent_fallback_is_impossible() {
    // 铁律四：静默 ≠ 无痕。配置回退必须对外可见，而不是只写日志。
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
    // ⚠️ 即使目录不存在，服务**照常起**且 /healthz 仍 200 —— 这就是铁律七。
    assert_eq!(resp.status(), StatusCode::OK);
    let text = body_text(resp).await;
    assert!(
        text.contains("\"ui_assets_available\":false"),
        "必须显式报出前端产物不可用：{text}"
    );
}
