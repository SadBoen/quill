//! MCP 配置与 SKILL 的 HTTP 契约。
//!
//! 重点钉三件事：
//!
//! 1. **不伪造连通性。** 协议层（`rmcp`）还没接，所以响应里 `connected` 必须
//!    恒为 `false`，而且 `note` 要说清下一步接什么。哪天有人为了让界面显示
//!    「已连接」而把它写死成 `true`，这条测试会红。
//! 2. **存得下也读得回。** 前端是全量提交，存进去的每个字段都得原样回来；
//!    `enabled_capabilities` 的三态（`null` / `[]` / 数组）尤其不能被抹平。
//! 3. **报错要说清下一步**，并且按用户隔离。

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

/// `Config` 的 `db_path` 决定 SKILL 正文目录（`<db 同级>/skills`）。
/// 每个用例一份自己的目录，用例之间不串；Drop 时连目录一起清掉。
struct Harness {
    db: TestDb,
    _dir: std::path::PathBuf,
}

impl Harness {
    fn new(label: &str) -> Self {
        let db = TestDb::new(label);
        let dir = std::path::PathBuf::from(db.path())
            .parent()
            .expect("临时库必有父目录")
            .to_path_buf();
        let mut cfg = Config::from_env();
        cfg.db_path = dir.join("quill.db");
        Self { db, _dir: dir }
    }

    fn state(&self) -> AppState {
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
            config: self.config(),
            tokens: Arc::new(resolver),
            db: Some(self.db.bridge()),
            db_problem: None,
            llm: Arc::new(RwLock::new(None)),
            llm_config: Arc::new(RwLock::new(Default::default())),
            providers: Arc::new(RwLock::new(Default::default())),
            login_limiter: Arc::new(Default::default()),
            pbkdf2: quill_control::Pbkdf2Params::for_tests(),
        }
    }

    fn config(&self) -> Config {
        let mut cfg = Config::from_env();
        cfg.db_path = std::path::PathBuf::from(self.db.path())
            .parent()
            .expect("临时库必有父目录")
            .join("quill.db");
        cfg
    }

    fn skill_dir(&self) -> std::path::PathBuf {
        self.config()
            .db_path
            .parent()
            .expect("临时库必有父目录")
            .join("skills")
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
            .map_err(|e| storage_error("铺用户", e))?;
            Ok(())
        })
    })
    .expect("铺用户失败");
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

fn json_of(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("响应体不是 JSON（{e}）：{text}"))
}

fn req(method: &str, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("构造请求")
}

fn post_json(path: &str, token: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("构造请求")
}

fn stdio(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "transport": "stdio",
        "command": "npx",
        "args": ["-y", "some-server"],
        "env": {"TOKEN": "secret"},
        "enabled_capabilities": null,
        "timeout_ms": 30000
    })
}

/// 存一份配置并回读，断言状态码是 200。
async fn save(h: &Harness, token: &str, body: serde_json::Value) -> serde_json::Value {
    let resp = build_router(h.state())
        .oneshot(post_json("/api/extensions/mcp", token, body))
        .await
        .expect("请求失败");
    let st = resp.status();
    let text = body_text(resp).await;
    assert_eq!(st, StatusCode::OK, "保存应当成功：{text}");
    json_of(&text)
}

async fn list(h: &Harness, token: &str) -> serde_json::Value {
    let resp = build_router(h.state())
        .oneshot(req("GET", "/api/extensions/mcp", token))
        .await
        .expect("请求失败");
    let st = resp.status();
    let text = body_text(resp).await;
    assert_eq!(st, StatusCode::OK, "读取应当成功：{text}");
    json_of(&text)
}

// ------------------------------------------------------------------ 不伪造

#[tokio::test]
async fn saving_mcp_never_claims_to_be_connected() {
    let h = Harness::new("ext-mcp-not-connected");
    seed_user(&h.db.bridge(), UID_A);

    let v = save(
        &h,
        TOKEN_A,
        serde_json::json!({"servers": [stdio("filesystem")]}),
    )
    .await;
    assert_eq!(
        v["connected"],
        serde_json::json!(false),
        "协议层还没接，必须说未连接。写 true 等于骗界面"
    );
    let note = v["note"].as_str().expect("必须有说明");
    assert!(note.contains("rmcp"), "要说清下一步接什么：{note}");

    // 读回来也一样 —— 不能只在 POST 的响应里诚实。
    assert_eq!(list(&h, TOKEN_A).await["connected"], serde_json::json!(false));
}

