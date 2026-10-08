//! 升级三条路由的 HTTP 契约（queue Q038–Q040）。
//!
//! 钉住四件事：
//!
//! 1. **三条都只允许 admin**：`check` 会替服务端访问外部更新源、`prepare`
//!    会写出整份数据根的备份、`history` 会回显服务端绝对路径。非 admin 必须
//!    403（不是 401），且 403 之后磁盘上不能多出任何东西。
//! 2. **「不知道」不许被写成「已是最新」**：没配更新源、来源连不上、清单里的
//!    版本不可比较 —— 这三种情况下 `has_update` 必须是 JSON `null`，绝不出现
//!    `false`。`false` 只允许在真取到清单、真比出「不高于当前版本」时出现。
//! 3. **`prepare` 真落备份**：断言备份目录、清单、数据库快照真的在磁盘上，
//!    且响应里的摘要与磁盘重算一致。只断言「响应自洽」的话，一个把字段写死的
//!    桩也能过。
//! 4. **历史来自真落盘的记录**：prepare 之后列表里有且内容逐字一致；
//!    没有记录（200 空 + note）与读不出来（5xx）必须能分开 —— 读失败吞成
//!    空列表，用户会以为「从来没升级过」。
//!
//! 更新源用**真 HTTP** 验：本文件里起一个本地 TCP 服务器返回清单，服务端走
//! reqwest 真发请求、真解析、真比较。断源用 127.0.0.1:1（必然连不上）。

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

use common::TestDb;

const UID: &str = "0192b7c8-0000-7000-8000-000000000001";
const TOKEN: &str = "tok-upgrade";
const UPGRADE_ENV: &str = quill_server::api_upgrade::MANIFEST_URL_ENV;

/// `QUILL_UPGRADE_MANIFEST_URL` 是**进程级**环境变量：本文件里凡是碰它的用例
/// 都必须先拿这把锁，否则并行执行时会互相把对方的来源改掉（那条用例测的就成了
/// 夹具，而不是行为）。用 tokio 的锁而不是 std 的：guard 要跨 await 持有，
/// std 的 guard 跨 await 会被 clippy 的 await_holding_lock 判红。
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 与 `backup_http.rs` 同构：一例一份真实临时库，落盘路径都在临时目录里。
struct Harness {
    db: TestDb,
}

impl Harness {
    fn new(label: &str) -> Self {
        Self {
            db: TestDb::new(label),
        }
    }

    fn data_root(&self) -> PathBuf {
        PathBuf::from(self.db.path())
            .parent()
            .expect("临时库必有父目录")
            .to_path_buf()
    }

    /// 备份根 = 数据根的**兄弟** `backups`（与 `api_backup::backup_root` 同一处推导）。
    fn backup_root(&self) -> PathBuf {
        self.data_root()
            .parent()
            .expect("临时目录必有上级")
            .join("backups")
    }

    fn history_path(&self) -> PathBuf {
        self.data_root().join("upgrade-history.jsonl")
    }

    fn config(&self) -> Config {
        let mut cfg = Config::from_env();
        cfg.db_path = PathBuf::from(self.db.path());
        cfg
    }

    fn state(&self) -> AppState {
        self.state_as(true)
    }

