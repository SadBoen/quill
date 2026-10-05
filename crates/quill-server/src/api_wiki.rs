use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use quill_wiki::page::render_page;
use quill_wiki::WikiStore;

use crate::auth::AuthUser;
use crate::error::ApiError;
use crate::state::AppState;

fn store_for(state: &AppState, user: &AuthUser) -> WikiStore {
    WikiStore::new(state.config.wiki_dir.clone(), user.0.user_id)
}

fn is_missing(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

fn map_wiki(e: quill_wiki::WikiError) -> ApiError {
    match e {
        quill_wiki::WikiError::PathEscape { attempt, .. } => ApiError::bad_request(format!(
            "页面路径 {attempt:?} 试图越出资料库根目录，已拒绝。\
             下一步：用资料库内的相对路径，例如 `concepts/入门.md`。"
        )),
        quill_wiki::WikiError::NotFound { .. } => {
            ApiError::entity_not_found("资料库里没有这一页".to_string())
        }
        quill_wiki::WikiError::NotUtf8 { .. } => {
            ApiError::bad_request("页面不是 UTF-8 文本，无法读入".to_string())
        }
        quill_wiki::WikiError::Io { path, source } if is_missing(&source) => {
            ApiError::entity_not_found(format!(
                "资料库里没有 {}（路径 {}）。",
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "这一页".to_string()),
                path.display()
            ))
        }
        quill_wiki::WikiError::Io { path, .. } => ApiError::internal(format!(
            "资料库文件操作失败（{}）。下一步：确认该路径可写，\
             或用 `quill doctor` 查看数据根目录诊断。",
            path.display()
        )),
        other => ApiError::internal(format!("资料库操作失败：{other}")),
    }
}

pub async fn list_pages(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, ApiError> {
    let store = store_for(&state, &user);
    let pages = store.list_pages().map_err(map_wiki)?;
    Ok(Json(json!({ "pages": pages })))
}

pub async fn get_page(
    State(state): State<AppState>,
    user: AuthUser,
    Path(path): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let store = store_for(&state, &user);
    let page = store.read_page(&path).map_err(map_wiki)?;
    let mut body = Json(json!({
        "path": page.path,
        "content": render_page(&page),
    }));
    if let Some(t) = &page.frontmatter.title {
        body.0["title"] = json!(t);
    }
    if let Some(t) = page.page_type() {
        body.0["page_type"] = json!(format!("{t:?}").to_lowercase());
    }
    if !page.warnings.is_empty() {
        body.0["warnings"] = json!(page
            .warnings
            .iter()
            .map(|w| w.to_string())
            .collect::<Vec<String>>());
    }
    Ok(body)
}

pub async fn read_index(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, ApiError> {
    let store = store_for(&state, &user);
    let index = store.read_index().map_err(map_wiki)?;
    Ok(Json(json!({
        "index": index,
        "present": index.is_some(),
    })))
}

pub async fn read_log(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, ApiError> {
    let store = store_for(&state, &user);
    let log = store.read_log().map_err(map_wiki)?;
    Ok(Json(json!({
        "log": log,
        "present": log.is_some(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_wiki::UserId;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "quill-wiki-api-{label}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)
                .unwrap_or_else(|e| panic!("创建临时目录 {} 失败：{e}", dir.display()));
            Self(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const UID_A: &str = "0192b7c8-0000-7000-8000-0000000000aa";
    const UID_B: &str = "0192b7c8-0000-7000-8000-0000000000bb";

    fn store_for(root: &TempRoot, uid: &str) -> WikiStore {
        let store = WikiStore::new(root.path(), UserId::parse(uid).expect("测试 UID 必须合法"));
        store.ensure_layout().expect("建目录");
        store
    }

    #[test]
    fn an_absent_page_is_a_404_not_an_empty_success() {
        let root = TempRoot::new("absent");
        let store = store_for(&root, UID_A);
        let err = store.read_page("nope.md").expect_err("不存在的页面必须报错");
        assert!(
            matches!(err, quill_wiki::WikiError::NotFound { .. }),
            "实际 {err:?}"
        );
    }

    #[test]
    fn page_not_found_maps_to_entity_not_found_rather_than_500() {
        let root = TempRoot::new("notfound");
        let store = store_for(&root, UID_A);
        let err = map_wiki(store.read_page("nope.md").expect_err("缺页"));
        assert_eq!(err.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(err.code(), "entity_not_found");
    }

    #[test]
    fn a_path_escape_is_a_400_that_names_the_offending_path() {
        let root = TempRoot::new("escape");
        let store = store_for(&root, UID_A);
        let err = map_wiki(store.read_page("../../etc/passwd").expect_err("越界必须被拒"));
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
        let d = err.detail();
        assert!(d.contains("passwd"), "要点名越界路径：{d}");
        assert!(d.contains("concepts/入门.md"), "要给出正例：{d}");
    }

    #[test]
    fn two_users_never_see_each_others_pages() {
        let root = TempRoot::new("isolation");
        let a = store_for(&root, UID_A);
        let b = store_for(&root, UID_B);

        a.write_page("mine.md", "---\ntitle: 甲\n---\n\n甲的页\n")
            .expect("写页");

        assert_eq!(a.list_pages().expect("列页"), vec!["mine.md".to_string()]);
        assert!(
            b.list_pages().expect("列页").is_empty(),
            "换个用户必须看不到别人的页面"
        );
        assert!(
            b.read_page("mine.md").is_err(),
            "换个用户必须读不到别人的页面"
        );
    }

    #[test]
    fn index_and_log_report_absence_instead_of_inventing_content() {
        let root = TempRoot::new("empty");
        let store = store_for(&root, UID_A);
        assert_eq!(store.read_index().expect("读索引"), None);
        assert_eq!(store.read_log().expect("读日志"), None);
    }

    #[test]
    fn a_written_page_round_trips_through_the_wire_shape_the_api_returns() {
        let root = TempRoot::new("wire");
        let store = store_for(&root, UID_A);
        store
            .write_page(
                "concepts/入门.md",
                "---\ntitle: 入门\ntype: concept\n---\n\n正文。\n",
            )
            .expect("写页");

        let page = store.read_page("concepts/入门.md").expect("读页");
        assert_eq!(page.path, "concepts/入门.md");
        assert!(
            render_page(&page).contains("正文"),
            "API 返回的正文必须来自 render_page"
        );
        assert_eq!(page.frontmatter.title.as_deref(), Some("入门"));
    }
}
