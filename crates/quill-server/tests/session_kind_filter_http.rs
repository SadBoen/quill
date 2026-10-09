//! `GET /api/sessions?exclude_kind=` 的契约：能不能把某些 kind 藏起来。
//!
//! 背景是 BACKLOG B2-3：建团队时 `POST /api/teams` 会在事务里先插一条
//! `kind='team_leader'` 的会话 —— `teams.leader_session_id` 是 NOT NULL 且外键
//! 指向 `sessions`，而 `POST /api/dispatch` 强制要 `leader_session_id`，
//! 那条会话是派工记账的落点，**删不得、也建不得晚**。
//!
//! 它出现在用户侧栏里却是另一回事：前端没有任何 `team_leader` 字样，
//! 不按 kind 分组，于是一律当普通对话渲染 —— 用户看到的是「建个团队
//! 凭空多出一会话」，点进去还是空的。
//!
//! 过滤做在后端而不是前端，因为那条会话跑起来之后真的有消息：前端藏起来
//! 会让侧栏条数和实际数量对不上。
//!
//! **这套机制是自创的。** goose 完全没有多智能体；octop 的 team 只是专家名册，
//! 没有「给团队开一条会话当工作区」这层。所以本文件不能拿上游当判据，
//! 钉的是我们自己的契约。

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
const TOKEN_A: &str = "tok-a";

fn user_id(uid: &str) -> quill_domain::UserId {
    quill_domain::UserId::parse(uid).expect("测试 UID 必须合法")
}

fn state(t: &TestDb) -> AppState {
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

/// 直接铺一条指定 kind 的会话。走 SQL 而不是 API，是因为 API 只建普通会话 ——
/// team_leader 那条是建团的副产品，这里要的是一个**独立于建团**的干净夹具。
///
/// `seq` 保证同一个用例里多条会话的 id 不撞：主键是 `(user_id, id)`，
/// 用固定 id 的话第二条就插不进去，而那正是「一次滤掉两种 kind」要验的场景。
fn seed_session(db: &Arc<DbBridge>, seq: u8, kind: &str, room: &str) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[0] = 0x11;
    id[1] = seq;
    let uid = user_id(UID_A).as_bytes().to_vec();
    let sid = id.to_vec();
    let kind = kind.to_string();
    let room = room.to_string();
    // sessions 的 CHECK 约束（0001_init.sql:279-281）要求：非空、
    // 不含 `..`、**且不以 `/` 开头**（`NOT LIKE '/%'` —— 是相对路径）。
    let ws = format!(".quill-test-ws/{room}");
    // 0001_init.sql 里还有一条：非 solo 的会话必须带 team_id，
    // 且 `team_member` 额外要求 parent_session_id 与 expert_id 都非空。
    // 这几条约束本身就在说明问题 —— kind 之所以有多种，是因为会话分属
    // 不同的归属，而「成员会话」必须指回它的主持人。
    let (team_id, parent_id, expert_id): (Option<Vec<u8>>, Option<Vec<u8>>, Option<&str>) =
        match kind.as_str() {
            "solo" => (None, None, None),
            "team_leader" => (Some(vec![0x7e; 16]), None, Some("cost-analyst")),
            "team_member" => (
                Some(vec![0x7e; 16]),
                Some(vec![0x11; 16]),
                Some("cost-analyst"),
            ),
            _ => (
                Some(vec![0x7e; 16]),
                Some(vec![0x11; 16]),
                Some("cost-analyst"),
            ),
        };
    let expert_id = expert_id.map(str::to_string);
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO sessions (user_id, id, kind, room_id, team_id, parent_session_id, \
                 expert_id, provider_id, model, state, workspace_path, created_at, updated_at, \
                 last_active_at) \
                 VALUES (?,?,?,?,?,?,?,'local','local','IDLE',?,0,0,0)",
            )
            .bind(uid)
            .bind(sid)
            .bind(kind)
            .bind(room)
            .bind(team_id)
            .bind(parent_id)
            .bind(expert_id)
            .bind(ws)
            .execute(&pool)
            .await
            .map_err(|e| storage_error("铺会话", e))?;
            Ok(())
        })
    })
    .expect("铺会话失败");
    id
}

async fn call(app: &AppState, path: &str) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN_A}"))
        .body(Body::empty())
        .expect("构造请求失败");
    let resp = build_router(app.clone())
        .oneshot(request)
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("读响应体")
        .to_bytes();
    let text = String::from_utf8(bytes.to_vec()).expect("响应体必须是 UTF-8");
    (
        status,
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("响应必须是 JSON（{e}）：{text}")),
    )
}