#[tokio::test]
async fn a_server_that_is_not_running_still_saves_without_error() {
    // 这是这一层存在的理由：服务器没起 / 地址写错，不该让「保存」失败。
    // 用户恰恰需要先把配置登记录进去，才能去排查。
    let h = Harness::new("ext-mcp-unreachable");
    seed_user(&h.db.bridge(), UID_A);

    let v = save(
        &h,
        TOKEN_A,
        serde_json::json!({"servers": [{
            "name": "ghost",
            "transport": "streamable_http",
            // 127.0.0.1:1 上不会有东西监听
            "url": "http://127.0.0.1:1/mcp",
            "enabled_capabilities": []
        }]}),
    )
    .await;
    assert_eq!(v["servers"][0]["name"], serde_json::json!("ghost"));
}

// ------------------------------------------------------------------ 存取往返

#[tokio::test]
async fn a_stdio_server_survives_the_round_trip_field_by_field() {
    let h = Harness::new("ext-mcp-stdio-roundtrip");
    seed_user(&h.db.bridge(), UID_A);

    save(
        &h,
        TOKEN_A,
        serde_json::json!({"servers": [{
            "name": "Filesystem",
            "transport": "stdio",
            "command": "uvx",
            "args": ["mcp-server-git", "--repository", "/tmp"],
            "cwd": "/tmp/work",
            "env": {"B": "2", "A": "1"},
            "description": "本地文件系统",
            "max_concurrent_calls": 4,
            "enabled_capabilities": ["read", "write"],
            "timeout_ms": 45000
        }]}),
    )
    .await;

    let s = &list(&h, TOKEN_A).await["servers"][0];
    assert_eq!(s["name"], serde_json::json!("filesystem"), "名字要归一成小写");
    assert_eq!(s["command"], serde_json::json!("uvx"));
    assert_eq!(
        s["args"],
        serde_json::json!(["mcp-server-git", "--repository", "/tmp"])
    );
    assert_eq!(s["cwd"], serde_json::json!("/tmp/work"));
    assert_eq!(s["env"], serde_json::json!({"A": "1", "B": "2"}));
    assert_eq!(s["description"], serde_json::json!("本地文件系统"));
    assert_eq!(s["max_concurrent_calls"], serde_json::json!(4));
    assert_eq!(s["enabled_capabilities"], serde_json::json!(["read", "write"]));
    assert_eq!(s["timeout_ms"], serde_json::json!(45000));
    assert!(
        s.get("url").is_none(),
        "stdio 不该带 url：前端按 transport 决定读哪个字段，多余字段会让 \
         切换传输方式后残留上一份的值"
    );
}

#[tokio::test]
async fn the_capability_three_states_stay_distinguishable_over_http() {
    // null = 全禁，[] = 全开，数组 = 精确列举。前端靠这个区别渲染选择状态，
    // 存进去变成同一个东西就等于把「全开」静默降级成「全禁」。
    let h = Harness::new("ext-mcp-caps");
    seed_user(&h.db.bridge(), UID_A);

    for (i, caps) in [serde_json::Value::Null, serde_json::json!([]), serde_json::json!(["read"])]
        .iter()
        .enumerate()
    {
        let mut s = stdio(&format!("s{i}"));
        s["enabled_capabilities"] = caps.clone();
        save(&h, TOKEN_A, serde_json::json!({"servers": [s]})).await;
        let back = list(&h, TOKEN_A).await["servers"][0]["enabled_capabilities"].clone();
        assert_eq!(&back, caps, "第 {i} 项的能力三态被抹平了：{caps} → {back}");
    }
}

// ------------------------------------------------------------------ 全量覆盖

