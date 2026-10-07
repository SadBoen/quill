//! `GET/POST /api/extensions/bundle/*` —— 设置包导出 / 导入的契约（需求 4 多端同步）。
//!
//! 钉住四件事：
//!
//! 1. **凭据只进不出**：导出的 MCP 配置里**没有** `env` / `headers`，且响应点名剔掉了什么；
//!    但库里原来的 env **还在**（不是「导出顺手删了凭据」）。
//! 2. **导入是全量替换**：清单里没有的 MCP 服务器会被软删 —— 这是「同步」该有的语义。
//! 3. **版本不匹配直接拒**：不「尽力解析」，猜错的清单会写出半套配置。
//! 4. **技能正文往返**：导入写盘 + 落库，导出能把正文带回来。

mod common;

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use quill_server::auth::{AuthContext, EnvTokenResolver};
use quill_server::config::Config;
use quill_server::routes::build_router;
use quill_server::state::AppState;

use common::{scalar_i64, seed_token_user, TestDb};

const UID_A: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN_A: &str = "tok-a";

fn user_id() -> quill_domain::UserId {
    quill_domain::UserId::parse(UID_A).expect("测试 UID 必须合法")
}

/// 每个用例一份自己的临时目录：`Config.db_path` 决定 SKILL 正文目录
/// （`<db 同级>/skills`），用例之间不串。
struct Harness {
    db: TestDb,
}

impl Harness {
    fn new(label: &str) -> Self {
        Self {
            db: TestDb::new(label),
        }
    }

    fn config(&self) -> Config {
        let mut cfg = Config::from_env();
        cfg.db_path = PathBuf::from(self.db.path());
        cfg
    }

    fn state(&self) -> AppState {
        let resolver = EnvTokenResolver::new(vec![(
            TOKEN_A.to_string(),
            AuthContext {
                user_id: user_id(),
                is_admin: true,
            },
        )]);
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

    async fn setup(label: &str) -> Self {
        let h = Self::new(label);
        seed_token_user(&h.db.bridge(), &user_id(), "tester", true).await;
        h
    }
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

fn req(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN_A}"))
        .body(Body::empty())
        .expect("构造请求")
}

fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN_A}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("构造请求")
}

async fn send(h: &Harness, request: Request<Body>) -> (StatusCode, String) {
    let resp = build_router(h.state())
        .oneshot(request)
        .await
        .expect("请求失败");
    let st = resp.status();
    (st, body_text(resp).await)
}

async fn export(h: &Harness) -> serde_json::Value {
    let (st, text) = send(h, req("GET", "/api/extensions/bundle/export")).await;
    assert_eq!(st, StatusCode::OK, "导出应当成功：{text}");
    json_of(&text)
}

async fn import(h: &Harness, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let (st, text) = send(h, post_json("/api/extensions/bundle/import", body)).await;
    (st, json_of(&text))
}

/// 存一份带凭据的 MCP 配置（复用 `POST /api/extensions/mcp`）。
fn stdio_with_secret(name: &str, secret: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "transport": "stdio",
        "command": "quill-no-such-mcp-binary",
        "args": ["-y", "some-server"],
        "env": {"API_KEY": secret},
        "enabled_capabilities": null,
        "timeout_ms": 30000
    })
}

// ------------------------------------------------------------------ 凭据只进不出

