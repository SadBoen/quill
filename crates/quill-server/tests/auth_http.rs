//! 登录 / 登出 / 首管引导 的 HTTP 契约测试。
//!
//! 覆盖用户明确划定的范围：**只打通登录/退出这条线，注册默认关闭**。
//! 验证系统（验证码、邮箱）与注销（账号删除）不在本文件里 —— 它们的后端
//! 根本不存在，写「测试通过」是自欺。
//!
//! 口令哈希用 `Pbkdf2Params::for_tests()`：算法与标签解析路径完全一致，
//! 只是把 60 万次迭代降到 1 万次，让这组测试能在秒级跑完。生产值由
//! `server.rs::build_state` 注入，并有单独的单测钉住它是 60 万。

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

use quill_server::auth::{AuthContext, CompositeTokenResolver, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::ratelimit::RateLimiter;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::TestDb;

/// 满足 `MIN_PASSWORD_LEN`（12）的口令。
const PW: &str = "correct-horse-battery";
const WRONG_PW: &str = "definitely-not-the-one";

/// 复合解析器里那个环境变量令牌背后的账号。
const ENV_ADMIN: &str = "0192b7c8-0000-7000-8000-000000000001";
const ENV_MEMBER: &str = "0192b7c8-0000-7000-8000-000000000002";

fn env_admin_id() -> quill_domain::UserId {
    quill_domain::UserId::parse(ENV_ADMIN).expect("测试 UID 必须合法")
}

fn base_state(resolver: CompositeTokenResolver, db: &TestDb) -> AppState {
    AppState {
        config: Config::from_env(),
        tokens: Arc::new(resolver),
        db: Some(db.bridge()),
        db_problem: None,
        llm: Arc::new(RwLock::new(None)),
        llm_config: Arc::new(RwLock::new(Default::default())),
        providers: Arc::new(RwLock::new(Default::default())),
        login_limiter: Arc::new(RateLimiter::default()),
        pbkdf2: quill_control::Pbkdf2Params::for_tests(),
    }
}

fn state_with(db: &TestDb) -> AppState {
    let env = EnvTokenResolver::new(vec![(
        "tok-admin".to_string(),
        AuthContext {
            user_id: env_admin_id(),
            is_admin: true,
        },
    )]);
    // 必须是复合解析器：`POST /api/auth/login` 签发的会话令牌要能被认出来，
    // 否则后面所有「拿登录令牌访问 me/refresh/logout」的断言都会假红。
    let (resolver, sessions) = CompositeTokenResolver::new(env);
    // 同一个 Arc 交给两个解析器：会话令牌查 `sessions_auth`，环境变量令牌查 `users`。
    // 少了后者，环境变量令牌会因为「库句柄未就绪」被一律拒掉。
    resolver.attach(db.bridge());
    sessions.attach(db.bridge());

    base_state(resolver, db)
}

/// 库里**铺好了**那个环境变量令牌对应的账号行的状态。
///
/// 与 [`state_with`] 的区别就是这一行 `users` 数据。真实部署里 `bootstrap` 引导
/// 会建它；不铺的话测的是「身份不存在时会被拒」，不是「令牌能用」。
async fn state_with_provisioned_env_user(db: &TestDb) -> AppState {
    common::seed_token_user(&db.bridge(), &env_admin_id(), "tokadmin", true).await;
    state_with(db)
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

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("构造 GET 请求")
}

fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("构造 POST 请求")
}

fn with_token(method: &str, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("构造带令牌请求")
}

fn json_of(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("响应体不是 JSON（{e}）：{text}"))
}

fn err_code(text: &str) -> String {
    json_of(text)["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("响应体缺 error.code：{text}"))
        .to_string()
}

// ------------------------------------------------------------ 首管引导

#[tokio::test]
async fn a_fresh_instance_says_setup_is_required_and_registration_is_closed() {
    let db = TestDb::new("auth-setup-status");
    let resp = build_router(state_with(&db))
        .oneshot(get("/api/setup/status"))
        .await
        .expect("请求失败");

    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(&body_text(resp).await);
    assert_eq!(v["setup_required"], serde_json::json!(true));
    assert_eq!(v["user_count"], serde_json::json!(0));
    assert_eq!(
        v["registration_enabled"],
        serde_json::json!(false),
        "注册关闭必须写进响应体，客户端不能靠「路由不存在」去推断"
    );
}

#[tokio::test]
async fn there_is_no_register_route_at_all() {
    // 「注册默认关闭」在这套设计里 = 路由压根不存在，而不是返回 403。
    // 这条断言是防止有人日后「顺手加一个带开关的注册接口」。
    let db = TestDb::new("auth-no-register");
    let resp = build_router(state_with(&db))
        .oneshot(post_json(
            "/api/auth/register",
            serde_json::json!({"username":"mallory","password":PW}),
        ))
        .await
        .expect("请求失败");

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "注册路由必须不存在；一旦存在，这个端点就能被拿来无限建号"
    );
}

