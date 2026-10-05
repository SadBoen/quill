//! LLM 接入点的**适配层** —— 把 `quill-wiki` 的富类型翻译成契约层的低分辨率载荷。
//!
//! # 2026-10-05 修订：trait 迁到 `quill-adapters`
//!
//! 本文件原先**定义** `KnowledgeBackend` 及其上下文类型。契约 `docs/PHASE2_CONTRACT.md` §2.2
//! 要求它住在 `quill-adapters`，而实现时该 trait 尚不存在，遂在此按契约形状定义并登记偏差。
//!
//! **现已在 `crates/quill-adapters/src/knowledge.rs` 落地**，本文件降级为转换层。
//!
//! 🔴 为什么必须迁，而不是「就地定义挺好」：
//! 契约硬约束是「`quill-agent` 通过 `KnowledgeBackend` trait 访问资料库，**不直接依赖实现**」。
//! trait 若定义在 `quill-wiki`，`quill-agent` 就得 `use quill_wiki::KnowledgeBackend`
//! 才能拿到它 —— **那正是"直接依赖实现"**，把契约要防的洞又打开了。
//!
//! 同时契约层**不能**认识 wiki 的 `Page` / `IndexEntry` / `LintReport`：
//! `quill-adapters` 是最底层，一旦认识下游数据模型依赖方向就反了。
//! 故契约层的载荷刻意"低分辨率"（路径 + 文本），富结构留在这里转换。
//!
//! 保持「LLM 无文件系统权限」：所有转换后的类型里只有 `String`，没有 `&Path`。
//! 落盘仍由 [`crate::store::WikiStore`] 执行，它做路径校验与用户隔离。

use quill_adapters::{
    IndexDoc, IndexReceipt, IngestContext, KnowledgePage, KnowledgeSource, LintContext,
};

use crate::index::IndexEntry;
use crate::page::{render_page, Page, PageWarning};
use crate::store::WikiError;

/// `quill-wiki::Page` → 契约层的 `KnowledgePage`。
pub fn page_to_wire(p: &Page) -> KnowledgePage {
    KnowledgePage {
        rel_path: p.path.clone(),
        content: render_page(p),
    }
}

/// 判定一个解析告警是否属于「**这根本不是一个页面文件**」。
///
/// ⚠️ 只收 frontmatter **块本身**坏掉的三种。`UnknownPageType` / `BadDate`
/// **不在此列**：字段值写错是内容问题，归 [`crate::lint::Rule`] 去报，
/// 「体检发现问题」与「这压根不是页面」是两件事 —— 归错类会把人引向错误的排查方向。
fn is_structural_parse_failure(w: &PageWarning) -> bool {
    matches!(
        w,
        PageWarning::MissingFrontmatter
            | PageWarning::UnterminatedFrontmatter
            | PageWarning::MalformedFrontmatterLine { .. }
    )
}

/// 契约层的 `KnowledgePage` → `quill-wiki::Page`。
///
/// ⚠️ 解析失败**不兜底**：返回 `Err`，让调用方知道模型给的内容不是合法页面。
/// 默默存一个空页面 = 资料库被无声污染，比报错糟得多。
///
/// 「解析失败」在这里有**两个确切的含义**（`parse_page` 本身不会 panic，
/// 它对坏输入一律降级并记 `PageWarning`，所以判红的责任落在本函数）：
/// 1. `rel_path` 形态非法（空串 / 不以 `.md` 结尾）；
/// 2. frontmatter 块结构性损坏（缺失 / 未闭合 / 字段行写歪）。
///
/// 越权路径**不在**这里判 —— 它由 [`crate::store::WikiStore::resolve`]
/// 在写盘时拦下，那三道防线是真正的隔离边界，本函数不去重复它。
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

/// `index.md` 条目 + 它**解析到的页面** → 契约层的 `IndexDoc`。
///
/// 🔴 路径**必须**由调用方给，不能从 [`IndexEntry`] 里取：
/// `IndexEntry` 本身就**不存路径**（它只有标题/摘要/类型，页面靠标题反查，
/// 见 [`crate::query`])。硬编一个路径就是把「也许对」写成「一定对」。
pub fn index_entry_to_wire(e: &IndexEntry, resolved_path: &str) -> IndexDoc {
    IndexDoc {
        rel_path: resolved_path.to_string(),
        title: e.title.clone(),
        summary: e.summary.clone(),
    }
}

/// 组装摄入上下文：把 wiki 内部读好的材料转成契约层载荷。
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

/// 组装查询上下文。
///
/// ⚠️ `candidates` 收 `(&IndexEntry, &Page)` **配对**而不是两个独立列表：
/// 索引里可能有过期条目（页面已删），而 [`IndexEntry`] 不带路径，
/// 只有配上传入才能把「目录项」和「它真的解析到了哪一页」绑在一起。
/// 收两个平行列表就等于允许调用方传出**对不上**的候选与正文 ——
/// 那会让模型看到一条指向不存在页面的目录项，然后照着编答案。
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

/// 组装体检上下文：结构报告**渲染成文本**后再跨界。
///
/// 🔴 传文本而非 `LintReport` 结构体，是为了让契约层不认识本 crate 的类型。
pub fn lint_context(report_text: String, pages: &[Page]) -> LintContext {
    LintContext {
        report_text,
        pages: pages.iter().map(page_to_wire).collect(),
    }
}

/// 契约层的 `IndexReceipt` → 内部使用的 `(触及页面, 摘要)`。
///
/// 刻意**不做去重或数量限制** —— 规格 §3.1 明确「一次 ingest 触及 10-15 页是正常的」，
/// 在这里加限制就是在逼 LLM 少写。
pub fn split_receipt(r: IndexReceipt) -> (Vec<String>, String) {
    (r.touched, r.summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_types_carry_no_path_handles() {
        // 跨界的载荷里只有 String：LLM 拿不到文件系统
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

    /// 违规侧：模型给出不合法页面**必须** Err，不能兜底成空页面。
    #[test]
    fn page_from_wire_rejects_unparsable_content_instead_of_defaulting() {
        // ① 完全没有 frontmatter
        assert!(
            page_from_wire(&KnowledgePage {
                rel_path: "a.md".into(),
                content: "# 只有正文\n".into(),
            })
            .is_err(),
            "缺 frontmatter 的内容被兜底成了合法页面 = 资料库被无声污染"
        );
        // ② frontmatter 未闭合
        assert!(page_from_wire(&KnowledgePage {
            rel_path: "b.md".into(),
            content: "---\ntitle: 页\n\n正文。\n".into(),
        })
        .is_err());
        // ③ 路径形态非法
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

    /// 合法侧：证明上一条不是「一律拒绝」（铁律十二：禁令必须配合法反例）。
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

    /// 往返不丢内容：`parse → render → parse` 必须幂等，
    /// 否则模型每次重写都会让页面逐次掉内容。
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
