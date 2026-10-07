//! `/api/mbti/*` 的契约。
//!
//! 三条是这个功能的命门：
//!
//! 1. **「应用」必须点名专家。** 人格会写进 `experts.instructions`，那是用户
//!    自己写的正文。没带 `expert_id` 就必须 400 —— 静默挑一个「默认专家」
//!    等于替用户决定把 INFP 的语气写进谁的嘴里。
//! 2. **重复应用不增长。** 人格段被一对标记包着，点二十次「应用」不该让
//!    instructions 变成二十段。判据直接比两次应用后的长度。
//! 3. **跨用户读不到。** `row_id` 是全局自增的，不带 owner 查就是越权。
//!    判据拿 A 的 row_id 去问 B。
//!
//! 评分算法本身在 `mbti::score` 的单测里；这里只钉 HTTP 契约。

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

/// 错误文案在 `error.detail` 里（信封形状，见 `ApiError`）。
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

/// 28 题全选 A 的作答表。
fn all_a() -> Value {
    let mut m = serde_json::Map::new();
    for id in 1..=28 {
        m.insert(id.to_string(), json!("A"));
    }
    Value::Object(m)
}

/// 建一个专家，返回它的标识。
async fn make_expert(app: &axum::Router, token: &str, id: &str) -> String {
    let (s, b) = call(
        app.clone(),
        token,
        "POST",
        "/api/experts",
        Some(json!({
            "id": id,
            "display_name": "测试专家",
            "description": "判据用",
            "instructions": "",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "建专家失败：{b}");
    id.to_string()
}

/// 直接数库里的行数。
///
/// 用它是因为接口的 `LIMIT` 会把「存储层有没有真的清理」这件事盖住 ——
/// 见 `history_is_capped_and_keeps_the_newest`。
async fn count_rows(db: &Arc<DbBridge>, uid: &str) -> i64 {
    let id = user_id(uid).as_bytes().to_vec();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mbti_results WHERE owner_user_id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .map_err(|e| storage_error("数测评记录", e))?;
            Ok::<i64, quill_agent::AgentError>(row.0)
        })
    })
    .expect("数记录失败")
}

/// 提交一次测评，返回响应体。
async fn submit(app: &axum::Router, token: &str, answers: Value) -> (StatusCode, Value) {
    call(
        app.clone(),
        token,
        "POST",
        "/api/mbti/test",
        Some(json!({ "answers": answers })),
    )
    .await
}

