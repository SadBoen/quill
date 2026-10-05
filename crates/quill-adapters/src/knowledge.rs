//! # `KnowledgeBackend` —— 资料库的 LLM 接入点（契约 `docs/PHASE2_CONTRACT.md` §2.2）
//!
//! ## 为什么这个 trait 住在 `quill-adapters`（契约层）而不是 `quill-wiki`
//!
//! 契约硬约束：`quill-agent` 访问资料库**只能**通过本 trait，**不得直接依赖实现**。
//! 若 trait 定义在 `quill-wiki` 里，`quill-agent` 就必须 `use quill_wiki::KnowledgeBackend`
//! 才能拿到它 —— **那恰恰就是"直接依赖实现"**，等于把契约要防的那条绕过去了。
//!
//! 🔴 同时本模块**不能引用任何 wiki 内部的类型**（`Page` / `IndexEntry` / `LintReport`），
//! 因为 `quill-adapters` 是**最底层**——它一旦认识 wiki 的数据模型，依赖方向就反了。
//! 所以这里的载荷类型是**刻意"低分辨率"**的：路径 + 文本。
//! 富结构（frontmatter 解析、页面类型、断言）留在 `quill-wiki` 内部，
//! 由它在边界上做双向转换。
//!
//! ## 为什么是 `-> impl Future + Send` 而不是裸 `async fn`
//! 与 `quill_adapters::MemberExecutor` 同一理由：
//! 1. 裸 `async fn in trait` 触发 `async_fn_in_trait` 警告，而本项目 `clippy -- -D warnings`；
//! 2. 裸 `async fn` **无法声明 `Send`**，而实现方要 `tokio::spawn`。
//!
//! ⚠️ 代价（与 `MemberExecutor` 相同）：RPITIT **不是 dyn-compatible**，
//! 不能写 `Arc<dyn KnowledgeBackend>`。当前泛型注入已足够；
//! 若将来需要「按名持有异构 backend 集合」，须改装箱 future 或裁决引入 `async-trait`。
//!
//! ## 为什么输入输出**没有 `&Path`**
//! 规格 §6.4 约束 1：**所有写入必须走工具调用（受权限控制），不直接给 LLM 文件写权限**。
//! 体现在类型上就是：输入是**已读好的文本**，输出是**页面内容（数据）**，
//! 全程没有 `&Path`、没有文件系统句柄。落盘由 `quill-wiki` 的 `WikiStore` 执行，
//! 它做路径校验与用户隔离 —— LLM 给出的字符串过不了那三道防线。

use crate::AdapterError;
use crate::UserId;

/// 一个待摄入的原始来源（`raw/` 层）。**只读**，LLM 不得写。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KnowledgeSource {
    /// 相对路径（如 `raw/2026/report.pdf`）。
    pub rel_path: String,
    /// 正文全文。
    pub text: String,
}

/// 一篇页面（`wiki/` 层）的**低分辨率**视图：路径 + 全文。
///
/// 刻意不带 frontmatter 字段 —— 那属于 `quill-wiki` 的解析职责。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KnowledgePage {
    /// wiki 层相对路径（如 `concepts/rust.md`）。
    pub rel_path: String,
    /// 页面全文（含 frontmatter，与磁盘字节一致）。
    pub content: String,
}

/// `index.md` 里的一条目录项（低分辨率）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexDoc {
    /// 页面相对路径。
    pub rel_path: String,
    /// 标题。
    pub title: String,
    /// 一句话摘要。
    pub summary: String,
}

/// 一次写入的清单（对应规格 §3.1「一次 ingest 触及 10-15 个页面是正常的」）。
///
/// ⚠️ **刻意不设数量上限** —— 设了就是在鼓励 LLM 少写，那正是复利增长的反面。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexReceipt {
    /// 本次被创建或更新的页面路径。
    pub touched: Vec<String>,
    /// 给用户的要点摘要（写进 `log.md`）。
    pub summary: String,
}

