//! 专家团的 HTTP 契约。
//!
//! 覆盖最容易悄悄坏掉的几件：成员下限 2、上限 8、不许嵌套团队、成员必须在
//! 本用户名册里、列表只回自己的、软删后再 GET 是 404、二次删除幂等。

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

/// teams / team_members / sessions 都对 users 有外键，真实服务靠启动时按
/// QUILL_TOKENS 建档，测试里得自己铺。
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

/// 建若干个属于 uid 的普通专家（走真实 HTTP 接口，斜杠也顺带被覆盖到）。
async fn make_experts(app: &AppState, experts: &[&str]) {
    for e in experts {
        let resp = build_router(app.clone())
            .oneshot(req(
                "POST",
                "/api/experts",
                TOKEN_A,
                Some(serde_json::json!({
                    "id": e,
                    "display_name": e,
                    "description": "测试专家"
                })),
            ))
            .await
            .expect("oneshot 失败");
        let status = resp.status();
        let text = text(resp).await;
        assert_eq!(status, StatusCode::CREATED, "建专家 {e} 应 201：{text}");
    }
}

fn req(method: &str, path: &str, token: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    if body.is_some() {
        b = b.header("content-type", application_json());
    }
    b.body(match body {
        Some(v) => Body::from(v.to_string()),
        None => Body::empty(),
    })
    .expect("构造请求失败")
}

fn application_json() -> &'static str {
    "application/json"
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
    let resp = build_router(app.clone())
        .oneshot(req(method, path, token, body))
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    (status, json(&text(resp).await))
}

fn team_body(leader: &str, members: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "team_id": "growth-squad",
        "name": "增长小队",
        "description": "盯住获客成本",
        "leader_id": leader,
        "member_ids": members,
    })
}

fn fixture(label: &str) -> (TestDb, AppState) {
    let t = TestDb::new(label);
    seed_user(&t.bridge(), UID_A);
    seed_user(&t.bridge(), UID_B);
    let app = state(&t);
    (t, app)
}

#[tokio::test]
async fn creating_a_team_returns_201_and_the_stored_shape() {
    let (_t, app) = fixture("team-create");
    make_experts(
        &app,
        &["cost-analyst", "growth-analyst", "risk-reviewer"],
    )
    .await;

    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body(
            "cost-analyst",
            &["risk-reviewer", "growth-analyst"],
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "建团应 201：{v}");
    assert_eq!(v["team_id"], serde_json::json!("growth-squad"));
    assert_eq!(v["name"], serde_json::json!("增长小队"));
    assert_eq!(v["description"], serde_json::json!("盯住获客成本"));
    assert_eq!(v["leader_id"], serde_json::json!("cost-analyst"));
    assert_eq!(
        v["member_ids"],
        serde_json::json!(["growth-analyst", "risk-reviewer"]),
        "🔴 member_ids 必须按 id 升序且不含主持人：{v}"
    );
    assert!(v["created_at"].as_i64().unwrap_or(0) > 0, "必须带回时间戳：{v}");

    // 回读：创建后 GET 拿到的必须与创建回包一致（写路径真的落库了）。
    let (status, got) = call(&app, "GET", "/api/teams/growth-squad", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK, "读回应 200：{got}");
    assert_eq!(got, v, "GET 回来的团队必须与 POST 回包逐字段一致");

    // 主持人是一个普通专家、有自己的会话，且不计入成员数。
    let (status, list) = call(&app, "GET", "/api/teams", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK);
    let teams = list["teams"].as_array().expect("必须是 teams 数组");
    assert_eq!(teams.len(), 1);
    assert_eq!(
        teams[0]["member_ids"]
            .as_array()
            .expect("必须是数组")
            .len(),
        2,
        "主持人不计入成员数"
    );
}

