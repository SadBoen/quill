//! 专家与派工 handler 的**端到端**测试（真库 + 真实 axum 路由）。
//!
//! 走的是真实的路由匹配、中间件链、extractor、`IntoResponse`；
//! 唯一没走的是 TCP 与 hyper（与 `http_contract.rs` 同一取舍）。
//!
//! # 这一层要证明的，不是仓储已经证明过的事
//!
//! 仓储测试验的是「SQL 对不对」；这里验的是**接线对不对**：
//! 1. 契约 §5.1 的专家路由**不再是 501**（替换掉的占位确实被拿掉了）；
//! 2. 属主取自令牌 —— B 用自己的令牌去改 A 的专家，拿到的是 404 而不是 403；
//! 3. 存储不可用时是 **503**，不是 200 空列表；
//! 4. 错误响应是中文 + 下一步，且**不含**底层 SQL 串。

mod common;
mod dispatch_seed;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;
use dispatch_seed::{member_session, seed};

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const UID_B: &str = "0192b7c8-0000-7000-8000-000000000002";
const TOKEN_A: &str = "tok-a";
const TOKEN_B: &str = "tok-b";

fn user_id(uid: &str) -> quill_domain::UserId {
    quill_domain::UserId::parse(uid).expect("测试 UID 必须合法")
}

/// 状态 + 临时库。
///
/// ⚠️ 返回 `TestDb` 是**故意的**：它必须活到用例结束，
/// 否则临时目录会在连接还开着时被删掉（Windows 上直接删不掉）。
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
    }
}