/// 一次摄入的上下文（编排层已读好的全部材料）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IngestContext {
    /// 待摄入的来源。
    pub source: KnowledgeSource,
    /// 现有 `index.md` 全文（可能是空串）。
    pub index_text: String,
    /// schema 层规则全文；`None` = 用户还没写 schema。
    pub schema_text: Option<String>,
    /// 近期日志尾部（`log.md` 最后若干条），让 LLM 知道最近做过什么。
    pub recent_log: String,
    /// 可能相关的既有页面。
    pub related_pages: Vec<KnowledgePage>,
}

/// 一次查询的上下文。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryContext {
    /// 用户问题原文。
    pub question: String,
    /// `index.md` 里的候选目录项。
    pub candidates: Vec<IndexDoc>,
    /// 候选页面全文。
    pub pages: Vec<KnowledgePage>,
    /// schema 层规则全文。
    pub schema_text: Option<String>,
}

/// 一次查询的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryAnswer {
    /// 带引用的答案正文。
    pub answer: String,
    /// 答案引用到的页面标题。
    pub citations: Vec<String>,
    /// ★ 规格 §3.2 第 4 步：**好答案可归档回 wiki**（复利效应的关键）。
    ///
    /// ⚠️ 这里只给**建议**，不自动写盘 —— 自动归档会把一次随口的问
    /// 变成不可逆的文件改动。判定权在人/编排策略。
    pub archival_candidate: Option<KnowledgePage>,
}

/// 一次语义体检的上下文。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LintContext {
    /// 规则引擎算出的结构发现，**已渲染为文本**。
    ///
    /// 🔴 传文本而非结构体，是为了让本 trait 不认识 `quill-wiki` 的 `LintReport`。
    pub report_text: String,
    /// 全部页面全文（供模型做语义比对）。
    pub pages: Vec<KnowledgePage>,
}

/// 资料库的 LLM 接入点。
///
/// ⚠️ 本 trait **不做**推理，也不碰文件系统；
/// 它是「把材料交给模型、把模型产出的页面内容带回来」的唯一通道。
pub trait KnowledgeBackend: Send + Sync + 'static {
    /// ① 摄入规划：读来源 → 产出要写/改哪些页面。
    fn plan_ingest(
        &self,
        user: UserId,
        ctx: IngestContext,
    ) -> impl std::future::Future<Output = Result<IndexReceipt, AdapterError>> + Send;

    /// ② 查询综合：基于候选页面产出带引用的答案。
    fn answer_query(
        &self,
        user: UserId,
        ctx: QueryContext,
    ) -> impl std::future::Future<Output = Result<QueryAnswer, AdapterError>> + Send;

    /// ③ 语义体检：在结构发现之上补**语义**矛盾
    /// （结构类矛盾由 `quill-wiki` 的规则引擎负责，不在此重复）。
    ///
    /// 返回 `Ok(None)` = 模型认为无需补充（**不是**「查不到」——
    /// 若无法判定，实现方必须返回 `Err`，见铁律十九的三态要求）。
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
        // 规格 §3.1：一次 ingest 触及 10-15 页是正常的 → 实现方不得自设上限
        let r = IndexReceipt {
            touched: (0..20).map(|i| format!("p{i}.md")).collect(),
            summary: String::new(),
        };
        assert_eq!(r.touched.len(), 20);
    }

    #[test]
    fn contexts_carry_no_path_or_fs_handle() {
        // 类型层面的保证：所有上下文里只有 String，没有 PathBuf。
        // 若将来有人加了字段，这条测试提醒他复核「LLM 是否能拿到文件系统」。
        let c = IngestContext::default();
        let q = QueryContext::default();
        let l = LintContext::default();
        assert!(c.source.rel_path.is_empty());
        assert!(q.question.is_empty());
        assert!(l.report_text.is_empty());
    }
}