#[tokio::test]
async fn initial_admin_creates_the_first_owner_and_then_closes_for_good() {
    let db = TestDb::new("auth-initial-admin");
    let app = build_router(state_with(&db));

    let created = app
        .clone()
        .oneshot(post_json(
            "/api/setup/initial-admin",
            serde_json::json!({"username":"alice","password":PW,"display_name":"爱丽丝"}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(created.status(), StatusCode::CREATED);
    let v = json_of(&body_text(created).await);
    assert_eq!(v["username"], serde_json::json!("alice"));
    assert_eq!(v["display_name"], serde_json::json!("爱丽丝"));
    assert_eq!(v["role"], serde_json::json!("owner"));

    // 装完即焚：第二次必定 409。
    let again = app
        .oneshot(post_json(
            "/api/setup/initial-admin",
            serde_json::json!({"username":"mallory","password":PW}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(again.status(), StatusCode::CONFLICT);
    let text = body_text(again).await;
    assert_eq!(err_code(&text), "conflict");
    assert!(
        text.contains("注册通道已永久关闭"),
        "409 的文案要说清为什么不能再建：{text}"
    );

    // 闸门同步翻转：status 不再说需要引导，前端不会指着必然 409 的按钮。
    let status = build_router(state_with(&db))
        .oneshot(get("/api/setup/status"))
        .await
        .expect("请求失败");
    let v = json_of(&body_text(status).await);
    assert_eq!(v["setup_required"], serde_json::json!(false));
    assert_eq!(v["user_count"], serde_json::json!(1));
}

#[tokio::test]
async fn initial_admin_rejects_a_short_password_without_writing_a_row() {
    let db = TestDb::new("auth-weak-password");
    let resp = build_router(state_with(&db))
        .oneshot(post_json(
            "/api/setup/initial-admin",
            serde_json::json!({"username":"alice","password":"short"}),
        ))
        .await
        .expect("请求失败");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        common::scalar_i64(&db.bridge(), "SELECT COUNT(*) AS c FROM users"),
        0
    );
}

// ------------------------------------------------------------ 登录

async fn seeded_owner() -> TestDb {
    let db = TestDb::new("auth-login");
    let resp = build_router(state_with(&db))
        .oneshot(post_json(
            "/api/setup/initial-admin",
            serde_json::json!({"username":"alice","password":PW}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(resp.status(), StatusCode::CREATED);
    db
}

#[tokio::test]
async fn a_valid_login_returns_a_usable_bearer_token() {
    let db = seeded_owner().await;
    let app = build_router(state_with(&db));

    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":PW}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json_of(&body_text(resp).await);
    assert_eq!(v["token_type"], serde_json::json!("Bearer"));
    let token = v["access_token"]
        .as_str()
        .expect("缺 access_token")
        .to_string();
    assert!(v["expires_in"].as_i64().expect("缺 expires_in") > 0);
    assert_eq!(v["user"]["username"], serde_json::json!("alice"));
    assert_eq!(v["user"]["role"], serde_json::json!("owner"));

    // 拿这个令牌必须真的能用，且回显真实用户名而不是一串 id。
    let me = app
        .oneshot(with_token("GET", "/api/auth/me", &token))
        .await
        .expect("请求失败");
    assert_eq!(me.status(), StatusCode::OK);
    let me = json_of(&body_text(me).await);
    assert_eq!(me["username"], serde_json::json!("alice"));
    assert_eq!(me["is_admin"], serde_json::json!(true));
}

#[tokio::test]
async fn a_wrong_password_and_an_unknown_user_are_indistinguishable() {
    // 防用户名枚举：两种失败的响应必须**逐字相同**。
    // 差一个字符，攻击者就能拿这个端点列出所有用户名。
    let db = seeded_owner().await;
    let app = build_router(state_with(&db));

    let wrong = app
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":WRONG_PW}),
        ))
        .await
        .expect("请求失败");
    let wrong_status = wrong.status();
    let wrong_body = body_text(wrong).await;

    let unknown = app
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"nobody","password":WRONG_PW}),
        ))
        .await
        .expect("请求失败");
    let unknown_status = unknown.status();
    let unknown_body = body_text(unknown).await;

    assert_eq!(wrong_status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        wrong_body, unknown_body,
        "口令错与用户不存在的响应体必须逐字相同，否则这就是一个用户名枚举器"
    );
}

#[tokio::test]
async fn login_rejects_unknown_body_fields_instead_of_silently_ignoring_them() {
    // 尤其要挡 `role: "owner"` 这种注入：字段被默默忽略时，调用方会以为
    // 自己的权限设置生效了。
    let db = seeded_owner().await;
    let resp = build_router(state_with(&db))
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":PW,"role":"owner"}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ------------------------------------------------------------ 登录限流

#[tokio::test]
async fn repeated_failures_are_throttled_with_a_retry_hint() {
    let db = seeded_owner().await;
    let app = build_router(state_with(&db));
    let limit = quill_server::ratelimit::DEFAULT_MAX_FAILURES;

    // 前 limit 次应当都是真实的 401（走完了控制面）。
    for i in 0..limit {
        let resp = app
            .clone()
            .oneshot(post_json(
                "/api/auth/login",
                serde_json::json!({"username":"alice","password":WRONG_PW}),
            ))
            .await
            .expect("请求失败");
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "第 {} 次失败应当是 401，还没到阈值",
            i + 1
        );
    }

    // 再来一次必须是 429，且带 Retry-After。
    let resp = app
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":WRONG_PW}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry = resp
        .headers()
        .get("retry-after")
        .expect("429 必须带 Retry-After，否则客户端只能靠猜")
        .to_str()
        .expect("Retry-After 必须是 ASCII")
        .parse::<u64>()
        .expect("Retry-After 必须是秒数");
    assert!(retry >= 1, "Retry-After 不能是 0 秒：{retry}");

    let text = body_text(resp).await;
    assert_eq!(err_code(&text), "too_many_requests");
    assert!(text.contains("下一步"), "限流提示必须带下一步：{text}");
}

#[tokio::test]
async fn a_throttled_login_never_reaches_the_password_hasher() {
    // 这是限流的**全部意义**：被挡住的那次请求不能烧 PBKDF2。
    // 做法是记录该窗口的失败计数——若处理器真的跑到了控制面，
    // 计数会继续增长；停在阈值就说明根本没进去。
    let db = seeded_owner().await;
    let state = state_with(&db);
    let limiter = Arc::clone(&state.login_limiter);
    let app = build_router(state);
    let limit = quill_server::ratelimit::DEFAULT_MAX_FAILURES;

    for _ in 0..limit {
        app.clone()
            .oneshot(post_json(
                "/api/auth/login",
                serde_json::json!({"username":"alice","password":WRONG_PW}),
            ))
            .await
            .expect("请求失败");
    }
    let key = RateLimiter::key("alice", "unknown-peer");
    let before = limiter.failure_count(&key, quill_server::ratelimit::now_ms());
    assert_eq!(before, limit, "每次失败都应记一笔");

    let blocked = app
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":WRONG_PW}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);

    let after = limiter.failure_count(&key, quill_server::ratelimit::now_ms());
    assert_eq!(
        after, before,
        "被限流挡下的请求不能再进控制面 —— 一旦进去就意味着 PBKDF2 又烧了一遍"
    );
}

#[tokio::test]
async fn a_successful_login_clears_the_failure_budget() {
    // 注意语义：限流是**前置**检查，被挡住时连口令都不会验。
    // 所以「输对就能解锁」在已被限流时**不成立** —— 那样等于让攻击者
    // 靠猜中一次口令绕过限流。`record_success` 真正的用处是：还没到阈值时
    // 输对一次，把之前那几次误输的额度还回去，真人不会被永久锁死。
    let db = seeded_owner().await;
    let state = state_with(&db);
    let limiter = Arc::clone(&state.login_limiter);
    let app = build_router(state);
    let limit = quill_server::ratelimit::DEFAULT_MAX_FAILURES;
    let key = RateLimiter::key("alice", "unknown-peer");
    let now = quill_server::ratelimit::now_ms();

    for _ in 0..2 {
        let r = app
            .clone()
            .oneshot(post_json(
                "/api/auth/login",
                serde_json::json!({"username":"alice","password":WRONG_PW}),
            ))
            .await
            .expect("请求失败");
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(limiter.failure_count(&key, now), 2);

    let ok = app
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":PW}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(
        limiter.failure_count(&key, now),
        0,
        "输对一次必须把之前的失败额度还回去"
    );

    // 额度已恢复：还能再错满 limit 次而不会 429。
    for i in 0..limit {
        let r = app
            .clone()
            .oneshot(post_json(
                "/api/auth/login",
                serde_json::json!({"username":"alice","password":WRONG_PW}),
            ))
            .await
            .expect("请求失败");
        assert_eq!(
            r.status(),
            StatusCode::UNAUTHORIZED,
            "第 {} 次失败应当仍是 401，说明额度真的回来了",
            i + 1
        );
    }
}

#[tokio::test]
async fn a_correct_password_does_not_bypass_an_active_throttle() {
    // 这是限流的正确姿势：是否放行**不能**取决于口令是否正确，
    // 否则「猜中一次就清空额度」等于给爆破发了一张通行证。
    let db = seeded_owner().await;
    let app = build_router(state_with(&db));
    let limit = quill_server::ratelimit::DEFAULT_MAX_FAILURES;

    for _ in 0..limit {
        app.clone()
            .oneshot(post_json(
                "/api/auth/login",
                serde_json::json!({"username":"alice","password":WRONG_PW}),
            ))
            .await
            .expect("请求失败");
    }

    let blocked = app
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":PW}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(
        blocked.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "被限流时正确的口令也必须等窗口过去，不能凭它绕过"
    );
}

// ------------------------------------------------------------ 续期与登出

#[tokio::test]
async fn refresh_rotates_the_token_and_immediately_kills_the_old_one() {
    let db = seeded_owner().await;
    let app = build_router(state_with(&db));

    let login = app
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":PW}),
        ))
        .await
        .expect("请求失败");
    let old = json_of(&body_text(login).await)["access_token"]
        .as_str()
        .expect("缺 access_token")
        .to_string();

    let refreshed = app
        .clone()
        .oneshot(with_token("POST", "/api/auth/refresh", &old))
        .await
        .expect("请求失败");
    assert_eq!(refreshed.status(), StatusCode::OK);
    let new = json_of(&body_text(refreshed).await)["access_token"]
        .as_str()
        .expect("缺 access_token")
        .to_string();
    assert_ne!(
        new, old,
        "续期必须换一个新令牌，否则叫「续期」不如叫「复读」"
    );

    let stale = app
        .oneshot(with_token("GET", "/api/auth/me", &old))
        .await
        .expect("请求失败");
    assert_eq!(
        stale.status(),
        StatusCode::UNAUTHORIZED,
        "轮换后旧令牌必须立刻失效，否则轮换没有安全意义"
    );
}