#[tokio::test]
async fn servers_missing_from_the_next_post_are_soft_deleted() {
    let h = Harness::new("ext-mcp-replace");
    seed_user(&h.db.bridge(), UID_A);
    save(
        &h,
        TOKEN_A,
        serde_json::json!({"servers": [stdio("a"), stdio("b")]}),
    )
    .await;
    assert_eq!(list(&h, TOKEN_A).await["servers"].as_array().unwrap().len(), 2);

    let v = save(&h, TOKEN_A, serde_json::json!({"servers": [stdio("a")]})).await;
    let names: Vec<String> = v["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["a".to_string()], "b 应当被软删：{names:?}");

    // 软删而不是物理删：行还在，只是对用户不可见。
    let left = common::scalar_i64(
        &h.db.bridge(),
        "SELECT COUNT(*) AS c FROM mcp_servers WHERE name='b' AND deleted_at IS NULL",
    );
    assert_eq!(left, 0, "b 不得再出现在可见行里");
}

#[tokio::test]
async fn re_posting_identical_content_does_not_pretend_something_changed() {
    // 指纹相同就整行跳过，updated_at 不动。否则「更新时间」列会每次点保存
    // 都跳一下，用户没法用它判断自己到底改了什么。
    let h = Harness::new("ext-mcp-fingerprint");
    seed_user(&h.db.bridge(), UID_A);
    let body = serde_json::json!({"servers": [stdio("a")]});
    save(&h, TOKEN_A, body.clone()).await;

    let before = common::scalar_i64(
        &h.db.bridge(),
        "SELECT updated_at AS c FROM mcp_servers WHERE name='a'",
    );
    save(&h, TOKEN_A, body).await;
    let after = common::scalar_i64(
        &h.db.bridge(),
        "SELECT updated_at AS c FROM mcp_servers WHERE name='a'",
    );
    assert_eq!(before, after, "一字未改却动了 updated_at");

    // 真的改了内容就必须更新。
    let mut changed = stdio("a");
    changed["command"] = serde_json::json!("uvx");
    save(&h, TOKEN_A, serde_json::json!({"servers": [changed]})).await;
    let after2 = common::scalar_i64(
        &h.db.bridge(),
        "SELECT updated_at AS c FROM mcp_servers WHERE name='a'",
    );
    assert!(after2 > after, "内容真变了却没更新时间戳：{after} → {after2}");
}

#[tokio::test]
async fn delete_reports_honestly_when_there_was_nothing_to_delete() {
    let h = Harness::new("ext-mcp-delete");
    seed_user(&h.db.bridge(), UID_A);
    save(&h, TOKEN_A, serde_json::json!({"servers": [stdio("a")]})).await;

    let resp = build_router(h.state())
        .oneshot(req("DELETE", "/api/extensions/mcp/a", TOKEN_A))
        .await
        .expect("请求失败");
    let v = json_of(&body_text(resp).await);
    assert_eq!(v["deleted"], serde_json::json!(true));

    let resp = build_router(h.state())
        .oneshot(req("DELETE", "/api/extensions/mcp/a", TOKEN_A))
        .await
        .expect("请求失败");
    let v = json_of(&body_text(resp).await);
    assert_eq!(
        v["deleted"],
        serde_json::json!(false),
        "第二次删什么都没发生，报 true 等于凭空告诉用户「刚删掉了一台服务器」"
    );
}

// ------------------------------------------------------------------ 拒绝与隔离

#[tokio::test]
async fn one_user_never_sees_another_users_mcp_servers() {
    let h = Harness::new("ext-mcp-isolation");
    seed_user(&h.db.bridge(), UID_A);
    seed_user(&h.db.bridge(), UID_B);
    save(&h, TOKEN_A, serde_json::json!({"servers": [stdio("a-secret")]})).await;

    let b = list(&h, TOKEN_B).await;
    assert_eq!(
        b["servers"].as_array().unwrap().len(),
        0,
        "B 绝不能看到 A 的 MCP 配置：这里存着 command 与 token 环境变量"
    );
}

#[tokio::test]
async fn the_extension_endpoints_refuse_anonymous_callers() {
    let h = Harness::new("ext-mcp-anon");
    for (method, path) in [
        ("GET", "/api/extensions/mcp"),
        ("POST", "/api/extensions/mcp"),
        ("GET", "/api/extensions/skills"),
        ("POST", "/api/extensions/skills"),
    ] {
        let resp = build_router(h.state())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .expect("构造请求"),
            )
            .await
            .expect("请求失败");
        assert!(
            resp.status() == StatusCode::UNAUTHORIZED
                || resp.status() == StatusCode::FORBIDDEN,
            "{method} {path} 未登录时返回了 {}，应当 401/403",
            resp.status()
        );
    }
}

async fn post_bad(h: &Harness, body: serde_json::Value) -> String {
    let resp = build_router(h.state())
        .oneshot(post_json("/api/extensions/mcp", TOKEN_A, body))
        .await
        .expect("请求失败");
    let st = resp.status();
    let text = body_text(resp).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "应当 400：{text}");
    let v = json_of(&text);
    let msg = v["error"]["detail"].as_str().expect("错误体缺 detail").to_string();
    assert!(
        msg.contains("下一步"),
        "错误必须带修复动作，不然用户只知道自己错了、不知道怎么改：{msg}"
    );
    msg
}

#[tokio::test]
async fn a_stdio_server_without_a_command_is_rejected_with_a_next_step() {
    let h = Harness::new("ext-mcp-no-command");
    seed_user(&h.db.bridge(), UID_A);
    let msg = post_bad(
        &h,
        serde_json::json!({"servers": [{"name": "a", "transport": "stdio"}]}),
    )
    .await;
    assert!(msg.contains("command"), "要说清缺哪个字段：{msg}");
    assert!(msg.contains("第 0 项"), "要说清是哪一项：{msg}");
}

