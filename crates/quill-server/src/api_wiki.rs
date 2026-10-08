use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use quill_wiki::page::render_page;
use quill_wiki::{IndexEntry, WikiIndex, WikiStore};

use crate::auth::AuthUser;
use crate::body::JsonBody;
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

pub async fn list_pages(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
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

pub async fn read_index(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let store = store_for(&state, &user);
    let index = store.read_index().map_err(map_wiki)?;
    Ok(Json(json!({
        "index": index,
        "present": index.is_some(),
    })))
}

pub async fn read_log(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let store = store_for(&state, &user);
    let log = store.read_log().map_err(map_wiki)?;
    Ok(Json(json!({
        "log": log,
        "present": log.is_some(),
    })))
}

// ---------------------------------------------------------------- 索引检索

/// 搜索一次最多返回多少条。超过就报 400，**不悄悄截断**：
/// 悄悄截断会让「共 N 条命中」这类数字和人看到的不一致。
pub const MAX_SEARCH_LIMIT: usize = 50;

/// 不带 `limit` 时的默认条数。
pub const DEFAULT_SEARCH_LIMIT: usize = 10;

/// 摘录在命中处前后各留多少个字符。
const EXCERPT_PAD: usize = 24;

/// `POST /api/wiki/search` —— 在 **index.md** 上做文本匹配。
///
/// ## 只读索引（这一条的全部约束）
///
/// **不读页面正文、不调模型、不写任何文件**（连 log 都不追加）：数据源就是
/// `store.read_index()` 拿回来的那份 `index.md`。索引里每条能检索的只有
/// [`IndexEntry`] 的两列 —— **标题与摘要**
/// （`crates/quill-wiki/src/index.rs:9-20`；条目由 `build_index` 从页面的
/// `title` 与正文首段摘要生成，见 `crates/quill-wiki/src/ingest.rs:129` 与
/// `crates/quill-wiki/src/index.rs:23`）。
///
/// **索引里没有的，这里也查不到**，所以响应里也不编：
///
/// - `index.md` 的一行只有 `- [[标题]] — 摘要（source_count=N, updated=…）`，
///   **没有页面路径**（`render_entry_line`，`crates/quill-wiki/src/index.rs:196`），
///   所以命中里不给 `path` —— 给不了；
/// - 解析出的 `page_type` **恒为 `Summary`**（`parse_entry_line` 的硬编码，
///   `crates/quill-wiki/src/index.rs:250`），所以也不给 `page_type` —— 给出来是假话。
///
/// 想按正文 / 标签 / 链接检索，得另补索引列或另走图结构（`LinkGraph`，
/// 它需要把全部页面读进内存，不是「只读索引」）——那是另一条条目的事。
///
/// ## 返回结构
///
/// 每条命中都带**依据**：`matched_fields` 说命中在哪一列，`matches` 给出
/// （列，词，摘录）三元组；顶层给 `total_hits` / `returned` / `truncated`。
/// 计分与排序跟 `WikiIndex::lookup`（`crates/quill-wiki/src/index.rs:159`）
/// 逐条一致（标题 +2 / 摘要 +1，同分按标题升序），有一条测试把两边并排比。
pub async fn search(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    let req = parse_search_request(&body)?;
    let store = store_for(&state, &user);
    Ok(Json(search_store(&store, &req)?))
}

/// 解析并校验请求体。字段名与语义都写在错误里，别让调用方猜。
fn parse_search_request(body: &Value) -> Result<SearchRequest, ApiError> {
    crate::api_experts::only_keys(body, &["query", "limit"], "POST /api/wiki/search")?;
    let obj = body.as_object().expect("only_keys 已确认是对象");

    let query = obj
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::bad_request(
                "query 必填，且不能是空白：它是这次要在索引里找的词。\
                 下一步：{\"query\":\"所有权\"}，可选 {\"limit\":10}（1~50，默认 10）。"
                    .to_string(),
            )
        })?
        .to_string();

    let terms = search_terms(&query);
    if terms.is_empty() {
        // 「切不出词」与「没有命中」是两件事：前者说明这次根本搜不了，
        // 报 0 条命中等于把「没搜」说成「搜过了，没有」。
        return Err(ApiError::bad_request(format!(
            "query「{query}」里没有可检索的字词：索引检索按「非字母数字」切词，\
             切完是空的（例如全是标点或符号）。\
             下一步：换一个含字母、数字或汉字的词，例如「所有权」或 mcp。"
        )));
    }

    let limit = match obj.get("limit") {
        None | Some(Value::Null) => DEFAULT_SEARCH_LIMIT,
        Some(v) => {
            let n = v.as_u64().filter(|n| *n >= 1).ok_or_else(|| {
                ApiError::bad_request(
                    "limit 必须是正整数（1~50）。\
                         下一步：省略这个键用默认的 10，或填 1~50 之间的整数。"
                        .to_string(),
                )
            })? as usize;
            if n > MAX_SEARCH_LIMIT {
                return Err(ApiError::bad_request(format!(
                    "limit={n} 超过上限 {MAX_SEARCH_LIMIT}。\
                     上限是刻意的：再大只会让响应变长，不会多出命中。\
                     下一步：填 1~{MAX_SEARCH_LIMIT}，或收紧 query。"
                )));
            }
            n
        }
    };

    Ok(SearchRequest {
        query,
        terms,
        limit,
    })
}

