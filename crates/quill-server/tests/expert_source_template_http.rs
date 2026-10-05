//! 专家 source_template 的 HTTP 契约。
//!
//! 覆盖四件最容易悄悄坏掉的事：省略不等于清空、显式 null 才清空、
//! 非法模板 id 必须 400 且带「下一步」、以及派生关系能从列表接口读回来。

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
const TOKEN_A: &str = "tok-a";

fn user_id() -> quill_domain::UserId {
    quill_domain::UserId::parse(UID_A).expect("测试 UID 必须合法")
}

fn state(t: &TestDb) -> AppState {
    let resolver = EnvTokenResolver::new(vec![(
        TOKEN_A.to_string(),
        AuthContext {
            user_id: user_id(),
            is_admin: true,
        },
    )]);
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

fn req(method: &str, path: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN_A}"));
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

fn json(t: &str) -> serde_json::Value {
    serde_json::from_str(t).unwrap_or_else(|e| panic!("响应必须是 JSON（{e}）：{t}"))
}

async fn post_expert(app: &AppState, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let resp = build_router(app.clone())
        .oneshot(req("POST", "/api/experts", Some(body)))
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    (status, json(&text(resp).await))
}

#[tokio::test]
async fn post_creates_a_derived_expert_and_omission_means_no_source() {
    let t = TestDb::new("source-create");
    let app = state(&t);

    let (status, v) = post_expert(
        &app,
        serde_json::json!({
            "id": "prog-1",
            "display_name": "程序员1号",
            "description": "从 ai-coding-coach 派生",
            "source_template": "ai-coding-coach"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "创建应 201");
    assert_eq!(
        v["source_template"],
        serde_json::json!("ai-coding-coach"),
        "回包里必须带回来派生来源：{v}"
    );

    let (status, v) = post_expert(
        &app,
        serde_json::json!({
            "id": "handmade",
            "display_name": "手建专家",
            "description": "不来自任何模板"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(
        v.as_object().expect("必须是对象").contains_key("source_template"),
        "省略该字段时也必须出现这个键（缺失会被前端当成后端没做）"
    );
    assert!(
        v["source_template"].is_null(),
        "省略 = 不来自模板（NULL），实际 {}",
        v["source_template"]
    );
}

#[tokio::test]
async fn one_template_can_back_many_experts_and_the_list_endpoint_reads_it_back() {
    let t = TestDb::new("source-many");
    let app = state(&t);

    for (id, name) in [("prog-1", "程序员1号"), ("prog-2", "程序员2号")] {
        let (status, _) = post_expert(
            &app,
            serde_json::json!({
                "id": id,
                "display_name": name,
                "description": "同一个模板派生出来的",
                "source_template": "ai-coding-coach"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{id} 应能创建");
    }

    let resp = build_router(app)
        .oneshot(req("GET", "/api/experts", None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json(&text(resp).await);
    let experts = v["experts"]
        .as_array()
        .unwrap_or_else(|| panic!("必须是 experts 数组：{v}"));
    let derived: Vec<&str> = experts
        .iter()
        .filter(|e| e["source_template"] == serde_json::json!("ai-coding-coach"))
        .map(|e| e["id"].as_str().expect("id 是字符串"))
        .collect();
    assert_eq!(
        derived.len(),
        2,
        "🔴 一个模板派生出的多个专家都必须能从列表接口读回来：{v}"
    );
    assert!(derived.contains(&"prog-1") && derived.contains(&"prog-2"), "{v}");
}

#[tokio::test]
async fn patching_another_field_leaves_source_template_alone_and_null_clears_it() {
    let t = TestDb::new("source-patch");
    let app = state(&t);
    post_expert(
        &app,
        serde_json::json!({
            "id": "prog-1",
            "display_name": "程序员1号",
            "description": "从模板派生",
            "source_template": "ai-coding-coach"
        }),
    )
    .await;

    let resp = build_router(app.clone())
        .oneshot(req(
            "PATCH",
            "/api/experts/prog-1",
            Some(serde_json::json!({ "description": "只改描述" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json(&text(resp).await);
    assert_eq!(
        v["source_template"],
        serde_json::json!("ai-coding-coach"),
        "🔴 PATCH 省略 source_template 时必须沿用原值（部分更新，不是覆盖）"
    );

    let resp = build_router(app.clone())
        .oneshot(req(
            "PATCH",
            "/api/experts/prog-1",
            Some(serde_json::json!({ "source_template": null })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json(&text(resp).await);
    assert!(
        v["source_template"].is_null(),
        "显式 null 必须清除来源模板，实际 {}",
        v["source_template"]
    );

    let resp = build_router(app)
        .oneshot(req("GET", "/api/experts/prog-1", None))
        .await
        .expect("oneshot 失败");
    let v = json(&text(resp).await);
    assert!(v["source_template"].is_null(), "清除必须真的落库：{v}");
}

#[tokio::test]
async fn an_illegal_source_template_is_400_with_a_next_step() {
    let t = TestDb::new("source-badinput");
    let app = state(&t);

    let (status, v) = post_expert(
        &app,
        serde_json::json!({
            "id": "prog-1",
            "display_name": "程序员1号",
            "description": "模板 id 写错了",
            "source_template": "AI Coding Coach"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "非法模板 id 必须 400：{v}");
    let body = v.to_string();
    assert!(body.contains("下一步"), "错误必须带修复方向：{body}");
    assert!(
        body.contains("source_template"),
        "应点名出错的字段：{body}"
    );
    assert!(body.contains("next_step"), "响应体结构必须带下一步字段：{body}");

    let (status, v) = post_expert(
        &app,
        serde_json::json!({
            "id": "prog-1",
            "display_name": "程序员1号",
            "description": "模板 id 写成了驼峰",
            "source_template": "aiCodingCoach"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "驼峰模板 id 必须 400：{v}");

    let resp = build_router(app.clone())
        .oneshot(req(
            "PATCH",
            "/api/experts/prog-1",
            Some(serde_json::json!({ "source_template": "ai_coding" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "专家还不存在，不该先判模板 id 非法"
    );

    post_expert(
        &app,
        serde_json::json!({
            "id": "prog-1",
            "display_name": "程序员1号",
            "description": "先建好"
        }),
    )
    .await;
    let resp = build_router(app.clone())
        .oneshot(req(
            "PATCH",
            "/api/experts/prog-1",
            Some(serde_json::json!({ "source_template": "ai_coding" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "PATCH 也要 400");
    let body = text(resp).await;
    assert!(body.contains("下一步"), "错误必须带修复方向：{body}");

    let resp = build_router(app)
        .oneshot(req(
            "PATCH",
            "/api/experts/prog-1",
            Some(serde_json::json!({ "sourceTemplate": "ai-coding-coach" })),
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "字段名拼错必须判红（静默忽略会让人以为来源记上了）"
    );
    let body = text(resp).await;
    assert!(body.contains("sourceTemplate"), "应点名是哪个字段：{body}");
    assert!(body.contains("source_template"), "应列出可接受字段：{body}");
}