#[tokio::test]
async fn refresh_requires_a_token() {
    // 曾经的漏洞面：refresh 挂在公开区 = 任何人都能续别人的会话。
    let db = seeded_owner().await;
    let resp = build_router(state_with(&db))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/refresh")
                .body(Body::empty())
                .expect("构造请求"),
        )
        .await
        .expect("请求失败");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logging_out_revokes_the_token_and_is_idempotent() {
    let db = seeded_owner().await;
    let app = build_router(state_with(&db));

    let login = app
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":PW}),
        ))
        .await
        .expect("请求失败");
    let token = json_of(&body_text(login).await)["access_token"]
        .as_str()
        .expect("缺 access_token")
        .to_string();

    let first = app
        .clone()
        .oneshot(with_token("POST", "/api/auth/logout", &token))
        .await
        .expect("请求失败");
    assert_eq!(first.status(), StatusCode::OK);
    let v = json_of(&body_text(first).await);
    assert_eq!(v["revoked"], serde_json::json!(true));

    let second = app
        .clone()
        .oneshot(with_token("POST", "/api/auth/logout", &token))
        .await
        .expect("请求失败");
    assert_eq!(
        second.status(),
        StatusCode::OK,
        "重复登出必须是 200 而不是 401：网络超时后重试登出是常态"
    );
    let v = json_of(&body_text(second).await);
    assert_eq!(v["revoked"], serde_json::json!(false));
    assert_eq!(v["revoked_count"], serde_json::json!(0));

    let after = app
        .oneshot(with_token("GET", "/api/auth/me", &token))
        .await
        .expect("请求失败");
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logging_out_does_not_revoke_an_environment_token() {
    // 环境变量令牌是配置，不是会话：改配置+重启才收得回。
    // 登出声称吊销了它会是一个危险的谎报。
    let db = TestDb::new("auth-env-token");
    let app = build_router(state_with_provisioned_env_user(&db).await);

    let resp = app
        .clone()
        .oneshot(with_token("POST", "/api/auth/logout", "tok-admin"))
        .await
        .expect("请求失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let text = body_text(resp).await;
    assert!(
        text.contains("QUILL_TOKENS"),
        "文案必须说清环境变量令牌不受登出影响：{text}"
    );

    let me = app
        .oneshot(with_token("GET", "/api/auth/me", "tok-admin"))
        .await
        .expect("请求失败");
    assert_eq!(me.status(), StatusCode::OK, "登出后环境变量令牌仍应可用");
}

#[tokio::test]
async fn a_token_only_account_gets_401_not_500_when_someone_tries_a_password() {
    // 回归测试。`QUILL_TOKENS` 引导出来的账号是 token-only：它存的是哨兵
    // 字符串 `token-only`（10 字节 salt），不是真摘要。曾经 `find_credentials`
    // 在看 algo 之前就无条件要求 16 字节 salt，于是任何口令登录都会撞
    // 不变量 → 500「内部不变量被破坏」。
    //
    // 后果有两层：正常的登录失败被报成服务端故障（误导运维），
    // 而且它还会被当成一次失败尝试计入限流额度。
    let db = TestDb::new("auth-token-only");

    // 直接插一行 token-only 账号，形状与 ensure_token_user 写入的完全一致。
    // password_hash / password_salt 是 BLOB 列，哨兵必须按字节绑，
    // 传 TEXT 会被 SQLite 拒（code 3091）。
    {
        let bridge = db.bridge();
        bridge
            .call(move |pool, _rt| {
                Box::pin(async move {
                    sqlx::query(
                        "INSERT INTO users (id, username, username_norm, display_name, \
                         password_hash, password_salt, password_algo, role, pwd_changed_at, \
                         created_at, updated_at) \
                         VALUES (x'a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0', 'tokenonly', 'tokenonly', \
                         '令牌账号', CAST('token-only-no-password' AS BLOB), \
                         CAST('token-only' AS BLOB), 'token-only', \
                         'owner', 0, 0, 0)",
                    )
                    .execute(&pool)
                    .await
                    .map_err(|e| quill_server::db::storage_error("铺 token-only 账号", e))
                })
            })
            .expect("铺 token-only 账号失败");
    }

    let resp = build_router(state_with(&db))
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"tokenonly","password":PW}),
        ))
        .await
        .expect("请求失败");

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "token-only 账号没有口令，用口令登录必须是 401；500 会把正常登录失败说成服务端故障"
    );
    let text = body_text(resp).await;
    assert_eq!(err_code(&text), "unauthorized");
    assert!(
        !text.contains("不变量"),
        "响应体不该泄漏内部不变量信息：{text}"
    );
}