#[derive(Debug)]
struct SearchRequest {
    query: String,
    terms: Vec<String>,
    limit: usize,
}

/// 读索引并搜索。分开成一步是为了让测试能直接对着真文件调这一条。
fn search_store(store: &WikiStore, req: &SearchRequest) -> Result<Value, ApiError> {
    let index_text = store.read_index().map_err(map_wiki)?;
    let index = match &index_text {
        Some(text) => WikiIndex::parse(text).map_err(map_wiki)?,
        None => WikiIndex::new(),
    };
    Ok(search_parsed(&index, index_text.is_some(), req))
}

/// 纯逻辑：不碰文件系统，便于把边界一次测透。
fn search_parsed(index: &WikiIndex, index_present: bool, req: &SearchRequest) -> Value {
    let mut scored: Vec<(usize, &IndexEntry, Vec<FieldHit>)> = index
        .entries()
        .filter_map(|e| {
            let (score, hits) = score_entry(e, &req.terms);
            (score > 0).then_some((score, e, hits))
        })
        .collect();

    // 与 `WikiIndex::lookup` 同一套排序：分数降序，同分按标题升序（可复现）。
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));

    let total_hits = scored.len();
    let truncated = total_hits > req.limit;
    scored.truncate(req.limit);
    let hits: Vec<Value> = scored
        .iter()
        .map(|(score, e, fields)| hit_json(e, *score, fields))
        .collect();
    let returned = hits.len();

    json!({
        "query": req.query,
        "terms": req.terms,
        // 说清算的是什么 —— 响应里的每个数字都只关于这一份文件。
        "source": "index.md",
        "hits": hits,
        "total_hits": total_hits,
        "returned": returned,
        "truncated": truncated,
        "limit": req.limit,
        "index_present": index_present,
        "index_entries": index.len(),
        "note": search_note(index_present, index.len(), total_hits, returned, req),
    })
}

/// 一条命中里的「哪一列、哪个词、哪一段」。
#[derive(Debug)]
struct FieldHit {
    field: &'static str,
    term: String,
    excerpt: String,
}