#[tokio::test]
async fn an_http_server_without_a_url_is_rejected() {
    let h = Harness::new("ext-mcp-no-url");
    seed_user(&h.db.bridge(), UID_A);
    let msg = post_bad(
        &h,
        serde_json::json!({"servers": [{"name": "a", "transport": "streamable_http"}]}),
    )
    .await;
    assert!(msg.contains("url"), "{msg}");
}

#[tokio::test]
async fn a_url_on_a_stdio_server_is_rejected_instead_of_silently_ignored() {
    let h = Harness::new("ext-mcp-mixed");
    seed_user(&h.db.bridge(), UID_A);
    let mut s = stdio("a");
    s["url"] = serde_json::json!("https://example.com/mcp");
    let msg = post_bad(&h, serde_json::json!({"servers": [s]})).await;
    assert!(msg.contains("url"), "{msg}");
}

#[tokio::test]
async fn a_duplicate_name_in_one_post_is_rejected() {
    // 名字是主键的一部分。重复时「差分软删」失去意义：到底留哪个？
    let h = Harness::new("ext-mcp-dup");
    seed_user(&h.db.bridge(), UID_A);
    let msg = post_bad(
        &h,
        serde_json::json!({"servers": [stdio("same"), stdio("SAME")]}),
    )
    .await;
    assert!(msg.contains("唯一") || msg.contains("重复"), "{msg}");
}

#[tokio::test]
async fn an_unknown_field_is_rejected_rather_than_silently_dropped() {
    // 静默丢弃字段 = 用户以为自己配了、其实没生效。
    let h = Harness::new("ext-mcp-unknown");
    seed_user(&h.db.bridge(), UID_A);
    let mut s = stdio("a");
    s["autorize"] = serde_json::json!("Bearer x");
    let msg = post_bad(&h, serde_json::json!({"servers": [s]})).await;
    assert!(msg.contains("autorize"), "要说清是哪个字段不认识：{msg}");
}

#[tokio::test]
async fn a_capability_list_that_is_not_an_array_is_rejected() {
    let h = Harness::new("ext-mcp-caps-bad");
    seed_user(&h.db.bridge(), UID_A);
    let mut s = stdio("a");
    s["enabled_capabilities"] = serde_json::json!("all");
    let msg = post_bad(&h, serde_json::json!({"servers": [s]})).await;
    assert!(msg.contains("enabled_capabilities"), "{msg}");
}

// ------------------------------------------------------------------ SKILL

async fn post_skill(h: &Harness, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let resp = build_router(h.state())
        .oneshot(post_json("/api/extensions/skills", TOKEN_A, body))
        .await
        .expect("请求失败");
    let st = resp.status();
    let v = json_of(&body_text(resp).await);
    (st, v)
}

