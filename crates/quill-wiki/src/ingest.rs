
use quill_adapters::{AdapterError, KnowledgeBackend, UserId};

use crate::backend::{ingest_context, split_receipt};
use crate::date::Date;
use crate::index::{IndexEntry, WikiIndex};
use crate::log::{LogEntry, LogOp, ParsedLog};
use crate::page::{render_page, Page};
use crate::store::{WikiError, WikiStore};

pub const RECENT_LOG_ENTRIES: usize = 5;

pub const DEFAULT_SCHEMA_FILE: &str = "AGENTS.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestRequest {

    pub source_rel: String,

    pub date: Date,

    pub focus: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestOutcome {

    pub written: Vec<String>,

    pub index_entries: usize,

    pub log_entry: Option<LogEntry>,
}

impl IngestOutcome {

    pub fn touched_count(&self) -> usize {
        self.written.len()
    }
}

pub async fn ingest<B: KnowledgeBackend>(
    store: &WikiStore,
    backend: &B,
    user: UserId,
    req: &IngestRequest,
) -> Result<IngestOutcome, WikiError> {

    store.ensure_layout()?;

    let source_text = store.read_raw(&req.source_rel)?;

    let index_text = store.read_index()?.unwrap_or_default();
    let schema_text = store.read_schema(DEFAULT_SCHEMA_FILE)?;
    let recent_log = match store.read_log()? {
        Some(text) => {
            let parsed: ParsedLog = crate::log::parse_log(&text);
            parsed
                .tail(RECENT_LOG_ENTRIES)
                .iter()
                .map(|e| e.heading())
                .collect::<Vec<_>>()
                .join("\n")
        }
        None => String::new(),
    };
    let all_pages = store.load_all_pages()?;
    let related_pages = pick_related(&all_pages, &req.focus, &req.source_rel);

    let ctx = ingest_context(
        req.source_rel.clone(),
        source_text,
        index_text,
        schema_text,
        recent_log,
        &related_pages,
    );

    let receipt = backend
        .plan_ingest(user, ctx)
        .await
        .map_err(|e| ingest_backend_error(&e))?;
    let (touched, summary) = split_receipt(receipt);

    let mut outcome = IngestOutcome::default();
    for (i, rel) in touched.iter().enumerate() {
        match store.read_page(rel) {
            Ok(_) => {}
            Err(e @ (WikiError::PathEscape { .. } | WikiError::InvalidPath { .. })) => {
                return Err(e);
            }
            Err(e) => {
                return Err(WikiError::Backend(format!(
                    "模型声称已写入第 {}/{} 页 {rel:?}，但该页读不回来：{e}。\
                     已确认落盘的页面：{:?}。可重跑本 ingest（已写页面会被覆盖）",
                    i + 1,
                    touched.len(),
                    outcome.written
                )));
            }
        }
        outcome.written.push(rel.clone());
    }

    let pages_after = store.load_all_pages()?;
    let idx = build_index(&pages_after);
    outcome.index_entries = idx.len();
    store.write_index(&idx.render())?;

    let entry = LogEntry::new(
        req.date,
        LogOp::Ingest,
        req.source_rel.clone(),
        format!(
            "摄入来源 `{}`：写入 {} 页（{}），index.md 共 {} 条。\n{}",
            req.source_rel,
            outcome.written.len(),
            outcome
                .written
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join("、"),
            outcome.index_entries,
            summary
        ),
    );
    store.append_log(&entry.render())?;
    outcome.log_entry = Some(entry);

    Ok(outcome)
}

pub fn build_index(pages: &[Page]) -> WikiIndex {
    let mut idx = WikiIndex::new();
    for p in pages {
        if is_structural(&p.path) {
            continue;
        }
        if let Some(e) = IndexEntry::from_page(p) {
            idx.upsert(e);
        }
    }
    idx
}

fn is_structural(path: &str) -> bool {
    crate::graph::STRUCTURAL_FILES.contains(&path)
}

pub fn pick_related(pages: &[Page], focus: &[String], source_rel: &str) -> Vec<Page> {
    let stem = source_rel
        .rsplit('/')
        .next()
        .unwrap_or(source_rel)
        .to_lowercase();
    pages
        .iter()
        .filter(|p| !is_structural(&p.path))
        .filter(|p| {
            if focus.is_empty() {

                return p.path.to_lowercase().contains(&stem);
            }
            let title = p.title().to_lowercase();
            let body = p.body.to_lowercase();
            focus.iter().any(|f| {
                let f = f.to_lowercase();
                title.contains(&f) || body.contains(&f)
            })
        })
        .cloned()
        .collect()
}

pub fn reload(store: &WikiStore, rel: &str) -> Result<Page, WikiError> {
    store.read_page(rel)
}

pub fn render(p: &Page) -> String {
    render_page(p)
}

pub fn writable_layers() -> [&'static str; 1] {
    [crate::store::DIR_WIKI]
}

pub fn schema_file() -> &'static str {
    DEFAULT_SCHEMA_FILE
}

fn ingest_backend_error(e: &AdapterError) -> WikiError {
    WikiError::Backend(format!(
        "模型侧摄入规划失败：{e}。请检查 provider 配置与凭据（用 `quill doctor` 诊断）"
    ))
}