// ------------------------------------------------------------ 令牌隔离

#[tokio::test]
async fn a_session_token_cannot_reach_admin_only_routes() {
    // 登录签发的是 member 还是 owner 由 users 表决定。这里造一个 member，
    // 确认它拿不到 admin 路由 —— 令牌隔离不能只靠「登录成功」来证明。
    let db = seeded_owner().await;
    let app = build_router(state_with(&db));

    // 降级 alice 为 member，直接改表。
    let bridge = db.bridge();
    bridge
        .call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query("UPDATE users SET role = 'member' WHERE username_norm = 'alice'")
                    .execute(&pool)
                    .await
                    .map_err(|e| quill_server::db::storage_error("降级测试用户", e))
            })
        })
        .expect("降级失败");

    let login = app
        .clone()
        .oneshot(post_json(
            "/api/auth/login",
            serde_json::json!({"username":"alice","password":PW}),
        ))
        .await
        .expect("请求失败");
    assert_eq!(login.status(), StatusCode::OK);
    let token = json_of(&body_text(login).await)["access_token"]
        .as_str()
        .expect("缺 access_token")
        .to_string();

    let forbidden = app
        .clone()
        .oneshot(with_token("GET", "/api/admin/config", &token))
        .await
        .expect("请求失败");
    assert_eq!(
        forbidden.status(),
        StatusCode::FORBIDDEN,
        "member 令牌不得访问 admin 路由"
    );

    let allowed = app
        .oneshot(with_token("GET", "/api/experts", &token))
        .await
        .expect("请求失败");
    assert_eq!(
        allowed.status(),
        StatusCode::OK,
        "member 仍应能读自己的数据"
    );
}