#[tokio::test]
async fn a_skill_is_stored_in_the_row_and_its_body_lands_on_disk() {
    let h = Harness::new("ext-skill-save");
    seed_user(&h.db.bridge(), UID_A);

    let (st, v) = post_skill(
        &h,
        serde_json::json!({
            "slug": "Code Review",
            "description": "审代码的固定流程",
            "content": "第一步：读 diff。\n第二步：找未处理的下拉。",
            "version": "0.2.0"
        }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    let f = h.skill_dir().join("code-review.md");
    assert!(f.is_file(), "正文必须落盘：{}", f.display());
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "第一步：读 diff。\n第二步：找未处理的下拉。"
    );

    // 行里只留摘要与路径，名字归一。
    let slug = common::text_of(
        &h.db.bridge(),
        "SELECT name AS v FROM skills WHERE name='code-review'",
    );
    assert_eq!(slug, "code-review");
    let path = common::text_of(
        &h.db.bridge(),
        "SELECT install_path AS v FROM skills WHERE name='code-review'",
    );
    assert_eq!(path, f.to_string_lossy());
}

#[tokio::test]
async fn a_skill_shows_up_as_a_tool_named_after_its_slug() {
    // 抄 Octop 的 `SkillListItem.tool_name`：SKILL 在模型眼里就是一个工具。
    // 少了 tool_name，前端就不知道该把它挂到模型的哪个槽位。
    let h = Harness::new("ext-skill-tool-name");
    seed_user(&h.db.bridge(), UID_A);
    post_skill(
        &h,
        serde_json::json!({"slug": "code-review", "content": "读 diff 再说话。"}),
    )
    .await;

    let resp = build_router(h.state())
        .oneshot(req("GET", "/api/extensions/skills", TOKEN_A))
        .await
        .expect("请求失败");
    let v = json_of(&body_text(resp).await);
    let s = &v["skills"][0];
    assert_eq!(s["slug"], serde_json::json!("code-review"));
    assert_eq!(s["tool_name"], serde_json::json!("code-review"));
    assert_eq!(s["kind"], serde_json::json!("workspace"));
    assert_eq!(s["content_chars"], serde_json::json!(11), "「读 diff 再说话。」11 个字符");
    assert!(
        s.get("content_missing").is_none(),
        "文件在磁盘上就不该报缺失"
    );
}

#[tokio::test]
async fn a_row_without_a_body_on_disk_is_flagged_rather_than_reported_as_empty() {
    // 库里说有、正文没了：这是「装了一半」。报成「正文为空」会让用户
    // 以为是内容写错，实际是文件被删了或目录没挂上。
    let h = Harness::new("ext-skill-missing-body");
    seed_user(&h.db.bridge(), UID_A);
    post_skill(
        &h,
        serde_json::json!({"slug": "gone", "content": "原本在这里的一段做法。"}),
    )
    .await;
    std::fs::remove_file(h.skill_dir().join("gone.md")).expect("删掉正文文件");

    let resp = build_router(h.state())
        .oneshot(req("GET", "/api/extensions/skills", TOKEN_A))
        .await
        .expect("请求失败");
    let v = json_of(&body_text(resp).await);
    assert_eq!(v["skills"][0]["content_missing"], serde_json::json!(true));
    assert_eq!(v["skills"][0]["content_chars"], serde_json::json!(0));
}

#[tokio::test]
async fn a_skill_body_that_is_empty_or_too_long_is_rejected_with_a_reason() {
    let h = Harness::new("ext-skill-limits");
    seed_user(&h.db.bridge(), UID_A);

    let (st, v) = post_skill(
        &h,
        serde_json::json!({"slug": "blank", "content": "   \n  "}),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"]["detail"].as_str().unwrap().contains("下一步"));

    let (st, v) = post_skill(
        &h,
        serde_json::json!({"slug": "huge", "content": "长".repeat(20_001)}),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    let msg = v["error"]["detail"].as_str().unwrap();
    assert!(msg.contains("20000"), "要报出上限值：{msg}");
    assert!(msg.contains("下一步"), "{msg}");
}

#[tokio::test]
async fn a_skill_name_looking_like_a_path_still_lands_inside_the_skill_directory() {
    // slug 直接来自 URL/body。归一把 `/`、`.` 都换成连字符，所以
    // `../../../etc/passwd` 变成 `etc-passwd` —— 落在技能目录里的一个普通
    // 文件名，**不是**拒绝而是收窄。两条路都安全，但「实际发生了什么」必须
    // 说清楚：这条断言盯的就是文件真的没跑到目录外。
    let h = Harness::new("ext-skill-traversal");
    seed_user(&h.db.bridge(), UID_A);
    let before = std::fs::read_to_string("/etc/passwd").unwrap_or_default();

    let (st, v) = post_skill(
        &h,
        serde_json::json!({"slug": "../../../etc/passwd", "content": "内容"}),
    )
    .await;
    assert!(
        st == StatusCode::OK || st == StatusCode::BAD_REQUEST,
        "要么按归一后的名字收下、要么拒掉，不能是别的：{v}"
    );

    if st == StatusCode::OK {
        let inside = h.skill_dir().join("etc-passwd.md");
        assert!(
            inside.is_file(),
            "归一后应当落在技能目录内：{}",
            inside.display()
        );
        assert!(
            h.skill_dir().starts_with(h.skill_dir().parent().unwrap()),
            "技能目录本身没被挪走"
        );
    }
    assert_eq!(
        std::fs::read_to_string("/etc/passwd").unwrap_or_default(),
        before,
        "/etc/passwd 被改写了 —— 路径穿越没挡住"
    );
    assert!(
        !std::path::Path::new("etc-passwd.md").is_file(),
        "文件落到了当前工作目录（技能目录之外）"
    );
}

#[tokio::test]
async fn deleting_a_skill_removes_both_the_row_and_the_body() {
    let h = Harness::new("ext-skill-delete");
    seed_user(&h.db.bridge(), UID_A);
    post_skill(
        &h,
        serde_json::json!({"slug": "temp", "content": "马上要删掉的一段做法。"}),
    )
    .await;
    let f = h.skill_dir().join("temp.md");
    assert!(f.is_file());

    let resp = build_router(h.state())
        .oneshot(req("DELETE", "/api/extensions/skills/temp", TOKEN_A))
        .await
        .expect("请求失败");
    let v = json_of(&body_text(resp).await);
    assert_eq!(v["deleted"], serde_json::json!(true));
    assert_eq!(v["file_removed"], serde_json::json!(true));
    assert!(!f.exists(), "正文文件也要删掉，否则是孤儿");

    let resp = build_router(h.state())
        .oneshot(req("DELETE", "/api/extensions/skills/temp", TOKEN_A))
        .await
        .expect("请求失败");
    assert_eq!(
        json_of(&body_text(resp).await)["deleted"],
        serde_json::json!(false),
        "重复删必须幂等"
    );
}

#[tokio::test]
async fn one_user_never_sees_another_users_skills() {
    let h = Harness::new("ext-skill-isolation");
    seed_user(&h.db.bridge(), UID_A);
    seed_user(&h.db.bridge(), UID_B);
    post_skill(
        &h,
        serde_json::json!({"slug": "a-private", "content": "A 的私有做法。"}),
    )
    .await;

    let resp = build_router(h.state())
        .oneshot(req("GET", "/api/extensions/skills", TOKEN_B))
        .await
        .expect("请求失败");
    let v = json_of(&body_text(resp).await);
    assert_eq!(v["skills"].as_array().unwrap().len(), 0);
}

// ------------------------------------------------- SKILL 挂进对话的工具表
//
// 下面这组盯的是「界面上装了技能，模型那边到底看不看得见」。
// 之前只做到存储层：GET /api/extensions/skills 能列出 SKILL，但那不代表
// 对话里真的有这个工具。中间这一段（`as_tool_spec` → `ToolRegistry`）没接上时，
// 界面上一切正常，模型却永远不知道技能的存在 —— 没有任何报错会指向它。

/// 按某个用户的身份，造出这次对话真正会用的那一份工具表。
async fn tool_table(harness: &Harness, uid: &str) -> quill_server::tools::ToolRegistry {
    let u = user_id(uid);
    quill_server::tools::ToolRegistry::builtin(Arc::new(harness.state()), u)
        .with_skills(harness.db.bridge().as_ref(), u, &harness.skill_dir())
        .await
        .expect("挂 SKILL 进工具表必须成功")
}

fn spec_names(r: &quill_server::tools::ToolRegistry) -> Vec<String> {
    r.specs().into_iter().map(|s| s.name).collect()
}

#[tokio::test]
async fn an_enabled_skill_reaches_the_model_as_a_tool_and_can_be_called() {
    let h = Harness::new("ext-skill-into-tools");
    seed_user(&h.db.bridge(), UID_A);
    let (st, v) = post_skill(
        &h,
        serde_json::json!({
            "slug": "code-review",
            "description": "审代码的固定流程",
            "content": "第一步：读 diff。第二步：找未处理的下拉。"
        }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    let r = tool_table(&h, UID_A).await;

    // 1. 工具真的在列表里（不是只在 GET 里出现过）。
    let names = spec_names(&r);
    assert!(
        names.contains(&"code-review".to_string()),
        "SKILL 必须在对话的工具表里，实际：{names:?}"
    );
    // 2. 内置工具不能被挤掉。
    assert!(
        names.contains(&"list_experts".to_string())
            && names.contains(&"get_expert_detail".to_string()),
        "内置工具应当仍在：{names:?}"
    );
    // 3. 模型得知道传什么参数。
    let spec = r
        .specs()
        .into_iter()
        .find(|s| s.name == "code-review")
        .expect("刚才断言过它在");
    assert_eq!(
        spec.parameters["required"],
        serde_json::json!(["task"]),
        "没有必填参数，模型只能瞎猜"
    );
    // 4. 正文进了描述 —— 「SKILL 即工具」靠的就是这个。
    assert!(
        spec.description.contains("读 diff"),
        "正文必须进工具描述，模型才知道这套方法是什么：{}",
        spec.description
    );

    // 5. 真调用一次，走的是对话里同一条执行路径。
    let call = quill_provider::ToolCall::new(
        "c1",
        "code-review",
        serde_json::json!({"task": "审一下 x.rs 里的下拉"}),
    );
    let out = r.call(&call).expect("SKILL 工具必须能执行");
    assert!(out.contains("code-review"), "要说清用的是哪套方法：{out}");
    assert!(out.contains("审一下 x.rs 里的下拉"), "任务要回给模型：{out}");
}

#[tokio::test]
async fn a_disabled_skill_is_not_offered_to_the_model() {
    let h = Harness::new("ext-skill-disabled");
    seed_user(&h.db.bridge(), UID_A);
    let (st, v) = post_skill(
        &h,
        serde_json::json!({
            "slug": "off-switch",
            "content": "用户明确关掉的一套做法。",
            "enabled": false
        }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    let names = spec_names(&tool_table(&h, UID_A).await);
    assert!(
        !names.contains(&"off-switch".to_string()),
        "关掉的技能不该出现在工具表里 —— 界面上禁用等于模型看不见：{names:?}"
    );
}

#[tokio::test]
async fn a_skill_whose_body_vanished_from_disk_is_not_offered_as_an_empty_tool() {
    // 库里说有、正文没了。挂一个 description 为空的工具进去，模型会调到一个
    // 必然没有产出的东西，而界面上这个技能还显示「已启用」。
    let h = Harness::new("ext-skill-body-vanished");
    seed_user(&h.db.bridge(), UID_A);
    post_skill(
        &h,
        serde_json::json!({"slug": "half-installed", "content": "原本在这里的做法。"}),
    )
    .await;
    std::fs::remove_file(h.skill_dir().join("half-installed.md")).expect("删掉正文文件");

    let names = spec_names(&tool_table(&h, UID_A).await);
    assert!(
        !names.contains(&"half-installed".to_string()),
        "正文缺失的技能不该注册成工具：{names:?}"
    );
}

#[tokio::test]
async fn a_dash_named_skill_coexists_with_the_underscore_named_builtin() {
    // 「最像撞名」的真实情况：SKILL 叫 `list-experts`，内置工具叫 `list_experts`。
    // 两者不是同一个名字，必须各自存在 —— 归一不会把连字符变成下划线。
    let h = Harness::new("ext-skill-shadow");
    seed_user(&h.db.bridge(), UID_A);
    let (st, v) = post_skill(
        &h,
        serde_json::json!({
            "slug": "list-experts",
            "description": "用户自己写的一套列专家方法",
            "content": "先按领域过滤，再排序。"
        }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    let r = tool_table(&h, UID_A).await;
    let names = spec_names(&r);
    assert!(
        names.contains(&"list-experts".to_string()) && names.contains(&"list_experts".to_string()),
        "两个不同的工具都该在：{names:?}"
    );
    let builtin = r
        .specs()
        .into_iter()
        .find(|s| s.name == "list_experts")
        .expect("内置工具必须还在");
    assert!(
        !builtin.description.contains("先按领域过滤"),
        "内置工具的描述被 SKILL 顶掉了：{}",
        builtin.description
    );
}

#[tokio::test]
async fn the_schema_itself_forbids_a_skill_from_taking_a_builtin_tools_underscored_name() {
    // 上面那条测试说明「连字符名」不会撞上「下划线名」。这一条钉住的是另一半：
    // 连下划线都写不进 `skills.name`，所以 `list_experts` 这种名字只能是内置
    // 工具的专利。守卫（`tools::veto` 的同名分支）在今天的 schema 下不可达，
    // 靠的是这条 CHECK，不是靠代码里那个 if。
    let h = Harness::new("ext-skill-name-check");
    seed_user(&h.db.bridge(), UID_A);
    let b = user_id(UID_A).as_bytes().to_vec();
    let err = h
        .db
        .bridge()
        .call(move |pool, _rt| {
            Box::pin(async move {
                sqlx::query(
                    "INSERT INTO skills (user_id, name, version, source, source_ref, description, \
                     enabled, content_hash, install_path, tool_allowlist_json, created_at, \
                     updated_at, deleted_at) VALUES (?,?,'0.1.0','local',NULL,'',\
                     1, zeroblob(32),'','[]',0,0,NULL)",
                )
                .bind(&b)
                .bind("list_experts")
                .execute(&pool)
                .await
                .map_err(|e| storage_error("插入带下划线的 skills 行", e))?;
                Ok(())
            })
        })
        .expect_err("带下划线的名字必须被 CHECK 拒掉");
    assert!(
        format!("{err}").contains("CHECK constraint failed"),
        "要看到 CHECK 失败，而不是别的错误：{err}"
    );
}

#[tokio::test]
async fn one_users_skill_never_becomes_a_tool_for_another_user() {
    let h = Harness::new("ext-skill-tool-isolation");
    seed_user(&h.db.bridge(), UID_A);
    seed_user(&h.db.bridge(), UID_B);
    post_skill(
        &h,
        serde_json::json!({"slug": "a-private", "content": "A 的私有做法。"}),
    )
    .await;

    let names = spec_names(&tool_table(&h, UID_B).await);
    assert!(
        !names.contains(&"a-private".to_string()),
        "B 的工具表里绝不能出现 A 的技能：{names:?}"
    );
}

// ------------------------------------------------- 界面报「模型看得见」的那一项
//
// GET /api/extensions/skills 与 tools::with_skills 调的是同一个
// `tools::skill_visibility`。下面这组盯的就是**两边不许漂**：如果哪天有人只改了
// 其中一边，界面上就会出现「已启用」的技能，而模型工具表里没有它 —— 那种故障
// 没有任何报错会指向它。

/// 读 GET /api/extensions/skills 的 `skills` 数组。
async fn listed_skills(h: &Harness, token: &str) -> Vec<serde_json::Value> {
    let resp = build_router(h.state())
        .oneshot(req("GET", "/api/extensions/skills", token))
        .await
        .expect("请求失败");
    let v = json_of(&body_text(resp).await);
    v["skills"].as_array().cloned().expect("skills 必须是数组")
}

fn find_skill<'a>(list: &'a [serde_json::Value], slug: &str) -> &'a serde_json::Value {
    list.iter()
        .find(|s| s["slug"] == serde_json::json!(slug))
        .unwrap_or_else(|| panic!("列表里没有 {slug}：{list:?}"))
}

#[tokio::test]
async fn the_api_says_model_can_see_a_skill_exactly_when_it_reaches_the_tool_table() {
    // 这一条是本组的支点：界面上那个布尔值，必须等于「工具表里到底有没有它」。
    let h = Harness::new("ext-skill-model-can-see");
    seed_user(&h.db.bridge(), UID_A);
    for (slug, enabled) in [("kept", true), ("turned-off", false)] {
        post_skill(
            &h,
            serde_json::json!({"slug": slug, "content": "一套做法。", "enabled": enabled}),
        )
        .await;
    }

    let list = listed_skills(&h, TOKEN_A).await;
    let names = spec_names(&tool_table(&h, UID_A).await);

    for slug in ["kept", "turned-off"] {
        let item = find_skill(&list, slug);
        let claimed = item["model_can_see"].as_bool().expect("必须有这个字段");
        let actually = names.contains(&slug.to_string());
        assert_eq!(
            claimed, actually,
            "{slug}：界面报 model_can_see={claimed}，工具表里实际上有={actually}。\
             两边漂了 —— 界面上就是在说谎。"
        );
    }
    assert_eq!(
        find_skill(&list, "kept")["model_can_see"],
        serde_json::json!(true),
        "启用了、正文也在，就该报模型看得见"
    );
}

#[tokio::test]
async fn a_skill_whose_body_vanished_is_reported_unmountable_with_a_reason() {
    // 「库里有行」不等于「模型看得见」。正文文件没了的话，with_skills 会跳过它，
    // 界面就必须说清楚为什么 —— 否则用户只会看到「已启用」却怎么都不生效。
    let h = Harness::new("ext-skill-unmountable-reason");
    seed_user(&h.db.bridge(), UID_A);
    post_skill(
        &h,
        serde_json::json!({"slug": "half", "content": "原本在这里的做法。"}),
    )
    .await;
    std::fs::remove_file(h.skill_dir().join("half.md")).expect("删掉正文文件");

    let list = listed_skills(&h, TOKEN_A).await;
    let item = find_skill(&list, "half");
    assert_eq!(item["model_can_see"], serde_json::json!(false));
    assert_eq!(item["content_missing"], serde_json::json!(true));
    let why = item["not_mounted_reason"]
        .as_str()
        .expect("没挂上就必须说原因");
    assert!(
        why.contains("正文"),
        "原因要指向正文丢失：{why}"
    );
    assert!(
        !spec_names(&tool_table(&h, UID_A).await).contains(&"half".to_string()),
        "正文没了就不该进工具表"
    );
}

#[tokio::test]
async fn a_mounted_skill_carries_no_not_mounted_reason() {
    // 挂了就是挂了。留一个空的 not_mounted_reason 会让前端有机会把
    // 「没挂上，原因：」这种话显示出来。
    let h = Harness::new("ext-skill-no-reason-when-ok");
    seed_user(&h.db.bridge(), UID_A);
    post_skill(
        &h,
        serde_json::json!({"slug": "fine", "content": "一套做法。"}),
    )
    .await;

    let list = listed_skills(&h, TOKEN_A).await;
    let item = find_skill(&list, "fine");
    assert_eq!(item["model_can_see"], serde_json::json!(true));
    assert!(
        item.get("not_mounted_reason").is_none(),
        "挂上了就不该有 not_mounted_reason：{item}"
    );
}

#[tokio::test]
async fn a_disabled_skill_is_listed_but_not_reported_as_visible() {
    // 停用的技能仍然列得出来（用户要能看到自己装过什么、能再打开），
    // 但绝不能报成模型看得见。
    let h = Harness::new("ext-skill-disabled-listed");
    seed_user(&h.db.bridge(), UID_A);
    post_skill(
        &h,
        serde_json::json!({"slug": "paused", "content": "一套做法。", "enabled": false}),
    )
    .await;

    let list = listed_skills(&h, TOKEN_A).await;
    let item = find_skill(&list, "paused");
    assert_eq!(item["enabled"], serde_json::json!(false));
    assert_eq!(item["model_can_see"], serde_json::json!(false));
    assert!(
        item.get("not_mounted_reason").is_none(),
        "停用是用户自己的选择，不是故障，不该报成「没挂上，原因：…」：{item}"
    );
}