#[tokio::test]
async fn types_endpoint_lists_all_sixteen_with_behavior_and_color() {
    let t = TestDb::new("mbti-types");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    let (s, b) = call(app, TOKEN_A, "GET", "/api/mbti/types", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let types = b["types"].as_array().expect("types 必须是数组");
    assert_eq!(types.len(), 16, "必须正好 16 型：{b}");

    let codes: Vec<&str> = types.iter().map(|t| t["code"].as_str().unwrap()).collect();
    for want in ["INTJ", "INFJ", "ESTJ", "ESFP", "ENTP", "ISFJ"] {
        assert!(codes.contains(&want), "缺 {want}：{codes:?}");
    }
    let intj = types
        .iter()
        .find(|t| t["code"] == "INTJ")
        .expect("应含 INTJ");
    assert!(intj["name"].as_str().unwrap().contains("建筑师"), "{intj}");
    assert!(intj["summary"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(intj["color"].as_str().unwrap().starts_with('#'));
    assert_eq!(
        intj["behavior"].as_object().map(|o| o.len()),
        Some(6),
        "六项行为指引必须齐全：{intj}"
    );
    assert_eq!(intj["dimensions"]["ei"][0], "I");
    assert_eq!(intj["dimensions"]["ei"][1], json!(78));
}

#[tokio::test]
async fn questions_endpoint_returns_twenty_eight_and_ships_the_threshold() {
    let t = TestDb::new("mbti-q");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    let (s, b) = call(app, TOKEN_A, "GET", "/api/mbti/questions", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["questions"].as_array().map(|a| a.len()), Some(28));
    assert_eq!(b["min_answers"], 20, "门槛必须随题库一起发");

    // 题库里不该出现极性，只有 id/dimension/题干/两个选项。
    let q0 = &b["questions"][0];
    assert!(q0.get("question").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
    assert!(q0.get("option_a").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
    assert!(q0.get("option_b").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
    assert!(q0.get("a_pole").is_none(), "极性属于计分内部，不该外发：{q0}");
}

#[tokio::test]
async fn submitting_all_a_scores_estj_at_the_cap_and_is_recorded() {
    let t = TestDb::new("mbti-submit");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    let (s, b) = submit(&app, TOKEN_A, all_a()).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["result"]["code"], "ESTJ");
    assert_eq!(b["result"]["dimensions"]["ei"][0], "E");
    assert_eq!(b["result"]["dimensions"]["ei"][1], 85, "全选一边应顶到上限");
    assert_eq!(b["profile"]["name"], json!("总经理"), "{b}");
    let row_id = b["result"]["row_id"].as_i64().expect("要回 row_id 才能应用");

    let (s, h) = call(app.clone(), TOKEN_A, "GET", "/api/mbti/history", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["history"].as_array().map(|a| a.len()), Some(1), "{h}");
    assert_eq!(h["current"]["code"], "ESTJ");
    assert_eq!(h["current"]["row_id"].as_i64(), Some(row_id));
    assert!(
        h["current"]["applied_expert_id"].is_null(),
        "还没应用，不该有 expert：{h}"
    );
}

#[tokio::test]
async fn too_few_answers_is_rejected_naming_both_numbers() {
    let t = TestDb::new("mbti-few");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    let mut few = all_a();
    let obj = few.as_object_mut().unwrap();
    for id in 1..=10 {
        obj.remove(&id.to_string());
    }
    let (s, b) = submit(&app, TOKEN_A, few).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    let d = detail_of(&b).unwrap_or_default();
    assert!(d.contains("20"), "文案应说清门槛：{d}");
    assert!(d.contains("18"), "文案应说清答了几题（28-10=18）：{d}");

    // 被拒的提交不该留下记录。
    let (_, h) = call(app.clone(), TOKEN_A, "GET", "/api/mbti/history", None).await;
    assert_eq!(h["history"].as_array().map(|a| a.len()), Some(0), "{h}");
}

#[tokio::test]
async fn exactly_the_threshold_is_accepted() {
    // 边界：20 题是**过**的，19 题才不过。门槛写错一位就是两种错法。
    let t = TestDb::new("mbti-edge");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    let mut at20 = all_a();
    for id in 1..=8 {
        at20.as_object_mut().unwrap().remove(&id.to_string());
    }
    assert_eq!(at20.as_object().unwrap().len(), 20, "本用例依赖恰好 20 题");
    let (s, b) = submit(&app, TOKEN_A, at20).await;
    assert_eq!(s, StatusCode::CREATED, "20 题应当通过：{b}");
}

#[tokio::test]
async fn answers_must_be_an_object() {
    let t = TestDb::new("mbti-shape");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    for bad in [json!([]), json!("A"), json!(3), json!(null)] {
        let (s, b) = call(
            app.clone(),
            TOKEN_A,
            "POST",
            "/api/mbti/test",
            Some(json!({ "answers": bad })),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad} 应被拒：{b}");
    }
}

#[tokio::test]
async fn apply_without_expert_id_is_refused_rather_than_picking_one() {
    let t = TestDb::new("mbti-noexpert");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    let (_, b) = submit(&app, TOKEN_A, all_a()).await;
    let row_id = b["result"]["row_id"].as_i64().unwrap();

    let (s, e) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/mbti/apply",
        Some(json!({ "row_id": row_id })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "没点名专家就必须拒：{e}");
    assert!(
        detail_of(&e).unwrap_or_default().contains("expert_id"),
        "文案要说清缺什么：{e}"
    );

    // 空串也不算数 —— 那等于点名了空气。
    let (s2, e2) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/mbti/apply",
        Some(json!({ "row_id": row_id, "expert_id": "   " })),
    )
    .await;
    assert_eq!(s2, StatusCode::BAD_REQUEST, "空 expert_id 也必须拒：{e2}");
    // 还要说清是**哪个字段**有问题。只断言 400 是不够的：
    // 放过空串之后，后面 `ExpertId::parse("")` 同样会 400，
    // 判据照样绿 —— 分不出「字段校验」与「标识格式校验」。
    assert!(
        detail_of(&e2).unwrap_or_default().contains("expert_id"),
        "报错要点名 expert_id：{e2}"
    );
}

#[tokio::test]
async fn apply_writes_the_persona_and_repeating_it_does_not_grow_the_text() {
    let t = TestDb::new("mbti-apply");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    let slug = make_expert(&app, TOKEN_A, "persona-demo").await;
    let (_, b) = submit(&app, TOKEN_A, all_a()).await;
    let row_id = b["result"]["row_id"].as_i64().unwrap();

    let (s, first) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/mbti/apply",
        Some(json!({ "row_id": row_id, "expert_id": slug })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{first}");
    assert_eq!(first["replaced"], false, "第一次不是替换：{first}");
    let len1 = first["instructions"].as_str().unwrap().len();
    assert!(first["instructions"]
        .as_str()
        .unwrap()
        .contains("MBTI: ESTJ"), "{first}");

    let (s, second) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/mbti/apply",
        Some(json!({ "row_id": row_id, "expert_id": slug })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{second}");
    assert_eq!(
        second["replaced"], true,
        "第二次应识别为替换：{second}"
    );
    assert_eq!(second["previous_code"], "ESTJ");
    let len2 = second["instructions"].as_str().unwrap().len();
    assert_eq!(len2, len1, "同类型重复应用不该增长：{len1} → {len2}");

    // 专家那一行确实变了。
    let (s, e) = call(
        app.clone(),
        TOKEN_A,
        "GET",
        &format!("/api/experts/{slug}"),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    assert!(e["instructions"]
        .as_str()
        .unwrap()
        .contains("MBTI: ESTJ"), "{e}");

    // 历史里标出已应用。
    let (_, h) = call(app, TOKEN_A, "GET", "/api/mbti/history", None).await;
    assert_eq!(h["current"]["applied_expert_id"], json!(slug));
}

#[tokio::test]
async fn switching_type_replaces_the_segment_and_keeps_hand_written_text() {
    let t = TestDb::new("mbti-switch");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    // 先给专家写一段自己的正文。
    let (s, _) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/experts",
        Some(json!({
            "id": "keeper",
            "display_name": "守规矩的",
            "description": "先问口径",
            "instructions": "先问口径，再动手。",
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);

    let (_, b) = submit(&app, TOKEN_A, all_a()).await;
    let row1 = b["result"]["row_id"].as_i64().unwrap();
    let (s, a1) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/mbti/apply",
        Some(json!({ "row_id": row1, "expert_id": "keeper" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a1}");

    // 再测一次全选 B，得到 INFP，应用到同一个专家。
    let mut allb = all_a();
    for v in allb.as_object_mut().unwrap().values_mut() {
        *v = json!("B");
    }
    let (_, b2) = submit(&app, TOKEN_A, allb).await;
    assert_eq!(b2["result"]["code"], "INFP", "{b2}");
    let row2 = b2["result"]["row_id"].as_i64().unwrap();

    let (s, a2) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/mbti/apply",
        Some(json!({ "row_id": row2, "expert_id": "keeper" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{a2}");
    assert_eq!(a2["previous_code"], "ESTJ", "要报出被换掉的那一型");

    let text = a2["instructions"].as_str().unwrap();
    assert!(text.contains("MBTI: INFP"), "{text}");
    assert!(!text.contains("MBTI: ESTJ"), "旧人格不该留着：{text}");
    assert!(
        text.contains("先问口径，再动手。"),
        "用户自己写的正文必须保住：{text}"
    );
    assert_eq!(text.matches("<!-- mbti -->").count(), 1, "{text}");
}

#[tokio::test]
async fn another_users_record_is_not_reachable() {
    let t = TestDb::new("mbti-cross");
    seed_user(&t.bridge(), UID_A);
    seed_user(&t.bridge(), UID_B);
    let app = build_router(state(&t));

    let (_, b) = submit(&app, TOKEN_A, all_a()).await;
    let row_id = b["result"]["row_id"].as_i64().unwrap();

    // **B 必须先有一个自己真实存在的专家**，否则这条判据就是假警报：
    // 第一次写这条用例时 B 传了个不存在的 expert_id，于是即便后端真的
    // 把 A 的记录读了出来，后面查专家那一步也会报 404，断言照样通过 ——
    // 测的是「专家不存在」，不是「读不到别人的行」。
    // 变异验证（去掉 SQL 里的 owner 条件）当时就是全绿。
    make_expert(&app, TOKEN_B, "b-expert").await;

    let (s, e) = call(
        app.clone(),
        TOKEN_B,
        "POST",
        "/api/mbti/apply",
        Some(json!({ "row_id": row_id, "expert_id": "b-expert" })),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "别人的记录必须读不到：{e}");

    // 再钉一层：B 的专家的人格正文必须是空的 ——
    // 万一哪天有人「顺手」把 404 改成 500，这里仍然能看出它写过东西。
    let (s, own) = call(
        app.clone(),
        TOKEN_B,
        "GET",
        "/api/experts/b-expert",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{own}");
    assert_eq!(
        own["instructions"], json!(""),
        "没授权就不该动任何人的 instructions：{own}"
    );

    // B 的历史里是空的。
    let (_, h) = call(app, TOKEN_B, "GET", "/api/mbti/history", None).await;
    assert_eq!(h["history"].as_array().map(|a| a.len()), Some(0), "{h}");
    assert!(h["current"].is_null(), "{h}");
}

#[tokio::test]
async fn unknown_fields_are_rejected_so_a_typo_is_not_silently_dropped() {
    let t = TestDb::new("mbti-typo");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    let (s, b) = call(
        app.clone(),
        TOKEN_A,
        "POST",
        "/api/mbti/test",
        Some(json!({ "answers": all_a(), "language": "zh", "auto_apply": true })),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "多出来的字段必须报出来：{b}");
    assert!(
        detail_of(&b).unwrap_or_default().contains("auto_apply"),
        "报错要说清是哪个字段：{b}"
    );
}

#[tokio::test]
async fn history_is_capped_and_keeps_the_newest() {
    let t = TestDb::new("mbti-cap");
    seed_user(&t.bridge(), UID_A);
    let app = build_router(state(&t));

    for i in 0..22 {
        let mut a = all_a();
        // 让每轮答案不完全一样，不然全是同一条。
        if i % 2 == 1 {
            for v in a.as_object_mut().unwrap().values_mut() {
                *v = json!("B");
            }
        }
        let (s, _) = submit(&app, TOKEN_A, a).await;
        assert_eq!(s, StatusCode::CREATED);
    }

    let (s, h) = call(app, TOKEN_A, "GET", "/api/mbti/history", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["keep"], 20, "保留期是接口的一部分：{h}");
    assert_eq!(
        h["history"].as_array().map(|a| a.len()),
        Some(20),
        "超过保留期就该被砍掉：{h}"
    );
    // 最新一条必须是最后一次提交：row_id 22（第 22 次），
    // 不去猜它到底是 ESTJ 还是 INFP（取决于循环里的奇偶），那是判据在替实现想。
    assert_eq!(
        h["current"]["row_id"].as_i64(),
        Some(22),
        "最新一条必须是最后一次：{h}"
    );
    assert_eq!(h["history"][0]["code"], h["current"]["code"]);

    // **光看接口返回 20 条不算数**：接口自己就带 `LIMIT 20`，
    // 就算存储层根本没清理，这里照样是 20 条。所以要直接数库里的行 ——
    // 清理是存储行为，判据得钉在它真正发生的地方，不能钉在它对外的投影上。
    assert_eq!(
        count_rows(&t.bridge(), UID_A).await,
        20,
        "库里必须真的被清到保留条数，而不是靠接口 LIMIT 遮住"
    );
}