    /// 同一套装置，但令牌可以**不是** admin —— 403 边界必须用真的非 admin 令牌打。
    fn state_as(&self, is_admin: bool) -> AppState {
        let resolver = EnvTokenResolver::new(vec![(
            TOKEN.to_string(),
            AuthContext {
                user_id: quill_domain::UserId::parse(UID).expect("测试 UID 必须合法"),
                is_admin,
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

    fn write_data(&self, rel: &str, content: &[u8]) -> PathBuf {
        let p = self.data_root().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("建数据子目录");
        }
        std::fs::write(&p, content).expect("写用户数据");
        p
    }
}

/// 用完即删的目录守卫（备份根是共享的 `%TEMP%/backups`，只删本次自己的那份）。
struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `QUILL_UPGRADE_MANIFEST_URL` 的设置守卫：用例结束（含 panic）时清掉，
/// 不给后面的用例留一个不属于它的来源。
struct EnvVar {
    key: &'static str,
}

impl EnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        std::env::set_var(key, value);
        Self { key }
    }

    fn unset(key: &'static str) -> Self {
        std::env::remove_var(key);
        Self { key }
    }
}

impl Drop for EnvVar {
    fn drop(&mut self) {
        std::env::remove_var(self.key);
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

fn request(method: &str, path: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN}"));
    let payload = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            v.to_string()
        }
        None => String::new(),
    };
    builder.body(Body::from(payload)).expect("构造请求")
}

async fn call(h: &Harness, method: &str, path: &str, is_admin: bool) -> (StatusCode, String) {
    let resp = build_router(h.state_as(is_admin))
        .oneshot(request(method, path, None))
        .await
        .expect("请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    (status, text)
}

fn json_of(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("响应体不是 JSON（{e}）：{text}"))
}

fn manifest_file_count(manifest_text: &str) -> usize {
    manifest_text
        .lines()
        .find_map(|l| l.strip_prefix("file.count="))
        .and_then(|v| v.trim().parse().ok())
        .expect("清单里应有 file.count")
}

/// 起一个只回一份清单的本地 HTTP 服务器，返回 URL 与任务句柄（用完 abort）。
///
/// 不 mock reqwest：服务端还是要走真实的 TCP + HTTP/1.1 解析 + JSON 解析。
async fn spawn_manifest_server(body: String) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("绑定测试 HTTP 端口");
    let addr = listener.local_addr().expect("取本地地址");
    let url = format!("http://{addr}/upgrade-manifest.json");
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let body = body.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (url, handle)
}

// ---------------------------------------------------------------------------
// 权限：三条都只允许 admin
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_three_upgrade_routes_refuse_a_non_admin_with_403() {
    let h = Harness::new("upgrade-nonadmin");

    for (method, path) in [
        ("GET", "/api/upgrade/check"),
        ("POST", "/api/upgrade/prepare"),
        ("GET", "/api/upgrade/history"),
    ] {
        let (status, text) = call(&h, method, path, false).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} 对非 admin 必须 403（不是 401/404）：{text}"
        );
        let envelope = json_of(&text);
        assert_eq!(
            envelope["error"]["code"],
            serde_json::json!("forbidden"),
            "{method} {path} 必须走 forbidden 分支：{text}"
        );
        let next = envelope["error"]["next_step"].as_str().unwrap_or_default();
        assert!(
            next.contains("admin"),
            "{method} {path} 的 403 必须说明这是 admin 限制并给出下一步：{text}"
        );
        for leak in ["current_version", "backup_dir", "db_sha256"] {
            assert!(
                !text.contains(leak),
                "{method} {path} 的 403 不许夹带版本/备份信息（{leak}）：{text}"
            );
        }
    }

    // 403 之后磁盘上不能多出任何东西：非 admin 的 prepare 若真跑了备份，
    // 历史文件会先出现在这个 harness 自己的数据根里。
    assert!(
        !h.history_path().exists(),
        "非 admin 的 prepare 绝不能留下历史记录：{}",
        h.history_path().display()
    );
}

// ---------------------------------------------------------------------------
// Q038：更新源
// ---------------------------------------------------------------------------

