//! 资料库页面**手写增删改**的 HTTP 端到端证据（queue Q058）。
//!
//! 这一批路由对应前端 `ui/web/src/memory/api.ts` 里那条先定下来的契约
//! `PUT /api/wiki/pages/{path}`（`WIKI_WRITE_ROUTE`）。判据不是「返回 200」，
//! 而是四条：
//!   - 新建 / 覆盖 / 删除各自**真落盘**（读回来一致），并把 `index.md` 重建、往变更日志追加；
//!   - 乐观并发**真的挡住**了过期写入（409，且**文件没被动过**）；
//!   - 写不进去的内容（解析不了）在**动盘之前**就被 400 挡下；
//!   - 越出资料库的路径进不来。

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

/// 一个合法页面：frontmatter 齐全、正文一句话。
const PAGE: &str = "---\ntitle: 入门\ntype: concept\ncreated: 2026-10-04\nupdated: 2026-10-04\n---\n\n这是入门页。\n";

fn user_id() -> quill_domain::UserId {
    quill_domain::UserId::parse(UID_A).expect("测试 UID 必须合法")
}

/// 临时资料库根（每个用例一个），用完不删也不影响别人 —— 名字里带进程与线程 id。
fn wiki_root(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "quill-wiki-http-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("建临时资料库根");
    p
}

fn app(t: &TestDb, wiki: std::path::PathBuf) -> AppState {
    let resolver = EnvTokenResolver::new(vec![(
        TOKEN_A.to_string(),
        AuthContext {
            user_id: user_id(),
            is_admin: true,
        },
    )]);
    let mut config = Config::from_env();
    config.wiki_dir = wiki;
    AppState {
        config,
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
    serde_json::from_str(t)
        .unwrap_or_else(|e| panic!("响应必须是 JSON（{e}）：{}", t.replace('\n', "\\n")))
}

/// 写一页（`expected_version` 由调用方给），返回 `(状态码, 响应体)`。
async fn put(
    app: AppState,
    path: &str,
    content: &str,
    expected_version: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let resp = build_router(app)
        .oneshot(req(
            "PUT",
            path,
            Some(serde_json::json!({
                "content": content,
                "expected_version": expected_version,
            })),
        ))
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    (status, json(&text(resp).await))
}

/// 读一页，返回 `(状态码, 响应体)`。
async fn get(app: AppState, path: &str) -> (StatusCode, serde_json::Value) {
    let resp = build_router(app)
        .oneshot(req("GET", path, None))
        .await
        .expect("oneshot 失败");
    let status = resp.status();
    (status, json(&text(resp).await))
}

const PAGE_URL: &str = "/api/wiki/pages/concepts/%E5%85%A5%E9%97%A8.md";

#[tokio::test]
async fn creating_a_page_writes_it_rebuilds_the_index_and_logs_it() {
    let t = TestDb::new("wiki-create");
    let root = wiki_root("create");
    let app = app(&t, root);

    let (status, body) = put(app.clone(), PAGE_URL, PAGE, None).await;
    assert_eq!(status, StatusCode::CREATED, "新建应当 201：{body}");
    assert_eq!(body["created"], true);
    assert_eq!(body["path"], "concepts/入门.md");
    let version = body["version"]
        .as_str()
        .expect("必须回版本号，否则前端下次没法带回来")
        .to_string();
    assert_eq!(version.len(), 64, "版本号是 sha256 的十六进制：{version}");

    // 真落盘：读回来内容一致，且版本号与写的时候一样。
    let (status, got) = get(app.clone(), PAGE_URL).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["content"], PAGE);
    assert_eq!(got["version"], version.as_str());

    // 索引重建了，而且里面有这一页。
    let (status, idx) = get(app.clone(), "/api/wiki/index").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(idx["present"], true, "写完必须有索引：{idx}");
    assert!(
        idx["index"].as_str().unwrap_or_default().contains("入门"),
        "索引里必须出现这一页的标题：{idx}"
    );

    // 变更日志追加了一条，且说得清是谁写的。
    let (_, log) = get(app, "/api/wiki/log").await;
    let log = log["log"].as_str().unwrap_or_default();
    assert!(
        log.contains("concepts/入门.md"),
        "日志必须点名这一页：{log}"
    );
    assert!(log.contains("新建"), "日志必须说清是新建：{log}");
}