/// 一条索引条目的得分与依据。
///
/// 计分**逐条对齐** `WikiIndex::lookup`：标题命中 +2、摘要命中 +1，
/// 每个词各算一次（query 里重复的词就重复算分）。
fn score_entry(e: &IndexEntry, terms: &[String]) -> (usize, Vec<FieldHit>) {
    let title_lc = e.title.to_lowercase();
    let summary_lc = e.summary.to_lowercase();
    let mut score = 0usize;
    let mut hits: Vec<FieldHit> = Vec::new();

    for term in terms {
        if title_lc.contains(term.as_str()) {
            score += 2;
            push_unique(
                &mut hits,
                FieldHit {
                    field: "title",
                    term: term.clone(),
                    // 标题本身就是「命中的那一段」，不需要窗口。
                    excerpt: e.title.clone(),
                },
            );
        }
        if summary_lc.contains(term.as_str()) {
            score += 1;
            push_unique(
                &mut hits,
                FieldHit {
                    field: "summary",
                    term: term.clone(),
                    excerpt: excerpt_of(&e.summary, &summary_lc, term),
                },
            );
        }
    }
    // 标题的证据在前：同一个词的两种命中，先说标题。
    hits.sort_by_key(|h| if h.field == "title" { 0 } else { 1 });
    (score, hits)
}

/// 依据去重（同列同词只留一条），但**分数不去重** —— 分数与 `lookup` 对齐。
fn push_unique(hits: &mut Vec<FieldHit>, hit: FieldHit) {
    if !hits
        .iter()
        .any(|h| h.field == hit.field && h.term == hit.term)
    {
        hits.push(hit);
    }
}

fn hit_json(e: &IndexEntry, score: usize, hits: &[FieldHit]) -> Value {
    let mut fields: Vec<&'static str> = Vec::new();
    for h in hits {
        if !fields.contains(&h.field) {
            fields.push(h.field);
        }
    }
    json!({
        "title": e.title,
        "summary": e.summary,
        "source_count": e.source_count,
        "updated": e.updated.to_string(),
        "score": score,
        "matched_fields": fields,
        "matches": hits
            .iter()
            .map(|h| json!({ "field": h.field, "term": h.term, "excerpt": h.excerpt }))
            .collect::<Vec<Value>>(),
    })
}

/// 命中处前后各留 [`EXCERPT_PAD`] 个字符的窗口，两端超出就加省略号。
fn excerpt(text: &str, hit_char: usize, term_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = hit_char.saturating_sub(EXCERPT_PAD);
    let end = (hit_char + term_chars + EXCERPT_PAD).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(chars[start..end].iter());
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// 在小写化文本上定出命中的位置，再回到原文取摘录。
///
/// `to_lowercase` 对个别字符会改变字符个数（Unicode 特例），对不齐时退回
/// 「整段摘要照给」—— 宁可给长了，也不指一段错的位置。
fn excerpt_of(text: &str, lower: &str, term: &str) -> String {
    let Some(byte) = lower.find(term) else {
        return text.to_string();
    };
    if lower.chars().count() != text.chars().count() {
        return text.to_string();
    }
    let hit = lower[..byte].chars().count();
    excerpt(text, hit, term.chars().count())
}

/// 切词：与 `quill_wiki::index::tokenize`（`crates/quill-wiki/src/index.rs:189`）
/// 同一口径 —— 按非字母数字切分、转小写。**CJK 不分词**：一整串汉字是一个词，
/// 所以「所有权模型的实现」只整体匹配，不会拆成「所有权」+「模型」。
/// 这条限制照实写在返回的 `terms` 里（用户能看见自己的 query 被切成了什么）。
fn search_terms(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|p| !p.is_empty())
        .map(|p| p.to_lowercase())
        .collect()
}

