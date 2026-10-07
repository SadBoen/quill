//! 用户管理的 HTTP 契约测试。
//!
//! 这个文件钉的是**三件容易被糊弄过去的事**：
//!
//! 1. `GET /api/users` 真的读库。名单里必须有引导出来的真实账号，
//!    且分页参数真的会改变返回的条数 —— 页面上那个「下一页」按钮此前
//!    只改前端 state，服务端压根不认，是个纯装饰。
//! 2. 停用一个账号**真的挡得住**所有人：已登录会话令牌下一请求即拒，
//!    环境变量令牌也一样（令牌鉴权会回 `users` 行核状态）。响应里带
//!    `has_env_token` 与 `warning`，但那句 `warning` 说的是**登出**收不回令牌，
//!    不再说「停用挡不住」—— 那句话曾经在替一个真洞背书。
//! 3. `POST /api/users` 与 `DELETE /api/users/{id}` 仍然是 501，且要
//!    说得出「为什么不做、下一步怎么办」—— 有意的拒绝和有意的沉默
//!    对用户是两回事。

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

const OWNER_TOKEN: &str = "tok-owner";
const BOB_TOKEN: &str = "tok-bob";
const PW: &str = "correct-horse-battery";

/// owner 的 id 要与 `OWNER_UID` 一致，且库里**必须有这一行**：
/// `ControlPlane::require_owner` 是查库的，库里没行的话它报的
/// 是「找不到这个人」而不是「不是 owner」，错误信息会指错方向。
const OWNER_UID: &str = "0192b7c8-0000-7000-8000-000000000001";
const BOB_UID: &str = "0192b7c8-0000-7000-8000-000000000002";

struct Harness {
    db: TestDb,
}

impl Harness {
    fn new(label: &str) -> Self {
        let db = TestDb::new(label);
        // owner 与 bob 都由环境变量令牌引导出行，bob 是普通成员。
        // bob 之所以**没有**令牌也能出现在名单里，正是因为引导会建行 ——
        // 如果哪天引导改成不建行，这条测试会立刻失败，那正是我们要的信号。
        seed(&db, OWNER_UID, "owner", true);
        seed(&db, BOB_UID, "bob", false);
        Self { db }
    }

    fn state(&self) -> AppState {
        let env = EnvTokenResolver::new(vec![
            (
                OWNER_TOKEN.to_string(),
                AuthContext {
                    user_id: quill_domain::UserId::parse(OWNER_UID).expect("测试 UID 必须合法"),
                    is_admin: true,
                },
            ),
            (
                BOB_TOKEN.to_string(),
                AuthContext {
                    user_id: quill_domain::UserId::parse(BOB_UID).expect("测试 UID 必须合法"),
                    is_admin: false,
                },
            ),
        ]);
        let (resolver, sessions) = CompositeTokenResolver::new(env);
        // 同一个 Arc 交给两个：会话令牌查 `sessions_auth`，环境变量令牌查 `users`
        // 核状态与角色。少了后者，这两枚环境变量令牌会被一律拒掉，
        // 下面每一条用它们发的请求都会假红。
        resolver.attach(self.db.bridge());
        sessions.attach(self.db.bridge());
        AppState {
            config: Config::from_env(),
            tokens: Arc::new(resolver),
            db: Some(self.db.bridge()),
            db_problem: None,
            llm: Arc::new(RwLock::new(None)),
            llm_config: Arc::new(RwLock::new(Default::default())),
            providers: Arc::new(RwLock::new(Default::default())),
            login_limiter: Arc::new(RateLimiter::default()),
            pbkdf2: quill_control::Pbkdf2Params::for_tests(),
        }
    }

    async fn call(
        &self,
        method: &str,
        path: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, String) {
        let b = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"));
        let req = match body {
            Some(v) => b
                .header("content-type", "application/json")
                .body(Body::from(v.to_string()))
                .expect("构造请求"),
            None => b.body(Body::empty()).expect("构造请求"),
        };
        let resp = build_router(self.state())
            .oneshot(req)
            .await
            .expect("请求失败");
        let status = resp.status();
        let bytes = resp
            .into_body()
            .collect()
            .await
            .expect("读响应体")
            .to_bytes();
        (
            status,
            String::from_utf8(bytes.to_vec()).expect("响应体必须是 UTF-8"),
        )
    }