#[tokio::test]
async fn fewer_than_two_members_is_400_with_a_next_step() {
    let (_t, app) = fixture("team-too-few");
    make_experts(&app, &["cost-analyst", "growth-analyst", "risk-reviewer"]).await;

    for members in [
        vec![],
        vec!["growth-analyst"],
    ] {
        let (status, v) = call(
            &app,
            "POST",
            "/api/teams",
            TOKEN_A,
            Some(team_body("cost-analyst", &members)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "成员不足 2 必须 400：{v}");
        let body = v.to_string();
        assert!(body.contains("下一步"), "错误必须带修复方向：{body}");
        assert!(body.contains("member_ids"), "应点名出错的字段：{body}");
    }

    let (status, v) = call(&app, "GET", "/api/teams", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        v["teams"].as_array().map(Vec::len),
        Some(0),
        "被拒的建团不得留下半写状态：{v}"
    );
}

#[tokio::test]
async fn more_than_eight_members_is_400() {
    let (_t, app) = fixture("team-too-many");
    let roster = [
        "e0", "e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9", "e10", "e11", "e12", "lead",
    ];
    make_experts(&app, &roster).await;

    // 主持人是「lead」，不在 member_ids 里（主持人不计入成员数）。
    let eight: Vec<&str> = roster[..8].to_vec();
    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(serde_json::json!({
            "team_id": "big-squad",
            "name": "大团",
            "leader_id": "lead",
            "member_ids": eight,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "8 个成员是上限本身，必须放行：{v}");

    let nine: Vec<&str> = roster[..9].to_vec();
    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(serde_json::json!({
            "team_id": "bigger-squad",
            "name": "更大的团",
            "leader_id": "lead",
            "member_ids": nine,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "9 个成员必须 400：{v}");
    let body = v.to_string();
    assert!(body.contains('9') && body.contains('8'), "应给出实际值与上限：{body}");
}

/// 主持人不计入成员数，因此也不能同时出现在 member_ids 里
/// （否则库里会出现同一专家的 leader 行与 member 行两行）。
#[tokio::test]
async fn the_leader_may_not_also_be_listed_as_a_member() {
    let (_t, app) = fixture("team-leader-as-member");
    make_experts(&app, &["cost-analyst", "growth-analyst", "risk-reviewer"]).await;

    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body(
            "cost-analyst",
            &["cost-analyst", "growth-analyst"],
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "主持人当成员必须 400：{v}");
    let body = v.to_string();
    assert!(body.contains("cost-analyst"), "应点名是谁：{body}");
    assert!(body.contains("下一步"), "{body}");
}

#[tokio::test]
async fn a_duplicate_member_is_400() {
    let (_t, app) = fixture("team-dup");
    make_experts(&app, &["cost-analyst", "growth-analyst", "risk-reviewer"]).await;

    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body(
            "cost-analyst",
            &["growth-analyst", "growth-analyst"],
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "重复成员必须 400：{v}");
    assert!(
        v.to_string().contains("growth-analyst"),
        "应点名重复的是谁：{v}"
    );
}

#[tokio::test]
async fn nesting_a_team_as_a_member_is_400() {
    let (_t, app) = fixture("team-nested");
    make_experts(
        &app,
        &["cost-analyst", "growth-analyst", "risk-reviewer", "growth-squad"],
    )
    .await;

    // growth-squad 这个「专家」与本团队同 id：模拟「把团队当成员塞进来」。
    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body(
            "cost-analyst",
            &["growth-analyst", "growth-squad"],
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "嵌套团队必须 400：{v}");
    let body = v.to_string();
    assert!(body.contains("嵌套"), "应说明是不支持嵌套团队：{body}");
    assert!(body.contains("growth-squad"), "应点名被当成成员的那个 id：{body}");
}

#[tokio::test]
async fn a_member_outside_my_roster_is_400() {
    let (_t, app) = fixture("team-unknown-expert");
    make_experts(&app, &["cost-analyst", "growth-analyst"]).await;

    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body("cost-analyst", &["growth-analyst", "ghost-expert"])),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "名册外的专家必须 400：{v}");
    assert!(v.to_string().contains("ghost-expert"), "{v}");

    // 主持人同样必须是名册里的普通专家。
    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body(
            "ghost-leader",
            &["growth-analyst", "cost-analyst"],
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "名册外的主持人必须 400：{v}");
    assert!(v.to_string().contains("ghost-leader"), "{v}");
}

#[tokio::test]
async fn a_taken_team_id_is_409() {
    let (_t, app) = fixture("team-conflict");
    make_experts(
        &app,
        &["cost-analyst", "growth-analyst", "risk-reviewer"],
    )
    .await;
    let body = team_body("cost-analyst", &["growth-analyst", "risk-reviewer"]);
    let (status, _) = call(&app, "POST", "/api/teams", TOKEN_A, Some(body.clone())).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, v) = call(&app, "POST", "/api/teams", TOKEN_A, Some(body)).await;
    assert_eq!(status, StatusCode::CONFLICT, "标识被占用必须 409：{v}");
    assert_eq!(v["error"]["code"], serde_json::json!("conflict"));
    assert!(v.to_string().contains("growth-squad"), "{v}");
}

#[tokio::test]
async fn the_list_only_shows_my_own_teams() {
    let (_t, app) = fixture("team-isolation");
    make_experts(
        &app,
        &["cost-analyst", "growth-analyst", "risk-reviewer"],
    )
    .await;
    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body("cost-analyst", &["growth-analyst", "risk-reviewer"])),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");

    // 另一个用户看不到、也读不到这个团队。
    let (status, list) = call(&app, "GET", "/api/teams", TOKEN_B, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list["teams"].as_array().map(Vec::len),
        Some(0),
        "别人的团队不得出现在我的列表里：{list}"
    );
    let (status, v) = call(&app, "GET", "/api/teams/growth-squad", TOKEN_B, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "读别人的团队按「不存在」口径：{v}");
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));

    let (status, v) = call(
        &app,
        "DELETE",
        "/api/teams/growth-squad",
        TOKEN_B,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "不得删掉别人的团队：{v}");
}