// ------------------------------------------- 环境变量令牌也要过 users 行

#[tokio::test]
async fn disabling_an_account_blocks_its_environment_token_on_the_next_request() {
    // 这是本文件最重要的一条。早先环境变量令牌命中静态表就直接放行、**不查库**，
    // 于是把状态改成 disabled 之后那个人照样进得来 —— 「停用」是假的，
    // 而管理界面上的「已停用」等于在骗运维。
    let db = TestDb::new("auth-env-disabled");
    let app = build_router(state_with_provisioned_env_user(&db).await);

    let before = app
        .clone()
        .oneshot(with_token("GET", "/api/auth/me", "tok-admin"))
        .await
        .expect("请求失败");
    assert_eq!(before.status(), StatusCode::OK, "停用前令牌当然能用");

    common::disable_user(&db.bridge(), &env_admin_id()).await;

    let after = app
        .oneshot(with_token("GET", "/api/auth/me", "tok-admin"))
        .await
        .expect("请求失败");
    assert_eq!(
        after.status(),
        StatusCode::UNAUTHORIZED,
        "账号已停用，环境变量令牌必须在下一个请求就被拒"
    );
}

#[tokio::test]
async fn an_environment_token_whose_account_is_missing_from_the_db_is_refused() {
    // 静态表里有这个人、库里却没有 → 不能放行。放行就等于「配了就能进」，
    // 与「服务端按账号状态裁决」是两回事。库坏了也该如此：宁可进不来，
    // 不要拿一个查不了的库当通行证。
    let db = TestDb::new("auth-env-no-row");
    let app = build_router(state_with(&db));

    let resp = app
        .oneshot(with_token("GET", "/api/auth/me", "tok-admin"))
        .await
        .expect("请求失败");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "库里没有对应账号行时不得放行环境变量令牌"
    );
}

