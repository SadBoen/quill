//! 备份三条路由的 HTTP 契约。
//!
//! 钉住四件事：
//!
//! 1. **导出真的落盘**，而且响应里的 `db_sha256` 要与磁盘上那份快照重算的摘要
//!    一致 —— 只断言「响应自洽」的话，一个把字段写死的桩也能过。
//! 2. **校验不是装饰**：改了备份里的一个字节，verify 必须失败并点名那个文件。
//!    只测「导出后再校验通过」的话，verify 写成永远返回 `ok: true` 也能过。
//! 3. **客户端不能指定落盘位置**：`../escape`、`..\escape`、`C:\...`、`/etc/passwd`
//!    一律 4xx，且备份根之外**没有**被写进任何东西。
//! 4. **还原不许假成功**：响应必须说清「服务正持有数据库」并给出真实命令；
//!    同时备份路径之外一个字节都不能被改。

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
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
const TOKEN: &str = "tok-backup";

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 备份名在用例之间必须唯一：备份根是**数据根的兄弟**（`api_backup` 里那条
/// 规则），而测试的数据根是 `%TEMP%/quill-server-test-*`，于是备份根落在
/// `%TEMP%/backups` —— 一个共享目录。同名会互相踩。
fn name(tag: &str) -> String {
    format!(
        "{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// 用完即删的备份目录守卫。
struct Bk(std::path::PathBuf);

impl Drop for Bk {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 与 `extensions_http.rs` 同构：一例一份真实临时库，落盘路径都在临时目录里。
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
        Self { db, _dir: dir }
    }

    fn config(&self) -> Config {
        let mut cfg = Config::from_env();
        cfg.db_path = std::path::PathBuf::from(self.db.path());
        cfg
    }

    fn state(&self) -> AppState {
        let resolver = EnvTokenResolver::new(vec![(
            TOKEN.to_string(),
            AuthContext {
                user_id: quill_domain::UserId::parse(UID).expect("测试 UID 必须合法"),
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

    /// 数据根 = 库文件所在目录；备份根 = 数据根的**兄弟** `backups`
    /// （与 `api_backup::backup_root` 同一处推导，见那里的注释）。
    fn data_root(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(self.db.path())
            .parent()
            .expect("临时库必有父目录")
            .to_path_buf()
    }

    fn backup_root(&self) -> std::path::PathBuf {
        self.data_root()
            .parent()
            .expect("临时目录必有上级")
            .join("backups")
    }

    /// 备份目录 + 用完即删的守卫。
    fn backup(&self, name: &str) -> (std::path::PathBuf, Bk) {
        let dir = self.backup_root().join(name);
        (dir.clone(), Bk(dir))
    }

    fn write_data(&self, rel: &str, content: &[u8]) -> std::path::PathBuf {
        let p = self.data_root().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("建数据子目录");
        }
        std::fs::write(&p, content).expect("写用户数据");
        p
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

fn post(name: &str) -> Request<Body> {
    post_path("/api/backup/export", name)
}

fn post_path(path: &str, name: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(format!("{{\"name\":{name:?}}}")))
        .expect("构造请求")
}

fn json_of(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("响应体不是 JSON（{e}）：{text}"))
}

fn sha256_of(path: &std::path::Path) -> String {
    quill_backup::sha256_file(path).expect("重算摘要")
}

fn manifest_file_count(manifest_text: &str) -> usize {
    manifest_text
        .lines()
        .find_map(|l| l.strip_prefix("file.count="))
        .and_then(|v| v.trim().parse().ok())
        .expect("清单里应有 file.count")
}

/// 清单里记的相对路径集合。
fn manifest_rels(manifest_text: &str) -> Vec<String> {
    manifest_text
        .lines()
        .filter_map(|l| l.strip_prefix("file\t"))
        .filter_map(|rest| rest.split('\t').nth(2))
        .map(|s| s.to_string())
        .collect()
}

/// 目录快照：相对路径 → 内容。用来证明「备份根之外一个字节都没动」。
fn snapshot(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(b) = std::fs::read(&p) {
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, b));
            }
        }
    }
    out.sort();
    out
}

async fn export_ok(h: &Harness, name: &str) -> String {
    let resp = build_router(h.state())
        .oneshot(post(name))
        .await
        .expect("导出请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert_eq!(status, StatusCode::CREATED, "导出应返回 201：{text}");
    text
}

/// 1. 导出真的落盘：清单与数据库快照都存在，摘要与磁盘重算一致。
#[tokio::test]
async fn export_writes_a_real_backup_and_the_digest_matches_the_file_on_disk() {
    let h = Harness::new("export-roundtrip");
    h.write_data("u1/wiki/notes.md", "第一份笔记".as_bytes());
    h.write_data("u1/session.json", b"{\"a\":1}");

    let name = name("first");
    let (dir, _bk) = h.backup(&name);
    let text = export_ok(&h, &name).await;
    let body = json_of(&text);

    let manifest_path = dir.join("MANIFEST");
    let db_snapshot = dir.join("db").join("quill.db");
    assert!(
        manifest_path.is_file(),
        "清单必须真落盘（不只是响应里说有）"
    );
    assert!(
        db_snapshot.is_file(),
        "数据库快照必须真落盘：{}",
        db_snapshot.display()
    );
    assert!(
        dir.join("data").join("u1/wiki/notes.md").is_file(),
        "用户数据必须被复制进备份目录"
    );

    let manifest_text = std::fs::read_to_string(&manifest_path).expect("读清单");
    assert_eq!(
        body["db_sha256"].as_str(),
        Some(sha256_of(&db_snapshot).as_str()),
        "响应里的 db_sha256 必须等于磁盘上那份快照重算的摘要"
    );
    assert!(
        manifest_text.contains(body["db_sha256"].as_str().expect("db_sha256 缺失")),
        "清单里记的也必须是同一个摘要"
    );
    assert_eq!(
        body["files"].as_u64(),
        Some(manifest_file_count(&manifest_text) as u64),
        "响应文件数必须与清单一致：{text}"
    );
    let rels = manifest_rels(&manifest_text);
    for want in ["u1/wiki/notes.md", "u1/session.json"] {
        assert!(
            rels.iter().any(|r| r == want),
            "用户数据必须进备份：清单里没有 {want}（{rels:?}）"
        );
    }
    assert!(body["excluded"].is_array(), "excluded 必须是数组：{text}");
    assert!(
        body["total_bytes"].as_u64().expect("total_bytes 缺失") > 0,
        "总字节数必须为正：{text}"
    );
}

/// 1b. 备份根必须在数据根**之外**。放里面的话 `create_backup` 会把「正在写的
/// 那个目录」也复制进去，路径越滚越长 —— 实测递归到 `File name too long`
/// 并把存储线程的栈打爆，导出返 500。这条用例钉住的就是那个坑。
#[tokio::test]
async fn a_second_export_does_not_swallow_the_first_one() {
    let h = Harness::new("no-self-nesting");
    h.write_data("u1/notes.md", "内容".as_bytes());

    let first = name("nest-a");
    let (_d1, _bk1) = h.backup(&first);
    export_ok(&h, &first).await;

    let second = name("nest-b");
    let (dir2, _bk2) = h.backup(&second);
    export_ok(&h, &second).await;

    assert!(
        !dir2.join("data").join("backups").exists(),
        "第二份备份里出现了第一份备份：备份根被放进了被复制的源目录（{}）",
        dir2.join("data").join("backups").display()
    );
    let manifest = std::fs::read_to_string(dir2.join("MANIFEST")).expect("读第二份清单");
    let rels = manifest_rels(&manifest);
    assert!(
        rels.iter().any(|r| r == "u1/notes.md"),
        "第二份备份必须含用户数据：{rels:?}"
    );
    assert!(
        !rels.iter().any(|r| r.starts_with("backups/")),
        "第二份清单里混进了第一份备份（{rels:?}）：备份根被放进了被复制的源目录"
    );
}

/// 1c. 活着的数据库文件不进 `data/` 副本。
///
/// 默认配置下数据库就在数据根里，所以「复制用户数据」会顺手拷一份正在被
/// 服务写着的 `quill.db`（以及 `-wal`）。那份是撕裂的中间态，清单还会给它
/// 记一个「摘要正确」的条目 —— 校验反而说它没问题。权威快照在 `db/quill.db`。
#[tokio::test]
async fn the_live_database_is_not_copied_into_the_user_data_part() {
    let h = Harness::new("no-live-db-copy");
    h.write_data("u1/notes.md", "内容".as_bytes());

    let name = name("livedb");
    let (dir, _bk) = h.backup(&name);
    let text = export_ok(&h, &name).await;

    let live_db_name = std::path::Path::new(&h.db.path())
        .file_name()
        .expect("库文件必有文件名")
        .to_string_lossy()
        .to_string();

    let db_copy = dir.join("data").join(&live_db_name);
    assert!(
        !db_copy.exists(),
        "用户数据目录里出现了活库副本 {}：那是一份边写边拷出来的撕裂文件",
        db_copy.display()
    );
    for sidecar in ["-wal", "-shm"] {
        let p = dir.join("data").join(format!("{live_db_name}{sidecar}"));
        assert!(
            !p.exists(),
            "用户数据目录里出现了 WAL 边车 {}：还原时会盖在刚换上的数据库旁边",
            p.display()
        );
    }

    // 权威快照仍然在，而且响应必须**说出来**它排除了什么，不能悄悄少拷。
    assert!(
        dir.join("db").join(&live_db_name).is_file(),
        "权威快照必须仍在 db/ 目录里：{}",
        dir.display()
    );
    let excluded = json_of(&text)["excluded"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        excluded.iter().any(|e| e["rel"]
            .as_str()
            .unwrap_or_default()
            .contains(&live_db_name)),
        "响应必须点名被排除的活库文件，用户才看得见备份里没有什么：{text}"
    );
}

/// 2. 校验接受导出刚做完的那份。
#[tokio::test]
async fn verify_accepts_what_export_just_produced() {
    let h = Harness::new("verify-ok");
    h.write_data("u1/notes.md", "笔记内容".as_bytes());

    let name = name("good");
    let (dir, _bk) = h.backup(&name);
    let text = export_ok(&h, &name).await;
    assert!(json_of(&text)["db_sha256"].as_str().is_some(), "{text}");

    let resp = build_router(h.state())
        .oneshot(post_path("/api/backup/verify", &name))
        .await
        .expect("校验请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert_eq!(status, StatusCode::OK, "校验通过应返回 200：{text}");
    let body = json_of(&text);
    assert_eq!(body["ok"], serde_json::json!(true), "{text}");
    let manifest = std::fs::read_to_string(dir.join("MANIFEST")).expect("读清单");
    assert_eq!(
        body["files"].as_u64(),
        Some(manifest_file_count(&manifest) as u64),
        "文件数必须与清单一致：{text}"
    );
    assert_eq!(
        body["db_sha256"].as_str(),
        Some(sha256_of(&dir.join("db").join("quill.db")).as_str()),
        "校验返回的摘要必须是重算出来的，不是抄清单的"
    );
}

/// 3. 校验必须抓到损坏 —— 否则它只是个装饰。
#[tokio::test]
async fn verify_fails_and_names_the_file_when_the_backup_is_tampered_with() {
    let h = Harness::new("verify-tamper");
    h.write_data("u1/notes.md", "原始内容".as_bytes());
    let name = name("tampered");
    let (dir, _bk) = h.backup(&name);
    export_ok(&h, &name).await;

    let victim = dir.join("data").join("u1/notes.md");
    let original = std::fs::read(&victim).expect("读被改文件");
    let mut broken = original.clone();
    broken[0] ^= 0xff;
    std::fs::write(&victim, &broken).expect("改一个字节");

    let resp = build_router(h.state())
        .oneshot(post_path("/api/backup/verify", &name))
        .await
        .expect("校验请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert!(
        !status.is_success(),
        "内容被改过却报成功：那是假成功：{status} {text}"
    );
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
    assert!(text.contains("u1/notes.md"), "必须点名损坏的文件：{text}");
    assert!(text.contains("校验未通过"), "必须说清是校验没过：{text}");
    assert!(
        !text.contains("\"ok\":true"),
        "失败响应里不许出现 ok:true：{text}"
    );
    assert!(text.contains("next_step"), "失败必须自带下一步：{text}");
}

/// 3b. 截断也算损坏（摘要与内容都要对得上才叫完整）。
#[tokio::test]
async fn verify_fails_when_a_file_is_truncated() {
    let h = Harness::new("verify-truncate");
    h.write_data("u1/notes.md", "一段足够长的内容用于截断".as_bytes());
    let name = name("cut");
    let (dir, _bk) = h.backup(&name);
    export_ok(&h, &name).await;

    let victim = dir.join("data").join("u1/notes.md");
    let all = std::fs::read(&victim).expect("读被截断文件");
    std::fs::write(&victim, &all[..all.len() / 2]).expect("截断");

    let resp = build_router(h.state())
        .oneshot(post_path("/api/backup/verify", &name))
        .await
        .expect("校验请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
    assert!(text.contains("u1/notes.md"), "必须点名被截断的文件：{text}");
}

/// 3c. 清单本身坏掉 → 422，而且**不能**说「换一个没用过的备份名」。
///
/// 那句改名建议对摘要不符还勉强算个方向，对「清单读不出来」则是彻底无关：
/// 重新导出一份新的，或者换一个**别的**备份，才是对的。
#[tokio::test]
async fn verify_reports_an_unreadable_manifest_without_pretending_a_rename_would_help() {
    let h = Harness::new("verify-badmanifest");
    h.write_data("u1/notes.md", "内容".as_bytes());
    let name = name("badman");
    let (dir, _bk) = h.backup(&name);
    export_ok(&h, &name).await;

    std::fs::write(dir.join("MANIFEST"), "{ this is not the manifest").expect("把清单写成坏格式");

    let resp = build_router(h.state())
        .oneshot(post_path("/api/backup/verify", &name))
        .await
        .expect("校验请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
    assert!(
        text.contains("清单无法解析"),
        "必须说清是清单坏了，不是别的原因：{text}"
    );
    let envelope = json_of(&text);
    // 走对路径：next_step 藏在 error 信封里。读错路径会拿到空串，
    // 「空串不含某句话」恒真 —— 一条永远绿的断言等于没写。
    let ns = envelope["error"]["next_step"].as_str().unwrap_or_default();
    assert!(!ns.is_empty(), "422 必须自带下一步：{text}");
    assert!(
        !ns.contains("换一个没用过"),
        "422 不许给出「换个备份名」这种无效建议：{text}"
    );
    assert!(
        !text.contains("\"ok\":true"),
        "失败响应里不许出现 ok:true：{text}"
    );
}

/// 4. 越界名一律 4xx，且备份根之外没有写出任何东西。
#[tokio::test]
async fn traversal_and_absolute_names_are_refused_and_write_nothing() {
    let h = Harness::new("traversal");
    let before = snapshot(&h.data_root());

    for bad in [
        "../escape",
        "..\\escape",
        "C:\\Windows\\x",
        "/etc/passwd",
        "",
    ] {
        let resp = build_router(h.state())
            .oneshot(post(bad))
            .await
            .expect("导出请求失败");
        let status = resp.status();
        let text = body_text(resp).await;
        assert!(
            status.is_client_error(),
            "越界名 {bad:?} 必须 4xx，实际 {status}：{text}"
        );
        assert!(
            !status.is_success(),
            "越界名 {bad:?} 绝不能成功：{status} {text}"
        );
        assert!(
            text.contains("next_step"),
            "拒绝必须自带下一步（{bad:?}）：{text}"
        );
    }

    // 逐个点名「这个路径必须不存在」，而不是比对整个备份根 ——
    // 备份根是共享的（见 `name` 处的说明），别的用例正在往里写。
    for must_not_exist in [
        h.backup_root().join("..\\escape"),
        h.backup_root().join("C:\\Windows\\x"),
        h.data_root()
            .parent()
            .expect("临时目录必有上级")
            .join("escape"),
    ] {
        assert!(
            !must_not_exist.exists(),
            "被拒的名字不许在磁盘上留下目录：{}",
            must_not_exist.display()
        );
    }
    assert_eq!(
        snapshot(&h.data_root()),
        before,
        "被拒绝的请求不许改动数据目录里的任何字节"
    );

    // 对照组：同一个备份根、同一条路由，一个正常名字**必须**写得进去 ——
    // 否则上面那几条「不存在」可能只是因为根目录根本没被用过（断言会空转）。
    let good = name("probe");
    let (dir, _bk) = h.backup(&good);
    export_ok(&h, &good).await;
    assert!(
        dir.join("MANIFEST").is_file(),
        "正常名字必须真能导出：{}",
        dir.display()
    );
}

/// 5. 目标非空 → 409，且消息点名那个目录。
#[tokio::test]
async fn a_non_empty_destination_is_refused_with_the_directory_named() {
    let h = Harness::new("dest-not-empty");
    h.write_data("u1/notes.md", "内容".as_bytes());

    let name = name("twice");
    let (dest, _bk) = h.backup(&name);
    export_ok(&h, &name).await;
    let snapshot_before = snapshot(&dest);

    let resp = build_router(h.state())
        .oneshot(post(&name))
        .await
        .expect("二次导出请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert_eq!(status, StatusCode::CONFLICT, "目标非空必须 409：{text}");
    // 比对**解析后的** detail，不比对原始正文：JSON 会把 Windows 路径里的每个
    // 反斜杠转义成两个，拿未转义的路径去 contains 原始正文，在 Windows 上
    // 必然匹配不上而在 Linux 上（路径用 `/`）恰好通过 —— 一条只在 Linux 上
    // 绿的断言，测不出它想测的东西。
    let detail = json_of(&text)["error"]["detail"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        detail.contains(&dest.display().to_string()),
        "409 必须点名目标目录：{text}"
    );
    assert!(detail.contains("非空"), "必须说清为什么拒绝：{text}");
    assert_eq!(
        snapshot(&dest),
        snapshot_before,
        "被拒的导出不许动已有那份备份"
    );
}

/// 6. 还原必须诚实：说清服务持有数据库、给出真实命令、不碰任何文件。
#[tokio::test]
async fn restore_is_honest_about_not_restoring_and_touches_nothing() {
    let h = Harness::new("restore-honest");
    h.write_data("u1/notes.md", "原始内容".as_bytes());
    let name = name("for-restore");
    let (_dir, _bk) = h.backup(&name);
    export_ok(&h, &name).await;

    // 请求前后：数据目录与库文件必须逐字节不变。
    let data_before = snapshot(&h.data_root());
    let db_path = h.data_root().join("quill.db");
    let db_bytes_before = std::fs::read(&db_path).expect("读库文件");

    let resp = build_router(h.state())
        .oneshot(post_path("/api/backup/restore", &name))
        .await
        .expect("还原请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert!(
        !status.is_server_error(),
        "还原不该是服务端错误（更不该是 501）：{status} {text}"
    );
    let body = json_of(&text);
    assert_eq!(body["restored"], serde_json::json!(false), "{text}");
    assert!(
        !text.contains("\"restored\":true"),
        "绝不能声称还原过：{text}"
    );
    assert!(
        text.contains("quill restore") && text.contains("--yes"),
        "必须给出真实的 CLI 命令：{text}"
    );
    assert!(
        text.contains("持有") || text.contains("占用"),
        "必须说清服务正持有数据库：{text}"
    );
    assert!(
        text.contains("停掉") || text.contains("停止"),
        "必须说清要先停服务：{text}"
    );

    assert_eq!(
        std::fs::read(&db_path).expect("再读库文件"),
        db_bytes_before,
        "库文件一个字节都不能被改"
    );
    assert_eq!(
        snapshot(&h.data_root()),
        data_before,
        "还原请求不许改动备份路径之外的任何东西"
    );
}

/// 6b. 名字指向的备份不存在时，要说的就是「没有这份备份」，不能编一条命令。
#[tokio::test]
async fn restore_on_a_missing_backup_says_it_is_missing() {
    let h = Harness::new("restore-missing");
    let missing = name("never-made");
    let resp = build_router(h.state())
        .oneshot(post_path("/api/backup/restore", &missing))
        .await
        .expect("还原请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    assert!(text.contains(&missing), "必须点名缺的是哪一份：{text}");
    assert!(
        !text.contains("quill restore"),
        "没有可还原的东西时不该给命令（那是照着跑会失败的建议）：{text}"
    );
}

/// 7. 校验不认识的备份目录 → 404（不是 501，也不是 ok:true）。
#[tokio::test]
async fn verify_on_a_missing_backup_is_404_not_501() {
    let h = Harness::new("verify-missing");
    let missing = name("nothing-here");
    let resp = build_router(h.state())
        .oneshot(post_path("/api/backup/verify", &missing))
        .await
        .expect("校验请求失败");
    let status = resp.status();
    let text = body_text(resp).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    assert!(!text.contains("\"ok\""), "404 里不该出现 ok 字段：{text}");
}