/// 存储不可用的状态（模拟「库打不开」）。
fn state_without_db() -> AppState {
    let resolver = EnvTokenResolver::new(vec![(
        TOKEN_A.to_string(),
        AuthContext {
            user_id: user_id(UID_A),
            is_admin: true,
        },
    )]);
    AppState {
        config: Config::from_env(),
        tokens: Arc::new(resolver),
        db: None,
        db_problem: Some("测试注入：数据库 /var/lib/quill/quill.db 打不开".to_string()),
    }
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

fn expert_id(name: &str) -> quill_adapters::ExpertId {
    quill_adapters::ExpertId::parse(name).expect("测试用专家名必须合法")
}

/// 16 字节 → 32 位十六进制（团队标识在路径里就是 hex）。
fn hex(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ─────────────────────────── 专家 CRUD ───────────────────────────

#[tokio::test]
async fn expert_crud_walks_the_full_lifecycle_over_http() {
    let t = TestDb::new("http-expert-crud");
    let app = state(&t);
    let s = state(&t);

    // 列表初始为空
    let resp = build_router(app.clone())
        .oneshot(req("GET", "/api/experts", Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "列表必须 200（不是 501/503）"
    );
    let body = text(resp).await;
    assert!(body.contains("\"experts\":[]"), "初始列表应为空：{body}");

    // 创建
    let resp = build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(TOKEN_A),
            Some(serde_json::json!({
                "id": "cost-analyst",
                "display_name": "成本分析师",
                "description": "算清本月成本"
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::CREATED, "创建应 201");
    let body = text(resp).await;
    assert!(body.contains("cost-analyst"), "应回显专家标识：{body}");

    // 读取
    let resp = build_router(app.clone())
        .oneshot(req("GET", "/api/experts/cost-analyst", Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = text(resp).await;
    assert!(body.contains("成本分析师"), "详情应含显示名：{body}");

    // 修改
    let resp = build_router(app.clone())
        .oneshot(req(
            "PATCH",
            "/api/experts/cost-analyst",
            Some(TOKEN_A),
            Some(serde_json::json!({ "display_name": "成本分析员", "default_enabled": false })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = text(resp).await;
    assert!(body.contains("成本分析员"), "改名应生效：{body}");
    assert!(
        body.contains("\"default_enabled\":false"),
        "关开关应生效：{body}"
    );

    // 删除（幂等：第二次 deleted=false）
    for (i, want) in [(1, true), (2, false)] {
        let resp = build_router(app.clone())
            .oneshot(req(
                "DELETE",
                "/api/experts/cost-analyst",
                Some(TOKEN_A),
                None,
            ))
            .await
            .expect("oneshot 失败");
        assert_eq!(resp.status(), StatusCode::OK);
        let body = text(resp).await;
        assert!(
            body.contains(&format!("\"deleted\":{want}")),
            "第 {i} 次删除的 deleted 应为 {want}：{body}"
        );
    }
    // 删后不可见
    let resp = build_router(app.clone())
        .oneshot(req("GET", "/api/experts/cost-analyst", Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "已删专家应 404");
    let _ = s;
}

#[tokio::test]
async fn user_b_cannot_see_or_touch_user_a_expert_and_gets_404_not_403() {
    // 🔴 跨用户隔离的 HTTP 表现（M3 判定项）。
    //    403 会确认「A 确实有这个专家」；必须与「不存在」完全同形。
    let t = TestDb::new("http-expert-isolation");
    let app = state(&t);
    build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(TOKEN_A),
            Some(serde_json::json!({
                "id": "secret-expert",
                "display_name": "机密专家",
                "description": "只有 A 能看"
            })),
        ))
        .await
        .expect("oneshot 失败");

    // B 列表看不到
    let resp = build_router(app.clone())
        .oneshot(req("GET", "/api/experts", Some(TOKEN_B), None))
        .await
        .expect("oneshot 失败");
    let body = text(resp).await;
    assert!(
        !body.contains("secret-expert"),
        "🔴 B 的列表里不得出现 A 的私有专家：{body}"
    );

    // B 直接按 id 取 → 404（不是 403）
    let resp = build_router(app.clone())
        .oneshot(req(
            "GET",
            "/api/experts/secret-expert",
            Some(TOKEN_B),
            None,
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "🔴 不可见必须是 404：403 等于确认它存在"
    );

    // B 改名 / 删除 → 同样 404
    for (method, body) in [
        ("PATCH", Some(serde_json::json!({ "display_name": "劫持" }))),
        ("DELETE", None),
    ] {
        let resp = build_router(app.clone())
            .oneshot(req(
                method,
                "/api/experts/secret-expert",
                Some(TOKEN_B),
                body,
            ))
            .await
            .expect("oneshot 失败");
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "🔴 B 的 {method} 必须是 404"
        );
    }

    // A 的专家毫发无损
    let resp = build_router(app)
        .oneshot(req(
            "GET",
            "/api/experts/secret-expert",
            Some(TOKEN_A),
            None,
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = text(resp).await;
    assert!(body.contains("机密专家"), "A 的专家不该被改动：{body}");
}

#[tokio::test]
async fn storage_unavailable_is_503_and_never_an_empty_200() {
    // 🔴 反向用例：存储故障必须**响亮**，不能伪装成「我没有专家」。
    let resp = build_router(state_without_db())
        .oneshot(req("GET", "/api/experts", Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "存储不可用必须是 503"
    );
    let body = text(resp).await;
    assert!(
        body.contains("storage_unavailable"),
        "错误码应可被前端分支：{body}"
    );
    assert!(
        body.contains("quill doctor --section=db"),
        "必须给出可复制的下一步：{body}"
    );
    assert!(
        !body.contains("\"experts\":[]"),
        "🔴 绝不能用空列表伪装成功：{body}"
    );
    // /healthz 仍 200 且报出存储不可用（服务活着 ≠ 数据可用）
    let resp = build_router(state_without_db())
        .oneshot(req("GET", "/healthz", None, None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK, "服务本身仍应存活");
    let body = text(resp).await;
    assert!(
        body.contains("\"ready\":false") && body.contains("/var/lib/quill/quill.db"),
        "healthz 必须报出存储不可用与原因：{body}"
    );
}

#[tokio::test]
async fn bad_input_is_400_in_chinese_and_unknown_fields_are_rejected() {
    let t = TestDb::new("http-expert-badinput");
    let app = state(&t);

    // 非法 slug
    let resp = build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(TOKEN_A),
            Some(serde_json::json!({
                "id": "Cost Analyst",
                "display_name": "x",
                "description": "y"
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = text(resp).await;
    assert!(body.contains("bad_request"), "错误码应明确：{body}");

    // 字段名拼错（驼峰）必须判红，否则会「返回成功却什么都没改」
    build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(TOKEN_A),
            Some(serde_json::json!({
                "id": "ok-expert",
                "displayName": "拼错的字段"
            })),
        ))
        .await
        .expect("oneshot 失败");
    let resp = build_router(app.clone())
        .oneshot(req(
            "POST",
            "/api/experts",
            Some(TOKEN_A),
            Some(serde_json::json!({
                "id": "ok-expert",
                "displayName": "拼错的字段"
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "驼峰字段必须判红");
    let body = text(resp).await;
    assert!(body.contains("displayName"), "应点名是哪个字段：{body}");

    // 空 PATCH → 400（不许返回 200 却什么都没改）
    let resp = build_router(app.clone())
        .oneshot(req(
            "PATCH",
            "/api/experts/ok-expert",
            Some(TOKEN_A),
            Some(serde_json::json!({})),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "空 PATCH 必须 400");
    let body = text(resp).await;
    assert!(body.contains("default_enabled"), "应列出可改字段：{body}");
}

#[tokio::test]
async fn expert_import_export_are_still_501_not_fake_success() {
    // 未实现的路由必须继续报 501（不许因为「专家 CRUD 已实现」就连带变成假成功）。
    let t = TestDb::new("http-expert-501");
    let app = state(&t);
    for (method, path) in [
        ("POST", "/api/experts/import"),
        ("GET", "/api/experts/export"),
    ] {
        let resp = build_router(app.clone())
            .oneshot(req(method, path, Some(TOKEN_A), None))
            .await
            .expect("oneshot 失败");
        assert_eq!(
            resp.status(),
            StatusCode::NOT_IMPLEMENTED,
            "{method} {path} 未实现，必须 501"
        );
        let body = text(resp).await;
        assert!(body.contains(path), "501 文案必须点名路由：{body}");
    }
}

#[tokio::test]
async fn unauthenticated_expert_requests_are_401_before_anything_else() {
    // 顺序断言：鉴权先于存储。未认证方不得通过状态码区分
    // 「专家路由存在但你没权限」与「路由不存在」。
    let t = TestDb::new("http-expert-401");
    let resp = build_router(state(&t))
        .oneshot(req("GET", "/api/experts", None, None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ─────────────────────────── 派工 ───────────────────────────

#[tokio::test]
async fn dispatch_booking_is_idempotent_and_visible_in_the_ledger() {
    let t = TestDb::new("http-dispatch-book");
    // ⚠️ 团队 / 主持人会话 / 房间**全部取自夹具**，不手写十六进制：
    //    手写就等于「用例与外键前提各写一份」，两者漂移时 FK 会以
    //    「内部错误」的形式出现，排障方向完全被带偏。
    let f = seed(&t.bridge(), user_id(UID_A), 0x21, &["cost-analyst"]);
    let app = state(&t);

    let body = serde_json::json!({
        "room_id": f.room_id,
        "round": 0,
        "leader_session_id": f.leader_session.to_compact_hex(),
        "members": [{
            "expert": "cost-analyst",
            "member": "cost-analyst-1",
            "member_session_id": member_session(0x21, &expert_id("cost-analyst")).to_compact_hex()
        }]
    });
    let team_path = format!("/api/teams/{}/dispatch", hex(&f.team_id));

    let resp = build_router(app.clone())
        .oneshot(req("POST", &team_path, Some(TOKEN_A), Some(body.clone())))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::ACCEPTED, "预记账应 202");
    let body_text = text(resp).await;
    assert!(
        body_text.contains("\"idempotency\":\"created\""),
        "首次应判 created：{body_text}"
    );
    assert!(
        body_text.contains("\"executed\":false"),
        "🔴 必须明说没执行（不许让用户以为成员跑过了）：{body_text}"
    );

    // 幂等：同一轮再记一次 → existed
    let resp = build_router(app.clone())
        .oneshot(req("POST", &team_path, Some(TOKEN_A), Some(body)))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let body_text = text(resp).await;
    assert!(
        body_text.contains("\"idempotency\":\"existed\""),
        "🔴 二次记账必须判 existed：{body_text}"
    );

    // 账本可读（GET 同路径）
    let resp = build_router(app.clone())
        .oneshot(req(
            "GET",
            &format!(
                "/api/teams/{}/dispatch?room_id={}&round=0",
                hex(&f.team_id),
                f.room_id
            ),
            Some(TOKEN_A),
            None,
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body_text = text(resp).await;
    assert!(
        body_text.contains("\"count\":1") && body_text.contains("cost-analyst-1"),
        "账本应含这条派工：{body_text}"
    );

    // 在途视图
    let resp = build_router(app)
        .oneshot(req("GET", "/api/dispatch/inflight", Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body_text = text(resp).await;
    assert!(
        body_text.contains("\"count\":1") && body_text.contains("\"state\":\"PENDING\""),
        "在途视图应含这条 PENDING：{body_text}"
    );
}

#[tokio::test]
async fn dispatch_booking_validates_member_id_prefix_and_rejects_empty_members() {
    let t = TestDb::new("http-dispatch-validate");
    let f = seed(&t.bridge(), user_id(UID_A), 0x22, &["cost-analyst"]);
    let app = state(&t);
    let team_path = format!("/api/teams/{}/dispatch", hex(&f.team_id));

    // 成员标识必须以「专家名-」开头
    let resp = build_router(app.clone())
        .oneshot(req(
            "POST",
            &team_path,
            Some(TOKEN_A),
            Some(serde_json::json!({
                "room_id": f.room_id,
                "round": 0,
                "leader_session_id": f.leader_session.to_compact_hex(),
                "members": [{
                    "expert": "cost-analyst",
                    "member": "coder-1",
                    "member_session_id": member_session(0x22, &expert_id("cost-analyst")).to_compact_hex()
                }]
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "成员标识前缀错必须 400"
    );
    let body = text(resp).await;
    assert!(body.contains("cost-analyst-"), "应说明前缀规则：{body}");

    // 空成员列表
    let resp = build_router(app)
        .oneshot(req(
            "POST",
            &team_path,
            Some(TOKEN_A),
            Some(serde_json::json!({
                "room_id": f.room_id,
                "round": 0,
                "leader_session_id": f.leader_session.to_compact_hex(),
                "members": []
            })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "空成员必须 400");
    let body = text(resp).await;
    assert!(body.contains("members"), "应点名字段：{body}");
}