#[tokio::test]
async fn an_environment_token_takes_its_role_from_the_user_row_not_from_the_config() {
    // `QUILL_TOKENS` 里写了 `:admin` 只在**启动引导**时决定初始角色。之后能改角色
    // 的是管理界面；鉴权必须跟管理界面一致，否则会出现「界面已把 owner 降级、
    // 那枚令牌还能当 admin 用」。这里行是 member、配置声称 admin，以行为准。
    let db = TestDb::new("auth-env-role");
    let id = quill_domain::UserId::parse(ENV_MEMBER).expect("测试 UID 必须合法");
    common::seed_token_user(&db.bridge(), &id, "tokmember", false).await;

    let env = EnvTokenResolver::new(vec![(
        "tok-lying-admin".to_string(),
        AuthContext {
            user_id: id,
            // 配置里明明写着 admin
            is_admin: true,
        },
    )]);
    let (resolver, sessions) = CompositeTokenResolver::new(env);
    resolver.attach(db.bridge());
    sessions.attach(db.bridge());
    let app = build_router(base_state(resolver, &db));

    let forbidden = app
        .clone()
        .oneshot(with_token("GET", "/api/admin/config", "tok-lying-admin"))
        .await
        .expect("请求失败");
    assert_eq!(
        forbidden.status(),
        StatusCode::FORBIDDEN,
        "users 行说他是 member，配置说 admin —— 必须以行为准，否则降级是假的"
    );

    let allowed = app
        .oneshot(with_token("GET", "/api/experts", "tok-lying-admin"))
        .await
        .expect("请求失败");
    assert_eq!(
        allowed.status(),
        StatusCode::OK,
        "member 仍应能读自己的数据"
    );
}