#[tokio::test]
async fn export_never_carries_credentials_but_the_row_keeps_them() {
    let h = Harness::setup("bundle-export-redact").await;

    let (st, text) = send(
        &h,
        post_json(
            "/api/extensions/mcp",
            serde_json::json!({ "servers": [stdio_with_secret("notes", "s3cr3t-key")] }),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "存 MCP 应当成功：{text}");

    let v = export(&h).await;
    let servers = v["mcp_servers"].as_array().expect("导出里要有 mcp_servers");
    assert_eq!(servers.len(), 1, "存了一台就该导出一台：{v}");
    let s = &servers[0];

    // 导出对象里**不许**出现凭据字段 —— 连键都不许有。
    assert!(s.get("env").is_none(), "env 是凭据，不许进导出：{s}");
    assert!(
        s.get("headers").is_none(),
        "headers 是凭据，不许进导出：{s}"
    );
    // 非凭据字段照旧带上（不要把整条配置也一起抹掉）。
    assert_eq!(s["name"], serde_json::json!("notes"));
    assert_eq!(s["transport"], serde_json::json!("stdio"));

    // 响应要说清剔掉了什么，用户才知道「导到新机器后要补什么」。
    let redacted = v["redacted"]
        .as_array()
        .expect("必须有 redacted 清单")
        .iter()
        .filter_map(|x| x.as_str())
        .collect::<Vec<_>>();
    assert!(
        redacted.iter().any(|r| r.contains("env")),
        "redacted 要点名 env：{redacted:?}"
    );
    assert!(
        redacted.iter().any(|r| r.contains("headers")),
        "redacted 要点名 headers：{redacted:?}"
    );

    // 关键反向断言：导出**没有**顺手把库里的凭据删掉。
    let n = scalar_i64(
        &h.db.bridge(),
        "SELECT COUNT(*) FROM mcp_servers WHERE name = 'notes'",
    );
    assert_eq!(n, 1, "导出是只读的，行不该消失");
    let has_secret = scalar_i64(
        &h.db.bridge(),
        "SELECT COUNT(*) FROM mcp_servers WHERE name = 'notes' AND length(env_json) > 0",
    );
    assert_eq!(has_secret, 1, "凭据仍在库里 —— 导出只读不写");
}

// ------------------------------------------------------------------ 导入 = 全量替换

#[tokio::test]
async fn import_replaces_the_mcp_list_wholesale() {
    let h = Harness::setup("bundle-import-replace").await;

    // 先存一台「旧的」。
    let (st, text) = send(
        &h,
        post_json(
            "/api/extensions/mcp",
            serde_json::json!({ "servers": [stdio_with_secret("old-one", "k")] }),
        ),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{text}");

    // 导入只带一台「新的」。
    let (st, v) = import(
        &h,
        serde_json::json!({
            "bundle_version": 1,
            "mcp_servers": [{
                "name": "fresh",
                "transport": "stdio",
                "command": "quill-no-such-mcp-binary",
                "args": [],
                "enabled": true,
                "timeout_ms": 30000
            }],
            "skills": []
        }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "导入应当成功：{v}");

    // 导入后的清单里**只有** fresh —— old-one 被软删（这就是同步的语义）。
    let (st, text) = send(&h, req("GET", "/api/extensions/mcp")).await;
    assert_eq!(st, StatusCode::OK, "{text}");
    let listed = json_of(&text);
    let names = listed["servers"]
        .as_array()
        .expect("要有 servers")
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["fresh"], "全量替换后只剩清单里的：{listed}");

    // 导入的响应里也应当能看见替换后的清单。
    let after = v["mcp_servers"].as_array().expect("响应带 mcp_servers");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0]["name"], serde_json::json!("fresh"));
}

// ------------------------------------------------------------------ 版本不匹配直接拒

#[tokio::test]
async fn import_rejects_a_bundle_version_it_does_not_know() {
    let h = Harness::setup("bundle-import-version").await;

    let (st, v) = import(
        &h,
        serde_json::json!({
            "bundle_version": 999,
            "mcp_servers": [],
            "skills": []
        }),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "不认识的版本要 4xx，不能尽力解析：{v}"
    );
    let detail = v["error"]["detail"].as_str().unwrap_or_default();
    assert!(detail.contains("999"), "要点名是哪个版本：{detail}");
    assert!(
        detail.contains("下一步"),
        "要给出「照着做」的下一步：{detail}"
    );
}

// ------------------------------------------------------------------ 技能正文往返

#[tokio::test]
async fn imported_skill_body_lands_on_disk_and_round_trips_out() {
    let h = Harness::setup("bundle-skill-roundtrip").await;

    let body = "# 分诊\n\n先看有没有红旗症状。";
    let (st, v) = import(
        &h,
        serde_json::json!({
            "bundle_version": 1,
            "mcp_servers": [],
            "skills": [{
                "name": "triage",
                "version": "0.1.0",
                "description": "分诊技能",
                "enabled": false,
                "source": "bundle",
                "content": body
            }]
        }),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "导入技能应当成功：{v}");

    // 先直连库确认**真的写进去了**（HTTP 列表若查不到，才排除「没写」这一可能）。
    let rows = scalar_i64(
        &h.db.bridge(),
        "SELECT COUNT(*) FROM skills WHERE name = 'triage'",
    );
    assert_eq!(rows, 1, "import 之后库里应当有 triage 行：{v}");

    // 列表里能看到它，且正文长度对得上（说明真的写进去了）。
    let (st, text) = send(&h, req("GET", "/api/extensions/skills")).await;
    assert_eq!(st, StatusCode::OK, "{text}");
    let listed = json_of(&text);
    let item = listed["skills"]
        .as_array()
        .expect("要有 skills")
        .iter()
        .find(|s| s["slug"] == serde_json::json!("triage"))
        .expect("导入的技能应当被列出来");
    assert_eq!(
        item["content_chars"],
        serde_json::json!(body.chars().count()),
        "正文长度要对得上：{item}"
    );

    // 再导出一次，正文必须原样回来 —— 不然「同步」只是搬了个空壳。
    let v = export(&h).await;
    let sk = v["skills"]
        .as_array()
        .expect("导出带 skills")
        .iter()
        .find(|s| s["name"] == serde_json::json!("triage"))
        .expect("导出的技能里要有 triage");
    assert_eq!(sk["content"], serde_json::json!(body), "正文要往返：{sk}");
    // 导出的技能对象里**不许**有本机才有的 install_path —— 那是落库信息不是配置。
    assert!(
        sk.get("install_path").is_none(),
        "install_path 是本机的，不该进导出：{sk}"
    );
}
