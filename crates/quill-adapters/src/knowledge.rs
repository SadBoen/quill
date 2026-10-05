
use crate::AdapterError;
use crate::UserId;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KnowledgeSource {

    pub rel_path: String,

    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KnowledgePage {

    pub rel_path: String,

    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexDoc {

    pub rel_path: String,

    pub title: String,

    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexReceipt {

    pub touched: Vec<String>,

    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IngestContext {

    pub source: KnowledgeSource,

    pub index_text: String,

    pub schema_text: Option<String>,

    pub recent_log: String,

    pub related_pages: Vec<KnowledgePage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryContext {

    pub question: String,

    pub candidates: Vec<IndexDoc>,

    pub pages: Vec<KnowledgePage>,

    pub schema_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryAnswer {

    pub answer: String,

    pub citations: Vec<String>,

    pub archival_candidate: Option<KnowledgePage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LintContext {

    pub report_text: String,

    pub pages: Vec<KnowledgePage>,
}

pub trait KnowledgeBackend: Send + Sync + 'static {

    fn plan_ingest(
        &self,
        user: UserId,
        ctx: IngestContext,
    ) -> impl std::future::Future<Output = Result<IndexReceipt, AdapterError>> + Send;

    fn answer_query(
        &self,
        user: UserId,
        ctx: QueryContext,
    ) -> impl std::future::Future<Output = Result<QueryAnswer, AdapterError>> + Send;

    fn lint_semantics(
        &self,
        user: UserId,
        ctx: LintContext,
    ) -> impl std::future::Future<Output = Result<Option<Vec<String>>, AdapterError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_receipt_keeps_every_touched_page() {

        let r = IndexReceipt {
            touched: (0..20).map(|i| format!("p{i}.md")).collect(),
            summary: String::new(),
        };
        assert_eq!(r.touched.len(), 20);
    }

    #[test]
    fn contexts_carry_no_path_or_fs_handle() {

        let c = IngestContext::default();
        let q = QueryContext::default();
        let l = LintContext::default();
        assert!(c.source.rel_path.is_empty());
        assert!(q.question.is_empty());
        assert!(l.report_text.is_empty());
    }
}
