mod common;
mod dispatch_seed;

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
use dispatch_seed::{member_session, seed};

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
        llm: Arc::new(RwLock::new(None)),
        llm_config: Arc::new(RwLock::new(Default::default())),
        providers: Arc::new(RwLock::new(Default::default())),
        login_limiter: Arc::new(Default::default()),
        member_control: Default::default(),
        pbkdf2: quill_control::Pbkdf2Params::for_tests(),
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

fn hex(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[tokio::test]
async fn expert_crud_walks_the_full_lifecycle_over_http() {
    let t = TestDb::new("http-expert-crud");
    let app = state(&t);
    let s = state(&t);

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
    // 通用专家是**自动补齐**的：对话页不允许「未选角色就聊天」，所以
    // 一个从没建过专家的用户打开界面也必须至少有一个可选角色。
    // 这里从「断言列表为空」改成「断言恰好是通用专家这一条」——
    // 原断言只钉住「没有别的」，钉不住「有没有它」。
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("响应必须是 JSON");
    let list = parsed["experts"].as_array().expect("experts 必须是数组");
    assert_eq!(list.len(), 1, "新用户应当恰好只有通用专家：{body}");
    assert_eq!(list[0]["id"], "general", "补出来的必须是通用专家：{body}");
    assert_eq!(
        list[0]["is_general"], true,
        "服务端必须自己说哪一个是通用专家，前端不许硬编码 id：{body}"
    );
    assert_eq!(
        list[0]["default_enabled"], true,
        "通用专家必须默认启用，否则对话页下拉里没有它：{body}"
    );
    assert!(
        !list[0]["instructions"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "通用专家的人格正文不许是空串——那等于没建：{body}"
    );

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

    let resp = build_router(app.clone())
        .oneshot(req("GET", "/api/experts/cost-analyst", Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = text(resp).await;
    assert!(body.contains("成本分析师"), "详情应含显示名：{body}");

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

    let resp = build_router(app.clone())
        .oneshot(req("GET", "/api/experts/cost-analyst", Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "已删专家应 404");
    let _ = s;
}

#[tokio::test]
async fn user_b_cannot_see_or_touch_user_a_expert_and_gets_404_not_403() {
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

    let resp = build_router(app.clone())
        .oneshot(req("GET", "/api/experts", Some(TOKEN_B), None))
        .await
        .expect("oneshot 失败");
    let body = text(resp).await;
    assert!(
        !body.contains("secret-expert"),
        "🔴 B 的列表里不得出现 A 的私有专家：{body}"
    );

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

/// Q031 / Q032（2026-10-08）：import / export 真的接通了。
///
/// 这条测试以前叫 `expert_import_export_are_still_501_not_fake_success`，钉的是
/// 「桩不许返回假成功」。路由落地后它的前提没了，按本项目对过期测试的处理口径
/// （`http_contract.rs`：「样本路由必须挑一条仍然是 501 的」）改成钉**新事实**：
/// 两条路由是真 handler（不是空壳 200），且仍然先鉴权。完整往返判据在
/// `expert_bundle_http.rs`。
#[tokio::test]
async fn expert_import_export_are_real_handlers_not_a_fake_success() {
    let t = TestDb::new("http-expert-bundle-real");
    let app = state(&t);

    // 未带令牌：鉴权在 handler 之前，两条都必须 401。
    for (method, path) in [
        ("POST", "/api/experts/import"),
        ("GET", "/api/experts/export"),
    ] {
        let resp = build_router(app.clone())
            .oneshot(req(method, path, None, None))
            .await
            .expect("oneshot 失败");
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path} 未带令牌必须 401"
        );
    }

    // export 是真实现：200 且带清单版本，不是空壳。
    let resp = build_router(app.clone())
        .oneshot(req("GET", "/api/experts/export", Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK, "导出必须 200");
    let body = text(resp).await;
    assert!(
        body.contains("\"bundle_version\":1"),
        "导出必须带清单版本：{body}"
    );

    // import 是真实现：没有 JSON 体的请求必须 400（不是 501，也不是静默 200）。
    let resp = build_router(app.clone())
        .oneshot(req("POST", "/api/experts/import", Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "没有 JSON 体的导入必须 400（否则就是静默成功）"
    );
}

#[tokio::test]
async fn unauthenticated_expert_requests_are_401_before_anything_else() {
    let t = TestDb::new("http-expert-401");
    let resp = build_router(state(&t))
        .oneshot(req("GET", "/api/experts", None, None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn dispatch_booking_is_idempotent_and_visible_in_the_ledger() {
    let t = TestDb::new("http-dispatch-book");

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

/// 2026-10-07：派工必须先确认团队真的存在。
///
/// 之前路径里的 `{id}` 解析完就直接进了台账，于是**编一个不存在的团队 id**
/// 也能拿到 200 与一份空记录 —— 读起来像「这个团队没有派工历史」，
/// 台账里却多出一条指向虚空的派工，而且没有任何地方消费它。
///
/// 这条钉的是 404，且顺带确认它**不是** 400（400 会被读成「参数写错了」，
/// 而这里的真相是「这个团队没有」）。
#[tokio::test]
async fn dispatch_to_a_team_that_does_not_exist_is_404_and_writes_nothing() {
    let t = TestDb::new("http-dispatch-unknown-team");
    // 真的种一个团队进去，让「不存在的那个」确实是另一个 id。
    let f = seed(&t.bridge(), user_id(UID_A), 0x23, &["cost-analyst"]);
    let app = state(&t);

    let ghost = {
        let mut id = f.team_id;
        id[15] ^= 0xff; // 合法长度、合法十六进制，但不是任何一个团队
        format!("/api/teams/{}/dispatch", hex(&id))
    };

    let resp = build_router(app)
        .oneshot(req(
            "POST",
            &ghost,
            Some(TOKEN_A),
            Some(serde_json::json!({
                "room_id": f.room_id,
                "round": 0,
                "leader_session_id": f.leader_session.to_compact_hex(),
                "members": [{
                    "expert": "cost-analyst",
                    "member": "cost-analyst-1",
                    "member_session_id": member_session(0x23, &expert_id("cost-analyst")).to_compact_hex()
                }]
            })),
        ))
        .await
        .expect("oneshot 失败");

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "团队不存在必须是 404，而不是 200 空记录"
    );
    let body = text(resp).await;
    assert!(
        body.contains("不存在") || body.contains("404"),
        "应说明是团队不存在：{body}"
    );
}

/// 同一条边界，GET 侧也必须成立。
///
/// GET 这条路由原先是 `Path(_team)` —— 下划线前缀、**直接丢弃**。于是
/// 问一个编出来的团队 id 会拿到 200 与一份空记录，读起来像
/// 「这个团队没有派工历史」—— 那是在把「没有这个东西」说成「它没有记录」。
///
/// 判据有两条，别只测一条：
///   · 不存在的团队 → 404
///   · 存在的团队 → 200，且响应里带上 `team_id` 与 `filtered_by`，
///     让调用方知道路径里的 {id} 真的参与了判定，以及列表按什么过滤。
#[tokio::test]
async fn listing_dispatch_of_a_missing_team_is_404_not_an_empty_200() {
    let t = TestDb::new("http-dispatch-list-unknown-team");
    let f = seed(&t.bridge(), user_id(UID_A), 0x24, &["cost-analyst"]);
    let app = state(&t);

    let mut ghost = f.team_id;
    ghost[15] ^= 0xff;
    let path = format!(
        "/api/teams/{}/dispatch?room_id={}&round=0",
        hex(&ghost),
        f.room_id
    );

    let resp = build_router(app.clone())
        .oneshot(req("GET", &path, Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "GET 侧也必须 404：不存在 ≠ 没有记录"
    );
    let body = text(resp).await;
    assert!(
        !body.contains("\"count\""),
        "404 里不该出现记录条数：{body}"
    );

    // 反过来：真团队仍然 200，并如实说明自己按什么过滤。
    let ok_path = format!(
        "/api/teams/{}/dispatch?room_id={}&round=0",
        hex(&f.team_id),
        f.room_id
    );
    let resp = build_router(app)
        .oneshot(req("GET", &ok_path, Some(TOKEN_A), None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = text(resp).await;
    assert!(
        body.contains(&hex(&f.team_id)),
        "响应应回显路径里的 team_id：{body}"
    );
    assert!(
        body.contains("\"filtered_by\":\"room_id+round\""),
        "应如实说明列表按 room_id+round 过滤，而不是让调用方以为按团队过滤：{body}"
    );
}