fn search_note(
    index_present: bool,
    index_entries: usize,
    total_hits: usize,
    returned: usize,
    req: &SearchRequest,
) -> String {
    if !index_present {
        return format!(
            "资料库里还没有 index.md（索引），所以这次零命中只说明「没有索引可查」，\
             不代表资料库里没有与「{}」相关的内容。\
             下一步：先摄入一次把索引建起来（POST /api/wiki/ingest），再搜。",
            req.query
        );
    }
    if total_hits == 0 {
        return format!(
            "索引里 {index_entries} 条都没有命中「{}」：这次只在「标题」与「摘要」两列上\
             做词面子串匹配（不读正文、不调模型；CJK 不分词，一整串算一个词）。\
             下一步：换一个更短的词，或确认相关内容已经被摄入进索引。",
            req.query
        );
    }
    if total_hits > returned {
        return format!(
            "索引里共 {total_hits} 条命中，按分数降序（同分按标题升序）只返回前 {returned} 条\
             （limit={}，上限 {MAX_SEARCH_LIMIT}）。\
             下一步：把 limit 调大，或把 query 收紧。",
            req.limit
        );
    }
    format!(
        "索引里共 {total_hits} 条命中，全部返回。\
         命中的依据在每条的 matched_fields（哪一列）与 matches（哪个词、哪一段）。"
    )
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
            let dir = std::env::temp_dir()
                .join(format!("quill-wiki-api-{label}-{}-{n}", std::process::id()));
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
        let err = store
            .read_page("nope.md")
            .expect_err("不存在的页面必须报错");
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
        let err = map_wiki(
            store
                .read_page("../../etc/passwd")
                .expect_err("越界必须被拒"),
        );
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

    // ------------------------------------------------------------ 搜索

    /// 用**生产的**口径造一份真索引：写真页面 → `load_all_pages` → `build_index`
    /// → `render` → `write_index`。手写一段 index.md 会在渲染格式变化时继续绿，
    /// 那验的就不是真的那条路。
    ///
    /// `load_all_pages` 只出现在造数里，**搜索结果一个字节都不来自正文**
    /// （`the_search_reads_only_the_index_and_survives_losing_the_pages` 钉这条）。
    fn seed_index(store: &WikiStore, pages: &[(&str, &str)]) {
        for (rel, text) in pages {
            store.write_page(rel, text).expect("写页");
        }
        let all = store.load_all_pages().expect("回读页面");
        let idx = quill_wiki::ingest::build_index(&all);
        store.write_index(&idx.render()).expect("写索引");
    }

    fn req_of(query: &str, limit: usize) -> SearchRequest {
        let terms = search_terms(query);
        assert!(!terms.is_empty(), "测试 query 必须能切出词");
        SearchRequest {
            query: query.to_string(),
            terms,
            limit,
        }
    }

    #[test]
    fn the_search_really_hits_the_index_built_from_real_pages() {
        let root = TempRoot::new("search-hit");
        let store = store_for(&root, UID_A);
        seed_index(
            &store,
            &[
                (
                    "concepts/所有权.md",
                    "---\ntitle: 所有权\ntype: concept\nupdated: 2026-10-04\nsource_count: 2\n---\n\n资源的唯一归属规则。\n",
                ),
                (
                    "concepts/借用检查.md",
                    "---\ntitle: 借用检查\ntype: concept\nupdated: 2026-10-05\n---\n\n在编译期检查所有权。\n",
                ),
            ],
        );

        let out = search_store(&store, &req_of("所有权", 10)).expect("搜索不该失败");
        assert_eq!(out["index_present"], json!(true));
        assert_eq!(out["index_entries"], json!(2));
        assert_eq!(out["source"], json!("index.md"));
        assert_eq!(out["total_hits"], json!(2), "两条都命中：{out}");
        assert_eq!(out["returned"], json!(2));
        assert_eq!(out["truncated"], json!(false));

        // 标题命中排前（+2 对 +1），并且命中的列要说出来。
        assert_eq!(out["hits"][0]["title"], json!("所有权"));
        assert_eq!(out["hits"][0]["matched_fields"], json!(["title"]));
        assert_eq!(
            out["hits"][0]["source_count"],
            json!(2),
            "source_count 要来自真 frontmatter：{out}"
        );
        assert_eq!(out["hits"][0]["updated"], json!("2026-10-04"));
        assert_eq!(out["hits"][1]["title"], json!("借用检查"));
        assert_eq!(out["hits"][1]["matched_fields"], json!(["summary"]));
        let excerpt = out["hits"][1]["matches"][0]["excerpt"]
            .as_str()
            .expect("摘要命中必须给摘录");
        assert!(excerpt.contains("所有权"), "摘录要含命中词：{excerpt}");
        assert_eq!(out["hits"][1]["matches"][0]["term"], json!("所有权"));
    }

    /// 「只读索引」不是一句口号：把页面文件全删掉，搜索的命中必须一个字不少。
    /// 如果哪天有人把搜索改成读正文，这条立刻红。
    #[test]
    fn the_search_reads_only_the_index_and_survives_losing_the_pages() {
        let root = TempRoot::new("search-index-only");
        let store = store_for(&root, UID_A);
        seed_index(
            &store,
            &[(
                "concepts/所有权.md",
                "---\ntitle: 所有权\ntype: concept\nupdated: 2026-10-04\n---\n\n资源的唯一归属规则。\n",
            )],
        );
        let before = search_store(&store, &req_of("所有权", 10)).expect("搜索");
        assert_eq!(before["total_hits"], json!(1));

        // 页面在 `layer("wiki")` 下（`crates/quill-wiki/src/store.rs:116`：
        // layer = root/<层名>，root 已经是 base/wiki/<用户>），index.md 在同一层的根上 ——
        // 只删 concepts 子目录，索引留着。
        let pages_dir = store
            .layer(quill_wiki::store::DIR_WIKI)
            .expect("wiki 层")
            .join("concepts");
        std::fs::remove_dir_all(&pages_dir).expect("删掉页面目录");
        assert!(!pages_dir.join("所有权.md").exists(), "页面必须真的不在了");

        let after = search_store(&store, &req_of("所有权", 10)).expect("搜索");
        assert_eq!(
            after["total_hits"],
            json!(1),
            "正文没了也该照样命中 —— 搜索的数据源是 index.md：{after}"
        );
    }

    /// 排序与计分必须与 `WikiIndex::lookup` 一致。
    ///
    /// 搜索这一层需要「哪一列命中」而 `lookup` 只给条目，于是这里必须自己算一遍分。
    /// 两份实现漂了的话，`POST /api/wiki/query`（走 lookup）与搜索页的顺序会不一样，
    /// 而两边看起来都「有结果」。这条测试把两边并排比。
    #[test]
    fn the_search_ranking_matches_the_wiki_index_lookup_it_mirrors() {
        let root = TempRoot::new("search-lookup-parity");
        let store = store_for(&root, UID_A);
        seed_index(
            &store,
            &[
                (
                    "a.md",
                    "---\ntitle: 所有权\ntype: concept\nupdated: 2026-10-04\n---\n\n资源的归属规则。\n",
                ),
                (
                    "b.md",
                    "---\ntitle: 借用检查\ntype: concept\nupdated: 2026-10-04\n---\n\n在编译期检查所有权。\n",
                ),
                (
                    "c.md",
                    "---\ntitle: 生命周期\ntype: concept\nupdated: 2026-10-04\n---\n\n与所有权相关。\n",
                ),
            ],
        );
        let idx =
            WikiIndex::parse(&store.read_index().expect("读").expect("有索引")).expect("解析");
        let out = search_store(&store, &req_of("所有权", 10)).expect("搜索");

        let mine: Vec<&str> = out["hits"]
            .as_array()
            .expect("hits 是数组")
            .iter()
            .map(|h| h["title"].as_str().expect("标题是字符串"))
            .collect();
        let lookup: Vec<&str> = idx
            .lookup("所有权", 10)
            .iter()
            .map(|e| e.title.as_str())
            .collect();
        assert_eq!(mine, lookup, "搜索排序与 lookup 漂了：{out}");
        assert_eq!(mine, vec!["所有权", "借用检查", "生命周期"]);
    }

    #[test]
    fn truncation_is_reported_with_the_real_total_not_hidden() {
        let root = TempRoot::new("search-truncated");
        let store = store_for(&root, UID_A);
        seed_index(
            &store,
            &[
                (
                    "a.md",
                    "---\ntitle: 甲\nupdated: 2026-10-04\ntype: concept\n---\n\n共同词甲。\n",
                ),
                (
                    "b.md",
                    "---\ntitle: 乙\nupdated: 2026-10-04\ntype: concept\n---\n\n共同词乙。\n",
                ),
                (
                    "c.md",
                    "---\ntitle: 丙\nupdated: 2026-10-04\ntype: concept\n---\n\n共同词丙。\n",
                ),
            ],
        );

        let out = search_store(&store, &req_of("共同词", 2)).expect("搜索");
        assert_eq!(out["total_hits"], json!(3), "{out}");
        assert_eq!(out["returned"], json!(2));
        assert_eq!(out["truncated"], json!(true));
        assert_eq!(out["limit"], json!(2));
        let note = out["note"].as_str().expect("必须有说明");
        assert!(note.contains("3 条命中"), "总命中数要说清：{note}");
        assert!(note.contains("limit"), "截断要给出路：{note}");
    }

    /// 没有索引 ≠ 没有命中。两种都返回 0，但必须在响应里分得开 ——
    /// 混成一句「没搜到」，用户会去怀疑自己的资料库。
    #[test]
    fn an_absent_index_says_so_instead_of_pretending_the_library_is_empty() {
        let root = TempRoot::new("search-no-index");
        let store = store_for(&root, UID_A);
        let out = search_store(&store, &req_of("所有权", 10)).expect("搜索");
        assert_eq!(out["index_present"], json!(false));
        assert_eq!(out["index_entries"], json!(0));
        assert_eq!(out["total_hits"], json!(0));
        let note = out["note"].as_str().expect("必须有说明");
        assert!(note.contains("没有索引可查"), "{note}");
        assert!(note.contains("不代表"), "要与「没有相关内容」分开：{note}");
    }

    /// 索引里没有 path、解析出的 page_type 也不可用（解析时恒为 Summary）。
    /// 搜索不许把这两样编出来 —— 编了就成假数据。
    #[test]
    fn the_search_does_not_invent_a_path_or_a_page_type_the_index_cannot_supply() {
        let root = TempRoot::new("search-no-invented-fields");
        let store = store_for(&root, UID_A);
        seed_index(
            &store,
            &[(
                "concepts/所有权.md",
                "---\ntitle: 所有权\ntype: concept\nupdated: 2026-10-04\n---\n\n资源的唯一归属规则。\n",
            )],
        );
        let out = search_store(&store, &req_of("所有权", 10)).expect("搜索");
        let hit = out["hits"][0].as_object().expect("命中是对象");
        assert!(!hit.contains_key("path"), "索引里没有路径，不许编：{hit:?}");
        assert!(
            !hit.contains_key("page_type"),
            "解析后的 page_type 恒为 Summary，给出来是假话：{hit:?}"
        );
        // 能给的都得是索引里真有的列。
        for key in ["title", "summary", "source_count", "updated"] {
            assert!(hit.contains_key(key), "索引里有的列要给出来：{key}");
        }
    }

    #[test]
    fn a_query_with_no_searchable_terms_is_a_400_rather_than_zero_hits() {
        let err = parse_search_request(&json!({"query": "!!! ???"})).expect_err("切不出词必须报错");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(err.detail().contains("切词") || err.detail().contains("没有可检索"));
        assert!(err.detail().contains("下一步"));
    }

    #[test]
    fn the_search_request_bounds_are_enforced_and_unknown_keys_are_refused() {
        assert_eq!(
            parse_search_request(&json!({"query": "a"}))
                .expect("默认 limit")
                .limit,
            DEFAULT_SEARCH_LIMIT
        );
        for bad in [
            json!({}),
            json!({"query": "   "}),
            json!({"query": "a", "limit": 0}),
            json!({"query": "a", "limit": 51}),
            json!({"query": "a", "limit": -1}),
            json!({"query": "a", "q": "b"}),
        ] {
            let err = match parse_search_request(&bad) {
                Ok(ok) => panic!("{bad} 必须被拒，实际 limit={}", ok.limit),
                Err(e) => e,
            };
            assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST, "{bad}");
        }
        assert_eq!(
            parse_search_request(&json!({"query": " a ", "limit": 50}))
                .expect("上限本身要收")
                .limit,
            50
        );
    }
}