#[tokio::test]
async fn patching_replaces_the_member_set_and_keeps_the_rest() {
    let (_t, app) = fixture("team-patch");
    make_experts(
        &app,
        &[
            "cost-analyst",
            "growth-analyst",
            "risk-reviewer",
            "tech-writer",
            "legal-reviewer",
        ],
    )
    .await;
    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body(
            "cost-analyst",
            &["growth-analyst", "risk-reviewer"],
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    let created_at = v["created_at"].clone();

    let (status, v) = call(
        &app,
        "PATCH",
        "/api/teams/growth-squad",
        TOKEN_A,
        Some(serde_json::json!({ "name": "增长二队" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "只改名字应 200：{v}");
    assert_eq!(v["name"], serde_json::json!("增长二队"));
    assert_eq!(
        v["member_ids"],
        serde_json::json!(["growth-analyst", "risk-reviewer"]),
        "🔴 省略 member_ids 时必须沿用原成员（部分更新，不是覆盖）：{v}"
    );
    assert_eq!(v["created_at"], created_at, "原始创建时间不得被改写");

    let (status, v) = call(
        &app,
        "PATCH",
        "/api/teams/growth-squad",
        TOKEN_A,
        Some(serde_json::json!({ "member_ids": ["tech-writer", "legal-reviewer"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "换成员应 200：{v}");
    assert_eq!(
        v["member_ids"],
        serde_json::json!(["legal-reviewer", "tech-writer"]),
        "成员集合必须被整体替换（而不是只增不减）：{v}"
    );

    // 换人也换主持人：主持人不计入成员数，所以新主持人不能是现有成员。
    let (status, v) = call(
        &app,
        "PATCH",
        "/api/teams/growth-squad",
        TOKEN_A,
        Some(serde_json::json!({ "leader_id": "growth-analyst" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "换主持人应 200：{v}");
    assert_eq!(v["leader_id"], serde_json::json!("growth-analyst"));
    assert_eq!(
        v["member_ids"],
        serde_json::json!(["legal-reviewer", "tech-writer"]),
        "换主持人不得动成员集合：{v}"
    );

    // 把当前成员提为主持人（同一人不得既是主持人又是成员）。
    let (status, v) = call(
        &app,
        "PATCH",
        "/api/teams/growth-squad",
        TOKEN_A,
        Some(serde_json::json!({ "leader_id": "tech-writer" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "成员当主持人必须 400：{v}");

    // 换主持人后把成员减到 1 人必须被拒。
    let (status, v) = call(
        &app,
        "PATCH",
        "/api/teams/growth-squad",
        TOKEN_A,
        Some(serde_json::json!({ "member_ids": ["legal-reviewer"] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "成员减到 1 必须 400：{v}");
    assert!(v.to_string().contains("下一步"), "{v}");

    // 显式 null 清描述，省略则沿用。
    let (status, v) = call(
        &app,
        "PATCH",
        "/api/teams/growth-squad",
        TOKEN_A,
        Some(serde_json::json!({ "description": null })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["description"].is_null(), "null 必须清除描述：{v}");

    let (status, v) = call(
        &app,
        "PATCH",
        "/api/teams/nope-squad",
        TOKEN_A,
        Some(serde_json::json!({ "name": "改名" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "改不存在的团队必须 404：{v}");
}

#[tokio::test]
async fn delete_is_soft_idempotent_and_hides_the_team() {
    let (_t, app) = fixture("team-delete");
    make_experts(
        &app,
        &["cost-analyst", "growth-analyst", "risk-reviewer"],
    )
    .await;
    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body("cost-analyst", &["growth-analyst", "risk-reviewer"])),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");

    let (status, v) = call(&app, "DELETE", "/api/teams/growth-squad", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK, "删除应 200：{v}");
    assert_eq!(v["team_id"], serde_json::json!("growth-squad"));
    assert_eq!(v["deleted"], serde_json::json!(true));
    assert!(v["note"].as_str().is_some_and(|n| !n.is_empty()), "{v}");

    let (status, v) = call(&app, "GET", "/api/teams/growth-squad", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "删掉后必须 404：{v}");
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));
    assert!(v["error"]["next_step"].as_str().is_some(), "{v}");

    let (status, list) = call(&app, "GET", "/api/teams", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list["teams"].as_array().map(Vec::len),
        Some(0),
        "软删的团队不得再出现在列表里：{list}"
    );

    // 二次删除必须幂等（200 + deleted:false），不能变成 404。
    let (status, v) = call(&app, "DELETE", "/api/teams/growth-squad", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::OK, "二次删除必须幂等 200：{v}");
    assert_eq!(v["deleted"], serde_json::json!(false));
    assert!(
        v["note"].as_str().is_some_and(|n| n.contains("幂等")),
        "幂等说明要写进 note：{v}"
    );

    // 软删后同名可重建。
    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(team_body("cost-analyst", &["growth-analyst", "risk-reviewer"])),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "软删后同名应可重建：{v}");
}

#[tokio::test]
async fn deleting_a_team_that_never_existed_is_404_with_a_next_step() {
    let (_t, app) = fixture("team-delete-missing");
    let (status, v) = call(&app, "DELETE", "/api/teams/ghost-squad", TOKEN_A, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], serde_json::json!("entity_not_found"));
    let body = v.to_string();
    assert!(body.contains("下一步"), "{body}");
}

#[tokio::test]
async fn a_misspelled_or_illegal_identifier_is_400_not_silently_ignored() {
    let (_t, app) = fixture("team-bad-id");
    make_experts(&app, &["cost-analyst", "growth-analyst", "risk-reviewer"]).await;

    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(serde_json::json!({
            "team_id": "Growth Squad",
            "name": "非法标识",
            "leader_id": "cost-analyst",
            "member_ids": ["growth-analyst", "risk-reviewer"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "大写与空格必须 400：{v}");
    assert!(v.to_string().contains("team_id"), "{v}");

    // 驼峰字段名必须判红。
    let (status, v) = call(
        &app,
        "POST",
        "/api/teams",
        TOKEN_A,
        Some(serde_json::json!({
            "team_id": "growth-squad",
            "name": "驼峰字段",
            "leaderId": "cost-analyst",
            "memberIds": ["growth-analyst", "risk-reviewer"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "驼峰字段必须 400：{v}");
    let body = v.to_string();
    assert!(body.contains("leaderId"), "应点名写错的字段：{body}");
    assert!(body.contains("member_ids"), "应列出可接受字段：{body}");
}