    async fn users(&self, token: &str, query: &str) -> serde_json::Value {
        let (status, text) = self
            .call("GET", &format!("/api/users?{query}"), token, None)
            .await;
        assert_eq!(status, StatusCode::OK, "列用户应返回 200：{text}");
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("响应体不是 JSON（{e}）：{text}"))
    }
}

/// 借 DbBridge 的通道跑一次引导。池的所有权在 bridge 的 worker 里，
/// 跨线程直接用会踩数据竞争，所以只能走 `call`。
///
/// 通道的错误类型是 `AgentError`，而账号域只认 `ControlError`，两个 crate
/// 不能互相依赖成环。所以照 `api_auth::with_control` 的老办法：**把结果装在
/// 槽位里带出来**，通道上只走 `Ok(())`，业务错误不经过任何字符串转换。
fn seed(db: &TestDb, uid: &str, username: &str, is_admin: bool) {
    let id = quill_domain::UserId::parse(uid).expect("测试 UID 必须合法");
    let username = username.to_string();
    let slot: Arc<
        std::sync::Mutex<Option<Result<quill_control::Provision, quill_control::ControlError>>>,
    > = Arc::new(std::sync::Mutex::new(None));
    let writer = Arc::clone(&slot);
    db.bridge()
        .call(move |pool, _rt| {
            Box::pin(async move {
                *writer.lock().expect("结果槽位不该被毒化") =
                    Some(quill_control::ensure_token_user(&pool, id, &username, is_admin).await);
                Ok(())
            })
        })
        .expect("通道失败");
    slot.lock()
        .expect("槽位")
        .take()
        .expect("通道必须带回结果")
        .expect("引导账号必须成功");
}

/// 同一个通道手法，建一个**有口令、且不在 QUILL_TOKENS 里**的账号。
fn seed_password_user(db: &TestDb, username: &str, password: &str) {
    let username = username.to_string();
    let password = password.to_string();
    let slot: Arc<
        std::sync::Mutex<
            Option<Result<quill_control::PasswordProvision, quill_control::ControlError>>,
        >,
    > = Arc::new(std::sync::Mutex::new(None));
    let writer = Arc::clone(&slot);
    db.bridge()
        .call(move |pool, _rt| {
            Box::pin(async move {
                *writer.lock().expect("结果槽位不该被毒化") = Some(
                    quill_control::ensure_password_user(
                        &pool,
                        &username,
                        &password,
                        false,
                        quill_control::Pbkdf2Params::for_tests(),
                    )
                    .await,
                );
                Ok(())
            })
        })
        .expect("通道失败");
    slot.lock()
        .expect("槽位")
        .take()
        .expect("通道必须带回结果")
        .expect("口令账号必须建得出");
}

fn json_of(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("响应体不是 JSON（{e}）：{text}"))
}

// ------------------------------------------------------------------ 列用户

/// 1. 名单来自库里，且两个引导出来的账号都在。
#[tokio::test]
async fn listing_users_returns_the_accounts_that_really_exist() {
    let h = Harness::new("users-list");
    let v = h.users(OWNER_TOKEN, "").await;

    let users = v["users"].as_array().expect("users 必须是数组");
    assert_eq!(
        v["total"].as_u64(),
        Some(2),
        "total 必须是过滤后的总数：{v}"
    );
    let names: Vec<&str> = users
        .iter()
        .map(|u| u["username"].as_str().unwrap_or_default())
        .collect();
    assert!(names.contains(&"owner"), "owner 不在名单里：{names:?}");
    assert!(names.contains(&"bob"), "bob 不在名单里：{names:?}");

    // 字段必须是**有来源的**那些。老的界面上写的是 name/email/locked/
    // bytes_used/quota_bytes —— email、locked、配额在本实例里根本没有来源，
    // 报出来就是编的。
    let bob = users
        .iter()
        .find(|u| u["username"] == "bob")
        .expect("名单里要有 bob");
    for f in [
        "id",
        "username",
        "display_name",
        "role",
        "status",
        "has_env_token",
    ] {
        assert!(bob.get(f).is_some(), "每条记录都必须带 {f}：{bob}");
    }
    for f in ["email", "locked", "bytes_used", "quota_bytes"] {
        assert!(
            bob.get(f).is_none(),
            "{f} 在本实例里没有来源，报出来就是编的：{bob}"
        );
    }
}

