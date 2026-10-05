use quill_adapters::{
    IndexDoc, IndexReceipt, IngestContext, KnowledgePage, KnowledgeSource, LintContext,
};

use crate::index::IndexEntry;
use crate::page::{render_page, Page, PageWarning};
use crate::store::WikiError;

pub fn page_to_wire(p: &Page) -> KnowledgePage {
    KnowledgePage {
        rel_path: p.path.clone(),
        content: render_page(p),
    }
}

fn is_structural_parse_failure(w: &PageWarning) -> bool {
    matches!(
        w,
        PageWarning::MissingFrontmatter
            | PageWarning::UnterminatedFrontmatter
            | PageWarning::MalformedFrontmatterLine { .. }
    )
}

pub fn page_from_wire(w: &KnowledgePage) -> Result<Page, WikiError> {
    let rel = w.rel_path.trim();
    if rel.is_empty() {
        return Err(WikiError::InvalidPath {
            attempt: w.rel_path.clone(),
            reason: "页面相对路径为空",
        });
    }
    if !rel.ends_with(".md") {
        return Err(WikiError::InvalidPath {
            attempt: w.rel_path.clone(),
            reason: "页面路径必须以 .md 结尾",
        });
    }

    let page = crate::page::parse_page(rel, &w.content);
    if let Some(bad) = page
        .warnings
        .iter()
        .find(|x| is_structural_parse_failure(x))
    {
        return Err(WikiError::Backend(format!(
            "模型返回的页面 {rel:?} 不是合法页面：{bad}。\
             请让模型按 schema 层规则重写（文件第一行必须是 `---`，且 frontmatter 必须闭合）"
        )));
    }
    Ok(page)
}

pub fn index_entry_to_wire(e: &IndexEntry, resolved_path: &str) -> IndexDoc {
    IndexDoc {
        rel_path: resolved_path.to_string(),
        title: e.title.clone(),
        summary: e.summary.clone(),
    }
}

pub fn ingest_context(
    source_rel: String,
    source_text: String,
    index_text: String,
    schema_text: Option<String>,
    recent_log: String,
    related_pages: &[Page],
) -> IngestContext {
    IngestContext {
        source: KnowledgeSource {
            rel_path: source_rel,
            text: source_text,
        },
        index_text,
        schema_text,
        recent_log,
        related_pages: related_pages.iter().map(page_to_wire).collect(),
    }
}

pub fn query_context(
    question: String,
    candidates: &[(IndexEntry, Page)],
    schema_text: Option<String>,
) -> quill_adapters::QueryContext {
    quill_adapters::QueryContext {
        question,
        candidates: candidates
            .iter()
            .map(|(e, p)| index_entry_to_wire(e, &p.path))
            .collect(),
        pages: candidates.iter().map(|(_, p)| page_to_wire(p)).collect(),
        schema_text,
    }
}

pub fn lint_context(report_text: String, pages: &[Page]) -> LintContext {
    LintContext {
        report_text,
        pages: pages.iter().map(page_to_wire).collect(),
    }
}

pub fn split_receipt(r: IndexReceipt) -> (Vec<String>, String) {
    (r.touched, r.summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_types_carry_no_path_handles() {
        let p = KnowledgePage {
            rel_path: "a.md".into(),
            content: "b".into(),
        };
        assert_eq!(p.rel_path, "a.md");
        let c = ingest_context(
            "s.pdf".into(),
            "t".into(),
            String::new(),
            None,
            String::new(),
            &[],
        );
        assert_eq!(c.source.rel_path, "s.pdf");
    }

    #[test]
    fn receipt_split_preserves_all_touched_pages() {
        let (touched, summary) = split_receipt(IndexReceipt {
            touched: (0..20).map(|i| format!("p{i}.md")).collect(),
            summary: "s".into(),
        });
        assert_eq!(touched.len(), 20, "一次 ingest 触及多页是正常的，不得截断");
        assert_eq!(summary, "s");
    }

    const LEGAL: &str =
        "---\ntitle: 页\ntype: concept\ncreated: 2026-10-04\nupdated: 2026-10-04\n---\n\n正文。\n";

    #[test]
    fn page_from_wire_rejects_unparsable_content_instead_of_defaulting() {
        assert!(
            page_from_wire(&KnowledgePage {
                rel_path: "a.md".into(),
                content: "# 只有正文\n".into(),
            })
            .is_err(),
            "缺 frontmatter 的内容被兜底成了合法页面 = 资料库被无声污染"
        );

        assert!(page_from_wire(&KnowledgePage {
            rel_path: "b.md".into(),
            content: "---\ntitle: 页\n\n正文。\n".into(),
        })
        .is_err());

        assert!(page_from_wire(&KnowledgePage {
            rel_path: String::new(),
            content: LEGAL.into(),
        })
        .is_err());
        assert!(page_from_wire(&KnowledgePage {
            rel_path: "c.txt".into(),
            content: LEGAL.into(),
        })
        .is_err());
    }

    #[test]
    fn page_from_wire_accepts_a_legal_page() {
        let p = page_from_wire(&KnowledgePage {
            rel_path: "concepts/页.md".into(),
            content: LEGAL.into(),
        })
        .expect("合法页面必须放行");
        assert_eq!(p.frontmatter.title.as_deref(), Some("页"));
        assert_eq!(p.page_type(), Some(crate::page::PageType::Concept));
    }

    #[test]
    fn page_wire_round_trip_is_stable() {
        let first = page_from_wire(&KnowledgePage {
            rel_path: "concepts/页.md".into(),
            content: LEGAL.into(),
        })
        .expect("解析");
        let second = page_from_wire(&page_to_wire(&first)).expect("再解析");
        assert_eq!(render_page(&first), render_page(&second));
    }
}