#[tokio::test]
async fn a_stale_version_is_refused_and_the_file_is_left_untouched() {
    let t = TestDb::new("wiki-cas");
    let root = wiki_root("cas");
    let app = app(&t, root);

    let (_, created) = put(app.clone(), PAGE_URL, PAGE, None).await;
    let v1 = created["version"].as_str().expect("v1").to_string();

    // 带对的版本号覆盖 → 200，版本号变了。
    let second = PAGE.replace("这是入门页。", "这是入门页的第二版。");
    let (status, updated) = put(app.clone(), PAGE_URL, &second, Some(&v1)).await;
    assert_eq!(status, StatusCode::OK, "带对版本号应当 200：{updated}");
    assert_eq!(updated["created"], false);
    let v2 = updated["version"].as_str().expect("v2").to_string();
    assert_ne!(v1, v2, "内容变了，版本号必须跟着变");

    // 拿**过期**的 v1 再覆盖 → 409，而且文件一个字都没变。
    // （内容必须是合法页面：校验在并发判定之前，这是有意的 —— 一份根本写不进去的
    //   内容报 409 没有意义。）
    let stale = PAGE.replace("这是入门页。", "这是有人抢在我前面写的第三版。");
    let (status, conflict) = put(app.clone(), PAGE_URL, &stale, Some(&v1)).await;
    assert_eq!(status, StatusCode::CONFLICT, "过期版本必须 409：{conflict}");
    assert!(
        conflict["error"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains(&v2),
        "409 必须把**当前**版本号回给用户，否则他没法重试：{conflict}"
    );
    let (_, got) = get(app, PAGE_URL).await;
    assert_eq!(got["content"], second, "被拒的那次不许动盘");
    assert_eq!(got["version"], v2.as_str());
}

#[tokio::test]
async fn creating_a_page_that_already_exists_conflicts() {
    let t = TestDb::new("wiki-exists");
    let root = wiki_root("exists");
    let app = app(&t, root);

    put(app.clone(), PAGE_URL, PAGE, None).await;
    // 第二次仍说「应当还不存在」→ 409（否则并发两个「新建」会互相覆盖）。
    let (status, body) = put(app.clone(), PAGE_URL, PAGE, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "重复新建必须 409：{body}");
    // 拿一个不存在的版本号去改也应当 409，而不是「那就新建一份」。
    let (status, body) = put(app.clone(), PAGE_URL, PAGE, Some("deadbeef")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn content_that_is_not_a_page_is_refused_before_anything_is_written() {
    let t = TestDb::new("wiki-bad-content");
    let root = wiki_root("bad-content");
    let app = app(&t, root);

    // 没有 frontmatter 的内容解析不成页面。
    let (status, body) = put(app.clone(), PAGE_URL, "# 只有正文\n", None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "写不进去的内容要 400：{body}"
    );
    // 一个字都不许落盘。
    let (status, _) = get(app.clone(), PAGE_URL).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "被拒的内容不许留下半页");
    // 索引也不该因为一次失败而出现。
    let (_, idx) = get(app, "/api/wiki/index").await;
    assert_eq!(idx["present"], false, "没写成功就不该有索引：{idx}");
}

#[tokio::test]
async fn deleting_needs_the_version_and_a_stale_one_conflicts() {
    let t = TestDb::new("wiki-delete");
    let root = wiki_root("delete");
    let app = app(&t, root);

    let (_, created) = put(app.clone(), PAGE_URL, PAGE, None).await;
    let version = created["version"].as_str().expect("version").to_string();

    // 不带 expected_version → 400（不是「那就直接删」）。
    let resp = build_router(app.clone())
        .oneshot(req("DELETE", PAGE_URL, None))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "删除必须要求版本号");

    // 带错版本号 → 409，页面还在。
    let resp = build_router(app.clone())
        .oneshot(req(
            "DELETE",
            &format!("{PAGE_URL}?expected_version=deadbeef"),
            None,
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert_eq!(get(app.clone(), PAGE_URL).await.0, StatusCode::OK);

    // 带对版本号 → 200，页面没了，索引重建（不再含这一页）。
    let resp = build_router(app.clone())
        .oneshot(req(
            "DELETE",
            &format!("{PAGE_URL}?expected_version={version}"),
            None,
        ))
        .await
        .expect("oneshot 失败");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json(&text(resp).await);
    assert_eq!(body["deleted"], true);
    assert_eq!(get(app.clone(), PAGE_URL).await.0, StatusCode::NOT_FOUND);
    let (_, idx) = get(app, "/api/wiki/index").await;
    assert!(
        !idx["index"].as_str().unwrap_or_default().contains("入门"),
        "删完索引里不该还留着它：{idx}"
    );
}

#[tokio::test]
async fn a_path_that_escapes_the_library_is_refused() {
    let t = TestDb::new("wiki-escape");
    let root = wiki_root("escape");
    let app = app(&t, root.clone());

    // 试图用 `..` 越出资料库根。
    let (status, _) = put(
        app.clone(),
        "/api/wiki/pages/concepts/%2E%2E/%2E%2E/escape.md",
        PAGE,
        None,
    )
    .await;
    assert!(!status.is_success(), "越界路径必须被拒，实际 {status}");
    // 越界那份也没被写出来。
    assert!(
        !root.join("escape.md").exists(),
        "越界写入不该落到资料库根之外"
    );
}
