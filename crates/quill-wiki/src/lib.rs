pub mod backend;
pub mod date;
pub mod graph;
pub mod index;
pub mod ingest;
pub mod lint;
pub mod log;
pub mod page;
pub mod query;
pub mod store;

pub use backend::{
    ingest_context, lint_context, page_from_wire, page_to_wire, query_context, split_receipt,
};
pub use date::Date;
pub use graph::LinkGraph;
pub use index::{IndexEntry, WikiIndex};
pub use ingest::{IngestOutcome, IngestRequest};
pub use lint::{LintConfig, LintReport, Rule};
pub use log::{LogEntry, LogOp};
pub use page::{Page, PageType, Wikilink};
pub use query::{QueryOutcome, QueryRequest};
pub use store::{WikiError, WikiStore};

pub use quill_adapters::UserId;

pub use quill_adapters::KnowledgeBackend;

pub fn run_lint(store: &WikiStore, date: Date, cfg: &LintConfig) -> Result<LintReport, WikiError> {
    store.ensure_layout()?;
    let pages = store.load_all_pages()?;
    let index = match store.read_index()? {
        Some(t) => Some(WikiIndex::parse(&t)?),
        None => None,
    };
    let report = lint::lint(&pages, index.as_ref(), cfg);
    let entry = LogEntry::new(date, LogOp::Lint, "全库体检", report.render());
    store.append_log(&entry.render())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_surface_is_reachable() {
        let _: Option<PageType> = None;
        let _: Option<LogOp> = None;
        let _: Option<Rule> = None;
        let _: Option<Page> = None;
        let _: Option<WikiIndex> = None;
        let _: Option<LinkGraph> = None;
        let _: fn(&WikiStore, Date, &LintConfig) -> Result<LintReport, WikiError> = run_lint;
    }
}