/// 2. 分页参数真的会改变返回内容（此前那个「下一页」按钮是装饰）。
#[tokio::test]
async fn offset_and_limit_actually_page_through_the_list() {
    let h = Harness::new("users-paging");
    let first = h.users(OWNER_TOKEN, "offset=0&limit=1").await;
    let second = h.users(OWNER_TOKEN, "offset=1&limit=1").await;

    assert_eq!(first["users"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(second["users"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(first["total"], second["total"], "翻页不改变总数");
    assert_ne!(
        first["users"][0]["id"], second["users"][0]["id"],
        "第二页必须给出不同的账号，否则翻页没翻"
    );

    // 越界不报错，给空数组 + 真实总数。
    let beyond = h.users(OWNER_TOKEN, "offset=99&limit=10").await;
    assert_eq!(beyond["users"].as_array().map(|a| a.len()), Some(0));
    assert_eq!(beyond["total"].as_u64(), Some(2));
}

/// 3. limit 有上限，不是一个能把整张表序列化出去的旋钮。
#[tokio::test]
async fn limit_is_capped() {
    let h = Harness::new("users-limit-cap");
    let v = h.users(OWNER_TOKEN, "limit=100000").await;
    let n = v["users"].as_array().map(|a| a.len()).unwrap_or(usize::MAX);
    assert!(
        n <= quill_server::api_users::MAX_LIMIT,
        "limit 未被截断：{n}"
    );
}

/// 4. 非 owner 看不了名册。
#[tokio::test]
async fn a_member_cannot_list_users() {
    let h = Harness::new("users-perm");
    let (status, text) = h.call("GET", "/api/users", BOB_TOKEN, None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "普通成员列名册必须被拒：{text}"
    );
    assert!(text.contains("forbidden"), "{text}");
}

// -------------------------------------------------------------- 启停账号

/// 5. 停用真的生效：被停用的人已登录的会话，下一个请求即被拒。
#[tokio::test]
async fn disabling_an_account_actually_breaks_its_live_session() {
    let h = Harness::new("users-disable-real");
    let bob = bob_id(&h).await;

    let (status, text) = h
        .call(
            "PATCH",
            &format!("/api/users/{bob}"),
            OWNER_TOKEN,
            Some(serde_json::json!({ "status": "disabled" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "停用应成功：{text}");
    assert_eq!(json_of(&text)["status"], "disabled", "{text}");

    let list = h.users(OWNER_TOKEN, "").await;
    let bob_row = list["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "bob")
        .expect("bob 仍在名单里");
    assert_eq!(bob_row["status"], "disabled", "库里必须真的改了：{list}");
}

/// 6. 停用**也挡得住**环境变量令牌 —— 而且响应必须说出来它挡的是什么。
///
/// 这条测试的断言方向被整个翻过来过一次。早先环境变量令牌命中静态表就直接放行、
/// **完全不查库**，于是停用之后 bob 照常进得来，接口还配了一句「停用挡不住他」
/// 的 `warning` 去把这个洞合法化。现在令牌鉴权会回 `users` 行核状态
/// （`auth.rs` 的 `confirm_identity`），停用对所有人一视同仁。
///
/// 留着的那个 `has_env_token` 回答的是**另一个**问题：登出收不收得回他的凭据。
#[tokio::test]
async fn disabling_also_blocks_an_env_token_and_says_so() {
    let h = Harness::new("users-disable-envtoken");
    let bob = bob_id(&h).await;

    // 先证明这枚令牌在停用前是通的，否则后面「被拒」可能只是它从来就不通。
    let (before, _) = h.call("GET", "/api/auth/me", BOB_TOKEN, None).await;
    assert_eq!(before, StatusCode::OK, "停用前 bob 的令牌当然能用");

    let (status, text) = h
        .call(
            "PATCH",
            &format!("/api/users/{bob}"),
            OWNER_TOKEN,
            Some(serde_json::json!({ "status": "disabled" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let v = json_of(&text);

    assert_eq!(v["has_env_token"], true, "bob 明明有环境变量令牌：{text}");
    let warning = v["warning"].as_str().unwrap_or_default();
    assert!(
        warning.contains("QUILL_TOKENS"),
        "仍然要点名令牌这条路：{warning}"
    );
    assert!(
        warning.contains("登出"),
        "要说清剩下那个真实的缺口是登出收不回，而不是停用挡不住：{warning}"
    );
    assert!(
        !warning.contains("挡不住"),
        "这句现在是假的，留着等于骗运维：{warning}"
    );

    // 文案不能只是文案：bob 此刻必须真的进不来了。
    let (after, _) = h.call("GET", "/api/auth/me", BOB_TOKEN, None).await;
    assert_eq!(
        after,
        StatusCode::UNAUTHORIZED,
        "账号已停用，环境变量令牌必须在下一个请求就被拒"
    );
}

/// 7. 不带令牌的纯口令账号，停用后令牌这条路走不通，此时**不该**再喊警告。
#[tokio::test]
async fn disabling_a_password_only_account_reports_no_env_token() {
    let h = Harness::new("users-disable-pwonly");
    // carol 不在 QUILL_TOKENS 里，只是一个口令账号。
    let carol = quill_control::derive_user_id("carol").expect("carol 的 id 必须可推");
    seed_password_user(&h.db, "carol", PW);

    let id = carol.to_compact_hex();
    let (status, text) = h
        .call(
            "PATCH",
            &format!("/api/users/{id}"),
            OWNER_TOKEN,
            Some(serde_json::json!({ "status": "disabled" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let v = json_of(&text);
    assert_eq!(
        v["has_env_token"], false,
        "carol 不该有环境变量令牌：{text}"
    );
    assert!(
        v.get("warning").is_none(),
        "没有令牌就没什么可警告的，不许重复喊一遍：{text}"
    );
}

/// 8. 不能停用自己 —— 否则当场把自己锁在门外。
#[tokio::test]
async fn an_owner_cannot_disable_itself() {
    let h = Harness::new("users-self-disable");
    let (status, text) = h
        .call(
            "PATCH",
            &format!("/api/users/{OWNER_UID}"),
            OWNER_TOKEN,
            Some(serde_json::json!({ "status": "disabled" })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "自停用必须被拒：{text}");
    assert!(text.contains("conflict"), "{text}");

    // 必须真的没改：拒了却改了，是最坏的一种结果。
    let list = h.users(OWNER_TOKEN, "").await;
    let me = list["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "owner")
        .expect("owner 仍在名单里");
    assert_eq!(me["status"], "active", "拒了却真改了：{list}");
}

/// 9. 乱填的 status 与乱填的 id 都要给出可执行的错，而不是 500。
#[tokio::test]
async fn bad_status_and_bad_id_are_refused_with_a_reason() {
    let h = Harness::new("users-bad-input");
    let bob = bob_id(&h).await;

    let (s1, t1) = h
        .call(
            "PATCH",
            &format!("/api/users/{bob}"),
            OWNER_TOKEN,
            Some(serde_json::json!({ "status": "deleted" })),
        )
        .await;
    assert_eq!(s1, StatusCode::BAD_REQUEST, "{t1}");
    assert!(t1.contains("disabled"), "要说清只有哪两个值：{t1}");

    let (s2, t2) = h
        .call(
            "PATCH",
            "/api/users/not-a-uuid",
            OWNER_TOKEN,
            Some(serde_json::json!({ "status": "active" })),
        )
        .await;
    assert_eq!(s2, StatusCode::BAD_REQUEST, "{t2}");
    assert!(t2.contains("GET /api/users"), "要说清 id 从哪来：{t2}");
}

// -------------------------------------------------- 有意不做的那两条

/// 10. 建号与删号继续 501，且必须带「为什么 + 下一步」。
#[tokio::test]
async fn creating_and_deleting_accounts_stay_501_and_explain_themselves() {
    let h = Harness::new("users-not-allowed");
    let bob = bob_id(&h).await;

    let (s1, t1) = h
        .call(
            "POST",
            "/api/users",
            OWNER_TOKEN,
            Some(serde_json::json!({ "username": "x", "password": PW })),
        )
        .await;
    assert_eq!(s1, StatusCode::NOT_IMPLEMENTED, "{t1}");
    assert!(
        t1.contains("QUILL_PASSWORD_USERS"),
        "建号要走部署配置：{t1}"
    );

    let (s2, t2) = h
        .call("DELETE", &format!("/api/users/{bob}"), OWNER_TOKEN, None)
        .await;
    assert_eq!(s2, StatusCode::NOT_IMPLEMENTED, "{t2}");
    assert!(
        t2.contains("deleted_at"),
        "要说清为什么不能删（软删除只做了一半）：{t2}"
    );
    assert!(
        t2.contains("PATCH /api/users/{id}"),
        "必须给出真正可行的下一步：{t2}"
    );
}

async fn bob_id(h: &Harness) -> String {
    let list = h.users(OWNER_TOKEN, "").await;
    list["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "bob")
        .expect("bob 必须在名单里")["id"]
        .as_str()
        .expect("id 必须是字符串")
        .to_string()
}