fn kinds(v: &serde_json::Value) -> Vec<String> {
    v["sessions"]
        .as_array()
        .expect("回包必须是数组")
        .iter()
        .map(|s| s["kind"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn fixture(label: &str) -> (TestDb, AppState) {
    let t = TestDb::new(label);
    seed_user(&t.bridge(), UID_A);
    let app = state(&t);
    (t, app)
}

/// 不带参数时行为不变 —— 团队页要用全量，不能顺手把别人的会话也滤掉。
#[tokio::test]
async fn no_parameter_returns_every_kind() {
    let (_t, app) = fixture("sess-kind-none");
    let db = app.db.as_ref().unwrap().clone();
    seed_session(&db, 1, "solo", "r0");
    seed_session(&db, 2, "team_leader", "r0");

    let (status, v) = call(&app, "/api/sessions").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let ks = kinds(&v);
    assert!(ks.iter().any(|k| k == "solo"), "普通会话必须在：{ks:?}");
    assert!(
        ks.iter().any(|k| k == "team_leader"),
        "不传参数就不该过滤任何 kind：{ks:?}"
    );
}

/// 主判据：排除 team_leader 之后它真的不在了，别的 kind 一个不少。
#[tokio::test]
async fn exclude_kind_removes_only_that_kind() {
    let (_t, app) = fixture("sess-kind-exclude");
    let db = app.db.as_ref().unwrap().clone();
    seed_session(&db, 1, "solo", "r0");
    seed_session(&db, 2, "team_leader", "r0");
    seed_session(&db, 3, "solo", "r1");

    let (status, v) = call(&app, "/api/sessions?exclude_kind=team_leader").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let ks = kinds(&v);
    assert_eq!(ks.len(), 2, "应只剩两条普通会话：{ks:?}");
    assert!(
        ks.iter().all(|k| k == "solo"),
        "team_leader 必须被滤掉，别的 kind 不许被连坐：{ks:?}"
    );
}

/// 一次滤掉两种。格式是逗号分隔而不是重复键 —— `serde_urlencoded` 解重复键
/// 要写成 `?exclude_kind[]=a`，方括号在 URL 里既不干净也不好手拼。
/// 这里钉的是「两个都收进来」，少收一个就意味着「一次只能滤一种」，
/// 那正是这套参数设计要避免的（下次要滤别的 kind 就得改后端签名）。
#[tokio::test]
async fn comma_separated_kinds_are_all_collected() {
    let (_t, app) = fixture("sess-kind-multi");
    let db = app.db.as_ref().unwrap().clone();
    seed_session(&db, 4, "solo", "r0");
    seed_session(&db, 5, "team_leader", "r0");
    seed_session(&db, 6, "team_member", "r0");

    let (status, v) = call(&app, "/api/sessions?exclude_kind=team_leader,team_member").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    let ks = kinds(&v);
    assert_eq!(ks, vec!["solo".to_string()], "两个 kind 都该被滤掉：{ks:?}");
}

/// 空串段被丢掉。
///
/// **但这条现在证明不了什么，别指望它会红。** 2026-10-07 实测过：把
/// `.filter(|s| !s.is_empty())` 整行删掉，这一条照样绿。原因是
/// `NOT IN ('','team_leader')` 与 `NOT IN ('team_leader')` 的结果**完全相同** ——
/// `''` 匹配不到任何行（kind 的 CHECK 约束限死在三个合法值），SQLite 视它为
/// 「这一项没约束」。**这是一个伪变异：它不改变任何可观测行为。**
///
/// 那 `filter` 还有没有用？有，但是防御性的：kind 一旦放宽到允许空串，
/// 留着空段就等于「滤掉所有 kind 为空的会话」，而那种会话将来可能是合法的。
/// 写这条不是为了让它红（它红不了），是为了把当前行为钉住 —— 将来谁放宽了
/// kind 约束，这条会立刻变红提醒他回来处理。
///
/// 顺带记一个花了三轮才查清的坑：**别拿「两种实现结果恰好相同」的输入当判据。**
/// 第一版写 `?exclude_kind=,solo`、第二版换成 `,team_leader`，两次都测不出差别，
/// 因为空串在两种实现下都不影响结果。
#[tokio::test]
async fn empty_segments_do_not_change_the_result() {
    let (_t, app) = fixture("sess-kind-empty-seg");
    let db = app.db.as_ref().unwrap().clone();
    seed_session(&db, 1, "solo", "r0");
    seed_session(&db, 2, "team_leader", "r0");

    let (status, v) = call(&app, "/api/sessions?exclude_kind=,team_leader").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        kinds(&v),
        vec!["solo".to_string()],
        "空串段被丢掉，只滤 team_leader：{v}"
    );

    // 纯空白段（%20 是空格）同理，这条还要过 trim 那一层。
    let (status, v) = call(&app, "/api/sessions?exclude_kind=%20,%20,team_leader").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(kinds(&v), vec!["solo".to_string()], "纯空白段被丢掉：{v}");
}

/// `?exclude_kind=` 整体为空时等于「不过滤」，而不是「滤掉 kind 为空的」。
#[tokio::test]
async fn wholly_empty_exclude_kind_means_no_filtering() {
    let (_t, app) = fixture("sess-kind-empty");
    let db = app.db.as_ref().unwrap().clone();
    seed_session(&db, 3, "solo", "r0");
    seed_session(&db, 4, "team_leader", "r0");

    let (status, v) = call(&app, "/api/sessions?exclude_kind=").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(kinds(&v).len(), 2, "空参数等于不过滤，不该清空列表：{v}");
}

/// 排除一个库里根本不存在的 kind：不能 500，也不能把结果清空。
/// 「滤掉的东西恰好是空的」是最容易被写成 `NOT IN` 拼串然后炸掉的分支。
#[tokio::test]
async fn excluding_an_absent_kind_is_a_no_op_not_an_error() {
    let (_t, app) = fixture("sess-kind-absent");
    let db = app.db.as_ref().unwrap().clone();
    seed_session(&db, 7, "solo", "r0");

    let (status, v) = call(&app, "/api/sessions?exclude_kind=team_leader").await;
    assert_eq!(status, StatusCode::OK, "滤掉不存在的 kind 不该报错：{v}");
    assert_eq!(kinds(&v).len(), 1, "不该因为库里没有就清空列表：{v}");
}

/// kind 来自查询参数，**不能**拼进 SQL 字符串。
/// 这里是纯文本夹具（走 STRICT 表的 TEXT 列），真被拼进去就会破坏语法。
#[tokio::test]
async fn kind_parameter_is_bound_not_interpolated() {
    let (_t, app) = fixture("sess-kind-inject");
    let db = app.db.as_ref().unwrap().clone();
    seed_session(&db, 8, "solo", "r0");

    let (status, v) = call(
        &app,
        "/api/sessions?exclude_kind=solo'%3B%20DROP%20TABLE%20sessions%3B%20--",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "注入串必须被当成普通字符串（查不到就等于没滤），不该让 SQL 炸掉：{v}"
    );

    // 表还在，且没被误伤。
    let (status, _) = call(&app, "/api/sessions").await;
    assert_eq!(status, StatusCode::OK, "表必须还在");
}

/// 会话列表的**硬上限**（queue Q055）。
///
/// 侧栏读的就是这个接口，而会话是**会自动累积**的东西 —— 每聊一次就多一条。
/// 没有上限在这里等于「聊得久了侧栏越来越慢、内存越吃越多」，而用户什么都没做错。
/// `chat_repo::LIST_SQL_TAIL` 里写死 `LIMIT 100`（`LIST_LIMIT`），这条钉住它真的生效：
/// 塞 101 条，回来必须**恰好** 100 条。
///
/// 为什么钉「恰好」而不是「≤100」：≤ 那种写法在「查询整个坏掉、只回 0 条」时也会通过。
#[tokio::test]
async fn the_session_list_is_capped_even_when_the_user_has_more() {
    let (t, app) = fixture("session-list-cap");
    let cap = quill_server::chat_repo::LIST_LIMIT;
    assert!(cap > 0, "上限必须是个正数，否则这条测试没有意义");

    // 塞 cap + 1 条。`seed_session` 的 seq 是 u8，cap = 100 时装得下。
    for i in 0..=cap {
        seed_session(&t.bridge(), i as u8, "solo", &format!("room-cap-{i}"));
    }

    let (status, v) = call(&app, "/api/sessions").await;
    assert_eq!(status, StatusCode::OK, "列表必须能读：{v}");
    let got = v["sessions"].as_array().expect("必须是数组").len();
    assert_eq!(
        got,
        cap as usize,
        "塞了 {} 条，列表必须被硬上限截到 {cap} 条（不无限返回）：拿到 {got}",
        cap + 1
    );
}
