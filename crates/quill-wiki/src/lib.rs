//! xu-wiki：raw / wiki / schema 三层 + ingest / query / lint 三操作。
//!
//! 规格：`docs/XU_WIKI_SPEC.md`（权威）。本 crate 是它的**数据层实现**。
//!
//! # 三层架构
//!
//! | 层 | 目录 | 谁写 | 本 crate 的入口 |
//! |---|---|---|---|
//! | 原始来源（不可变） | `raw/` | 人 / 外部导入 | [`store::WikiStore::write_raw`]（LLM **只读**） |
//! | 知识层 | `wiki/` | LLM 经工具调用 | [`store::WikiStore::write_page`] |
//! | 规则层 | `schema/` | 人与 LLM 共同演进 | [`store::WikiStore::read_schema`]（只读） |
//!
//! # 文件是真相源
//!
//! 规格 §6.2：Markdown 文件是真相源，数据库只做索引缓存。
//! **本 crate 零数据库依赖** —— 索引缓存是 `wiki_index` 表的活
//! （`crates/quill-store`），而它的重建入口规格已注明「尚未实现」。
//! 这里不预先造一个：没有第二个真相源，就没有两者不一致的可能。
//!
//! # 依赖方向（契约 `docs/PHASE2_CONTRACT.md` §一）
//!
//! - ✅ 依赖 `quill-adapters`（`UserId` / `AdapterError`）、`quill-domain`
//! - 🔴 **禁止**依赖 `quill-agent`（`scripts/check-crate-deps.sh` G40-b 强制）
//! - 🔴 **不新增外部 crate**：本 crate 的 `[dependencies]` 只有两个 quill crate。
//!   frontmatter / wikilink / 日期解析全部手写（见 [`page`] / [`date`]）。
//!
//! # 多用户隔离
//!
//! 目录级隔离（契约 §七 data-engineer 第 1 条）。
//! **每一个**路径 API 都强制带 `&UserId`（[`store::WikiStore`]），
//! 且有 `resolve` 三道防线挡越权路径。隔离测试见 `tests/isolation.rs`。
//!
//! # LLM 的权限边界
//!
//! 规格 §6.4 约束 1：**不直接给 LLM 文件写权限**。
//! 跨 LLM 边界的 [`quill_adapters::IngestContext`] / [`quill_adapters::QueryContext`]
//! 里只有**文本**，没有任何 `PathBuf` 或文件系统句柄；
//! LLM 侧拿到的是页面**内容**，落盘由 [`store::WikiStore`] 执行，
//! 且必须过 [`store::WikiStore::resolve`] 的三道防线。
//!
//! # 依赖方向：`KnowledgeBackend` 在 `quill-adapters`，不在这里
//!
//! [`backend`] 是**转换层**，不是 trait 的家。trait 住在契约层
//! [`quill_adapters::KnowledgeBackend`]（契约 §2.2），原因见该模块文档。
//! 下方的 `pub use` 只是**转出**同一个 trait，方便既有调用方少写一条依赖 ——
//! 它不表示 trait 定义在此。
//!
//! # 首版**不做**向量检索
//!
//! `docs/V1_SCOPE_CONSTRAINTS.md` 约束 3 + 规格 §五：
//! 首版检索方式就是「LLM 自读 `index.md`」。
//! 见 [`query`] 模块文档与 [`index::WikiIndex::lookup`]。

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

/// 重导出 `UserId`（契约 §四：身份类型由 `quill-adapters` 定义，
/// `quill-domain` re-export；本 crate 同理，避免调用方多写一条依赖）。
pub use quill_adapters::UserId;

/// 重导出契约层的 [`KnowledgeBackend`] —— **只是转出同一个 trait，不是定义**。
///
/// 🔴 trait 的家在 `quill-adapters`（契约 §2.2），不在本 crate。
/// 本 crate 只在 [`backend`] 里做双向转换。
/// 保留这条重导出是为了让既有调用方（以及本 crate 的测试）不必为拿 trait
/// 多加一条 `quill-adapters` 依赖 —— 但**新代码请直接
/// `use quill_adapters::KnowledgeBackend`**，那才是契约要求的写法。
pub use quill_adapters::KnowledgeBackend;

/// 运行一次体检（读盘 → 规则引擎 → 写 `log.md`）。
///
/// 规格 §6.4 约束 4：**lint 是周期性的，不是每次写入都做**。
/// 因此本函数**不**在 [`ingest`] 里被调用 —— 由调度侧（每日一次全量）
/// 显式触发。把它塞进 ingest 会让每次摄入都付一次全库体检的钱。
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
        // 让 lib.rs 的重导出被真实使用：pub 项不被引用时 rustc 不报 unused，
        // 但这条测试保证「重导出名真的指向存在的类型」。
        let _: Option<PageType> = None;
        let _: Option<LogOp> = None;
        let _: Option<Rule> = None;
        let _: Option<Page> = None;
        let _: Option<WikiIndex> = None;
        let _: Option<LinkGraph> = None;
        let _: fn(&WikiStore, Date, &LintConfig) -> Result<LintReport, WikiError> = run_lint;
    }
}