/// 没配更新源：`has_update` / `latest_version` 必须是 `null` + note 说清原因。
///
/// 这条是红线本身：把 `null` 写成 `false`（「已是最新」），这条用例必须变红。
#[tokio::test]
async fn check_without_a_configured_source_says_unknown_instead_of_up_to_date() {
    let _lock = ENV_LOCK.lock().await;
    let _env = EnvVar::unset(UPGRADE_ENV);
    let h = Harness::new("upgrade-nosource");

    let (status, text) = call(&h, "GET", "/api/upgrade/check", true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let body = json_of(&text);

    assert_eq!(
        body["current_version"],
        serde_json::json!(env!("CARGO_PKG_VERSION")),
        "当前版本必须与 /api/version 同源（同一个 CARGO_PKG_VERSION）：{text}"
    );
    assert_eq!(body["source"], serde_json::json!("none"), "{text}");
    assert!(
        body["has_update"].is_null(),
        "没有更新源就没有「有没有新版」可言：必须是 null，不是 false：{text}"
    );
    assert!(
        body["latest_version"].is_null(),
        "没有来源时不许编一个最新版本号：{text}"
    );
    let note = body["note"].as_str().unwrap_or_default();
    assert!(note.contains("未配置更新源"), "note 必须点名原因：{text}");
    assert!(
        note.contains("不知道") && note.contains("已是最新"),
        "note 必须说清「不知道」不是「已是最新」：{text}"
    );
    assert!(
        !text.contains("\"has_update\":false"),
        "没配来源时出现 has_update:false 就是把「取不到」当「已是最新」：{text}"
    );
}

/// 配了真来源：服务端真发 HTTP、真比较，版本更高 → `has_update: true`。
#[tokio::test]
async fn check_fetches_a_real_manifest_and_reports_a_newer_version() {
    let _lock = ENV_LOCK.lock().await;
    let (url, server) = spawn_manifest_server(
        r#"{"version":"99.9.9","url":"https://example.invalid/q","notes":"演练清单"}"#.to_string(),
    )
    .await;
    let _env = EnvVar::set(UPGRADE_ENV, &url);
    let h = Harness::new("upgrade-manifest-newer");

    let (status, text) = call(&h, "GET", "/api/upgrade/check", true).await;
    server.abort();
    assert_eq!(status, StatusCode::OK, "{text}");
    let body = json_of(&text);

    assert_eq!(body["has_update"], serde_json::json!(true), "{text}");
    assert_eq!(
        body["latest_version"],
        serde_json::json!("99.9.9"),
        "{text}"
    );
    assert_eq!(body["source"], serde_json::json!("manifest_url"), "{text}");
    assert_eq!(body["source_url"], serde_json::json!(url), "{text}");
    assert_eq!(
        body["release_url"],
        serde_json::json!("https://example.invalid/q"),
        "{text}"
    );
    assert_eq!(body["notes"], serde_json::json!("演练清单"), "{text}");
    assert!(body["source_error"].is_null(), "{text}");
}

/// `false` 只允许出现在「真取到清单且比出不高」的时候 —— 这是它唯一的诚实用法。
#[tokio::test]
async fn check_reports_false_only_when_the_source_really_says_the_same_version() {
    let _lock = ENV_LOCK.lock().await;
    let current = env!("CARGO_PKG_VERSION");
    let (url, server) = spawn_manifest_server(format!("{{\"version\":\"{current}\"}}")).await;
    let _env = EnvVar::set(UPGRADE_ENV, &url);
    let h = Harness::new("upgrade-manifest-same");

    let (status, text) = call(&h, "GET", "/api/upgrade/check", true).await;
    server.abort();
    assert_eq!(status, StatusCode::OK, "{text}");
    let body = json_of(&text);
    assert_eq!(
        body["has_update"],
        serde_json::json!(false),
        "来源能问到且版本相同，false 才是诚实的：{text}"
    );
    assert_eq!(body["latest_version"], serde_json::json!(current), "{text}");
}

/// 清单里的版本不可比较 → `null`（不是 false），note 必须把规则端出来。
#[tokio::test]
async fn check_treats_an_incomparable_manifest_version_as_unknown() {
    let _lock = ENV_LOCK.lock().await;
    let (url, server) = spawn_manifest_server(r#"{"version":"latest"}"#.to_string()).await;
    let _env = EnvVar::set(UPGRADE_ENV, &url);
    let h = Harness::new("upgrade-manifest-bad-version");

    let (status, text) = call(&h, "GET", "/api/upgrade/check", true).await;
    server.abort();
    assert_eq!(status, StatusCode::OK, "{text}");
    let body = json_of(&text);
    assert!(
        body["has_update"].is_null(),
        "不可比较就是不知道，不许退化成 false：{text}"
    );
    assert_eq!(
        body["latest_version"],
        serde_json::json!("latest"),
        "取到的原文要如实回显：{text}"
    );
    let note = body["note"].as_str().unwrap_or_default();
    assert!(note.contains("不可比较"), "{text}");
    assert!(note.contains("null"), "note 必须说明结果是 null：{text}");
}

/// 配了但连不上：仍然是 `null` + `source_error`，且 note 明说「取不到不等于已是最新」。
#[tokio::test]
async fn check_on_an_unreachable_source_is_unknown_not_up_to_date() {
    let _lock = ENV_LOCK.lock().await;
    // 端口 1 是保留端口，本机上不会有东西监听：连接必然失败/被拒。
    let _env = EnvVar::set(UPGRADE_ENV, "http://127.0.0.1:1/upgrade-manifest.json");
    let h = Harness::new("upgrade-source-down");

    let (status, text) = call(&h, "GET", "/api/upgrade/check", true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let body = json_of(&text);
    assert!(
        body["has_update"].is_null(),
        "取不到更新源时 has_update 必须 null：{text}"
    );
    assert!(
        body["latest_version"].is_null(),
        "取不到时不许编最新版本号：{text}"
    );
    let err = body["source_error"].as_str().unwrap_or_default();
    assert!(!err.is_empty(), "取不到必须说出原因：{text}");
    let note = body["note"].as_str().unwrap_or_default();
    assert!(note.contains("取不到"), "note 必须承认这次没取到：{text}");
    assert!(
        note.contains("不等于"),
        "note 必须说清「取不到不等于已是最新」：{text}"
    );
    assert!(
        !text.contains("\"has_update\":false"),
        "网络失败绝不能被写成「已是最新」：{text}"
    );
}

// ---------------------------------------------------------------------------
// Q039：prepare 真落备份
// ---------------------------------------------------------------------------

/// prepare 必须在磁盘上落一份**可校验**的备份，并把当次事实写进历史。
#[tokio::test]
async fn prepare_really_backs_up_to_disk_and_history_lists_it() {
    let h = Harness::new("upgrade-prepare");
    h.write_data("u1/wiki/notes.md", "升级前的笔记".as_bytes());
    assert!(
        !h.history_path().exists(),
        "装置自检：prepare 之前历史文件不该存在"
    );

    let (status, text) = call(&h, "POST", "/api/upgrade/prepare", true).await;
    assert_eq!(status, StatusCode::CREATED, "prepare 应 201：{text}");
    let body = json_of(&text);

    assert_eq!(body["prepared"], serde_json::json!(true), "{text}");
    assert_eq!(
        body["upgraded"],
        serde_json::json!(false),
        "这条接口只做升级前备份，不执行升级，必须明说：{text}"
    );
    assert_eq!(
        body["app_version"],
        serde_json::json!(env!("CARGO_PKG_VERSION")),
        "{text}"
    );
    assert_eq!(
        body["history_appended"],
        serde_json::json!(true),
        "记账必须成功（否则历史会少一条）：{text}"
    );

    let dir = PathBuf::from(body["backup_dir"].as_str().expect("backup_dir 缺失"));
    let _cleanup = DirGuard(dir.clone());
    assert!(
        dir.starts_with(h.backup_root()),
        "备份必须落在备份根下：{} 不在 {} 里",
        dir.display(),
        h.backup_root().display()
    );
    assert!(
        !dir.starts_with(h.data_root()),
        "备份根必须在数据根**之外**（放里面会把正在写的目录一起复制进去）：{}",
        dir.display()
    );
    let name = body["backup_name"]
        .as_str()
        .expect("backup_name 缺失")
        .to_string();
    assert!(name.starts_with("pre-upgrade-"), "命名不合口径：{name}");
    assert_eq!(
        dir.file_name().map(|n| n.to_string_lossy().to_string()),
        Some(name.clone()),
        "backup_dir 的目录名必须就是 backup_name：{text}"
    );

    // 磁盘证据：清单 + 数据库快照 + 用户数据，三样都要真的在。
    let manifest_path = dir.join("MANIFEST");
    let db_snapshot = dir.join("db").join("quill.db");
    assert!(
        manifest_path.is_file(),
        "清单必须真落盘：{}",
        manifest_path.display()
    );
    assert!(
        db_snapshot.is_file(),
        "数据库快照必须真落盘：{}",
        db_snapshot.display()
    );
    assert!(
        dir.join("data").join("u1/wiki/notes.md").is_file(),
        "用户数据必须进备份：{}",
        dir.join("data").display()
    );

    // 响应里的摘要必须等于**磁盘上那份文件重算**的摘要。
    let manifest_text = std::fs::read_to_string(&manifest_path).expect("读清单");
    let on_disk = quill_backup::sha256_file(&db_snapshot).expect("重算快照摘要");
    assert_eq!(
        body["db_sha256"].as_str(),
        Some(on_disk.as_str()),
        "响应摘要必须来自磁盘文件，不是抄自己的：{text}"
    );
    assert!(
        manifest_text.contains(&on_disk),
        "清单里记的也必须是同一个摘要"
    );
    assert_eq!(
        body["files"].as_u64(),
        Some(manifest_file_count(&manifest_text) as u64),
        "响应文件数必须与清单一致：{text}"
    );
    assert!(
        body["total_bytes"].as_u64().expect("total_bytes 缺失") > 0,
        "总字节数必须为正：{text}"
    );
    assert!(body["excluded"].is_array(), "{text}");

    // 历史：真读回来，逐字段与当次 prepare 对齐。
    let (status, text) = call(&h, "GET", "/api/upgrade/history", true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let hist = json_of(&text);
    let entries = hist["entries"].as_array().expect("entries 必须是数组");
    assert_eq!(
        entries.len(),
        1,
        "刚做完一次 prepare，历史应恰好一条：{text}"
    );
    assert_eq!(entries[0]["backup_dir"], body["backup_dir"], "{text}");
    assert_eq!(entries[0]["backup_name"], body["backup_name"], "{text}");
    assert_eq!(entries[0]["db_sha256"], body["db_sha256"], "{text}");
    assert_eq!(
        entries[0]["app_version"],
        serde_json::json!(env!("CARGO_PKG_VERSION")),
        "{text}"
    );
    assert!(
        entries[0]["at_unix"].as_u64().expect("at_unix 缺失") > 0,
        "时间必须是真的：{text}"
    );
    assert!(
        hist["note"].is_null(),
        "有记录时不该再有「还没有记录」的 note：{text}"
    );

    // 交叉验证：升级前备份就是一份普通备份，verify 必须能校验通过。
    let resp = build_router(h.state())
        .oneshot(request(
            "POST",
            "/api/backup/verify",
            Some(serde_json::json!({ "name": name })),
        ))
        .await
        .expect("校验请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert_eq!(status, StatusCode::OK, "升级前备份必须可校验：{text}");
    assert_eq!(
        json_of(&text)["ok"],
        serde_json::json!(true),
        "升级前备份校验必须通过：{text}"
    );
}

/// 连做两次 prepare：两份备份、两条历史，各自独立（名字不许撞、记录不许串）。
#[tokio::test]
async fn two_prepares_create_two_backups_and_two_history_records() {
    let h = Harness::new("upgrade-prepare-twice");
    h.write_data("u1/notes.md", "内容".as_bytes());

    let (s1, t1) = call(&h, "POST", "/api/upgrade/prepare", true).await;
    assert_eq!(s1, StatusCode::CREATED, "{t1}");
    let b1 = json_of(&t1);
    let (s2, t2) = call(&h, "POST", "/api/upgrade/prepare", true).await;
    assert_eq!(s2, StatusCode::CREATED, "{t2}");
    let b2 = json_of(&t2);

    let d1 = PathBuf::from(b1["backup_dir"].as_str().expect("backup_dir"));
    let d2 = PathBuf::from(b2["backup_dir"].as_str().expect("backup_dir"));
    let _c1 = DirGuard(d1.clone());
    let _c2 = DirGuard(d2.clone());

    assert_ne!(d1, d2, "两次备份目录不能相同（撞名会毁掉上一份）");
    assert!(d1.join("MANIFEST").is_file() && d2.join("MANIFEST").is_file());

    let (status, text) = call(&h, "GET", "/api/upgrade/history", true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let hist = json_of(&text);
    let entries = hist["entries"].as_array().expect("entries 必须是数组");
    assert_eq!(entries.len(), 2, "两次 prepare 应有两条记录：{text}");
    assert_eq!(entries[0]["backup_dir"], b1["backup_dir"], "{text}");
    assert_eq!(entries[1]["backup_dir"], b2["backup_dir"], "{text}");
    assert_eq!(hist["count"], serde_json::json!(2), "{text}");
}

// ---------------------------------------------------------------------------
// Q040：历史的两种「空」与「读不出来」
// ---------------------------------------------------------------------------

/// 从没 prepare 过：200 + 空列表 + note 说清「确实没有记录」，不是读失败。
#[tokio::test]
async fn history_before_any_prepare_is_an_empty_list_with_a_reason() {
    let h = Harness::new("upgrade-history-empty");

    let (status, text) = call(&h, "GET", "/api/upgrade/history", true).await;
    assert_eq!(status, StatusCode::OK, "没有记录不是错误：{text}");
    let body = json_of(&text);
    assert_eq!(body["entries"], serde_json::json!([]), "{text}");
    assert_eq!(body["count"], serde_json::json!(0), "{text}");
    assert_eq!(
        body["path"],
        serde_json::json!(h.history_path().to_string_lossy().to_string()),
        "{text}"
    );
    let note = body["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("还没有升级记录"),
        "note 必须解释空列表：{text}"
    );
    assert!(
        note.contains("不是读失败"),
        "note 必须把「没有记录」与「读不出来」分开：{text}"
    );
    assert!(!h.history_path().exists(), "空列表不该顺手创建历史文件");
}

/// 读不出来 ≠ 没有记录：路径是目录、行解析不了，都必须 5xx 且说清原因；
/// 对照组是一行合法记录 → 200 列表。没有对照组的话，「恒 500」也能让前两条绿。
#[tokio::test]
async fn history_separates_unreadable_from_empty() {
    let h = Harness::new("upgrade-history-broken");
    let path = h.history_path();

    // 1) 路径是个目录：读取必然失败 → 5xx，绝不能回「空列表」。
    std::fs::create_dir_all(&path).expect("把历史路径建成目录");
    let (status, text) = call(&h, "GET", "/api/upgrade/history", true).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "读不出来必须是 5xx，不是 200 空列表：{text}"
    );
    assert!(text.contains("读不出来"), "必须说清是读失败：{text}");
    assert!(
        text.contains("不是") && text.contains("没有记录"),
        "必须与「没有记录」明确区分：{text}"
    );
    std::fs::remove_dir_all(&path).expect("移除目录");

    // 2) 第一行合法、第二行坏：5xx 且点名第 2 行 —— 不许静默跳过坏行。
    let good = "{\"at_unix\":1759851234,\"app_version\":\"0.1.0\",\
                \"backup_name\":\"pre-upgrade-1759851234000\",\"backup_dir\":\"/tmp/x\",\
                \"db_sha256\":\"aa\",\"files\":2,\"total_bytes\":7}\n";
    std::fs::write(
        &path,
        format!("{good}{{\"at_unix\": 2, \"app_version\": \"0.1.0\"}}\n"),
    )
    .expect("写历史");
    let (status, text) = call(&h, "GET", "/api/upgrade/history", true).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "坏行不许被静默跳过：{text}"
    );
    assert!(text.contains("第 2 行"), "必须点名坏在哪一行：{text}");

    // 3) 对照组：只有那一行合法记录 → 200 且列出来。
    std::fs::write(&path, good).expect("写合法历史");
    let (status, text) = call(&h, "GET", "/api/upgrade/history", true).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let body = json_of(&text);
    assert_eq!(
        body["entries"].as_array().map(Vec::len),
        Some(1),
        "合法记录必须能列出来：{text}"
    );
    assert_eq!(
        body["entries"][0]["backup_name"],
        serde_json::json!("pre-upgrade-1759851234000"),
        "{text}"
    );
    assert!(body["note"].is_null(), "{text}");
}
