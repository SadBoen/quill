
use quill_adapters::{AdapterError, KnowledgeBackend, KnowledgePage, UserId};

use crate::backend::{page_from_wire, query_context};
use crate::date::Date;
use crate::index::WikiIndex;
use crate::log::{LogEntry, LogOp};
use crate::page::Page;
use crate::store::{WikiError, WikiStore};

use crate::ingest::{build_index, DEFAULT_SCHEMA_FILE};

pub const MAX_CANDIDATE_PAGES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRequest {

    pub question: String,

    pub date: Date,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryOutcome {

    pub answer: String,

    pub used_pages: Vec<String>,

    pub archival_candidate: Option<KnowledgePage>,

    pub log_entry: Option<LogEntry>,
}

pub async fn query<B: KnowledgeBackend>(
    store: &WikiStore,
    backend: &B,
    user: UserId,
    req: &QueryRequest,
) -> Result<QueryOutcome, WikiError> {
    store.ensure_layout()?;

    let index = match store.read_index()? {
        Some(text) => WikiIndex::parse(&text)?,
        None => WikiIndex::new(),
    };

    let mut candidates: Vec<crate::index::IndexEntry> = index
        .lookup(&req.question, MAX_CANDIDATE_PAGES)
        .into_iter()
        .cloned()
        .collect();
    let fell_back_to_full_index = candidates.is_empty() && !index.is_empty();
    if fell_back_to_full_index {
        candidates = index.entries().take(MAX_CANDIDATE_PAGES).cloned().collect();
    }

    let mut pairs: Vec<(crate::index::IndexEntry, Page)> = Vec::new();
    for e in &candidates {
        if let Ok(p) = page_by_title(store, &e.title) {
            pairs.push((e.clone(), p));
        }
    }
    let candidates_len_used = pairs.len();

    let schema_text = store.read_schema(DEFAULT_SCHEMA_FILE)?;
    let ctx = query_context(req.question.clone(), &pairs, schema_text);

    let answer = backend.answer_query(user, ctx).await.map_err(|e| {
        WikiError::Backend(format!(
            "模型侧查询失败：{e}。用 `quill doctor` 诊断 provider 与凭据"
        ))
    })?;

    let mut out = QueryOutcome {
        answer: answer.answer,
        used_pages: pairs.iter().map(|(_, p)| p.path.clone()).collect(),
        archival_candidate: answer.archival_candidate,
        log_entry: None,
    };

    let mut body = format!(
        "查询「{}」：读了 {} 页（送模型的候选 {} 条，index.md 共 {} 条{}{}）。",
        req.question,
        out.used_pages.len(),
        candidates_len_used,
        candidates_len(store),
        if fell_back_to_full_index {
            "，词面零命中 → 回退全表前 N 条"
        } else {
            ""
        },
        if answer.citations.is_empty() {
            "；⚠️ 答案未给出任何引用"
        } else {
            ""
        }
    );
    if !answer.citations.is_empty() {
        body.push_str(&format!("引用：{}", answer.citations.join("、")));
    }
    if out.archival_candidate.is_some() {
        body.push_str("\n★ 模型建议把本答案归档为新页面。");
    }
    let entry = LogEntry::new(req.date, LogOp::Query, req.question.clone(), body);
    store.append_log(&entry.render())?;
    out.log_entry = Some(entry);

    Ok(out)
}

pub fn archive_answer(
    store: &WikiStore,
    write: &KnowledgePage,
    date: Date,
) -> Result<ArchiveOutcome, WikiError> {
    store.ensure_layout()?;

    let page = page_from_wire(write)?;
    store.write_page(&page.path, &write.content)?;
    let pages = store.load_all_pages()?;
    let idx = build_index(&pages);
    store.write_index(&idx.render())?;
    let entry = LogEntry::new(
        date,
        LogOp::Query,
        "归档答案",
        format!("查询答案归档为新页面 `{}`。", page.path),
    );
    store.append_log(&entry.render())?;
    Ok(ArchiveOutcome {
        path: page.path,
        index_entries: idx.len(),
        log_entry: entry,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveOutcome {

    pub path: String,

    pub index_entries: usize,

    pub log_entry: LogEntry,
}

fn candidates_len(store: &WikiStore) -> usize {

    match store.read_index() {
        Ok(Some(t)) => WikiIndex::parse(&t).map(|i| i.len()).unwrap_or(0),
        _ => 0,
    }
}

fn page_by_title(store: &WikiStore, title: &str) -> Result<Page, WikiError> {
    for rel in store.list_pages()? {
        if rel == title || rel == format!("{title}.md") {
            return store.read_page(&rel);
        }
        let p = store.read_page(&rel)?;
        if p.title() == title {
            return Ok(p);
        }
    }

    Err(WikiError::NotFound(format!(
        "没有标题为「{title}」的页面（索引可能已过期，跑一次 ingest 会重建它）"
    )))
}

pub fn classify(e: &AdapterError) -> &'static str {
    match e {
        AdapterError::Provider(_) => "provider 侧失败（可重试）",
        AdapterError::Storage(_) => "存储侧失败（查磁盘/权限）",
        AdapterError::Unauthorized(_) | AdapterError::Forbidden(_) => {
            "鉴权/授权失败（查凭据与角色）"
        }
        AdapterError::NotFound(_) => "目标不存在",
        AdapterError::Conflict(_) => "状态冲突",
        AdapterError::Internal(_) => "内部不变量被破坏（需修代码）",
    }
}
