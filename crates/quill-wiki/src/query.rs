//! 查询流程编排（规格 §三 3.2）。
//!
//! # 检索策略：把 `index.md` 摊平成候选集，**不做向量检索**
//!
//! 规格 §五与 `docs/V1_SCOPE_CONSTRAINTS.md` 约束 3 明确：
//! 首版「LLM 自读 index.md」，**禁止**引入 embedding 模型 / 向量库。
//! 原始文档实测：约 100 来源 / 数百页面规模下 index.md 方案够用。
//!
//! 因此本模块的检索能力就是 [`WikiIndex::lookup`] 的词面匹配 ——
//! 没有向量、没有 BM25（BM25 是 v1.1）。
//! ⚠️ 这**不是**「一个简化的向量检索」，而是规格选定的方案本身。
//! 若要升级，路径是引入 `pulsar` / `tantivy`，不是把本函数改复杂。
//!
//! # ★ 归档（复利效应的关键）
//!
//! 规格 §3.2 第 4 步：「好答案可归档回 wiki 成为新页面」。
//! [`QueryOutcome::archival_candidate`] 只**建议**不自动写 ——
//! 归档是不可逆的文件改动，判定权必须在人/策略手里。
//! 确认后调 [`archive_answer`] 走与 ingest **同一条**受控写入路径。

use quill_adapters::{AdapterError, KnowledgeBackend, KnowledgePage, UserId};

use crate::backend::{page_from_wire, query_context};
use crate::date::Date;
use crate::index::WikiIndex;
use crate::log::{LogEntry, LogOp};
use crate::page::Page;
use crate::store::{WikiError, WikiStore};

use crate::ingest::{build_index, DEFAULT_SCHEMA_FILE};

/// 喂给模型的最大候选页数。
///
/// ⚠️ **这是一个成本闸门，不是质量参数**：模型输入越大，本次查询越贵。
/// 规格 §九指出「lint 和索引维护的成本不是性能问题，是模型调用成本问题」，
/// 同理适用于查询。给出固定上限让成本可预测。
pub const MAX_CANDIDATE_PAGES: usize = 8;

/// 一次查询的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRequest {
    /// 用户问题原文。
    pub question: String,
    /// 日志日期（由调用方给 —— 本 crate 不读系统时间）。
    pub date: Date,
}

/// 一次查询的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryOutcome {
    /// 带引用的答案。
    pub answer: String,
    /// 实际参与综合的页面。
    pub used_pages: Vec<String>,
    /// ★ 建议归档为新页面的内容（规格 §3.2 第 4 步）。
    ///
    /// 用契约层的 `KnowledgePage`（路径 + 全文）而不是 wiki 的 `Page`：
    /// 归档建议要跨过 LLM 边界传回，边界上只能是低分辨率载荷。
    ///
    /// `None` = 模型认为这个答案不值得沉淀（或无法判定 → 那应当是 `Err`，
    /// 见铁律三态要求）。
    pub archival_candidate: Option<KnowledgePage>,
    /// 追加到 `log.md` 的那一条。
    pub log_entry: Option<LogEntry>,
}

/// 执行一次查询。
pub async fn query<B: KnowledgeBackend>(
    store: &WikiStore,
    backend: &B,
    user: UserId,
    req: &QueryRequest,
) -> Result<QueryOutcome, WikiError> {
    store.ensure_layout()?;

    // 1. 先读 index.md（规格 §3.2 第 1 步：先读它定位相关页面）。
    let index = match store.read_index()? {
        Some(text) => WikiIndex::parse(&text)?,
        None => WikiIndex::new(),
    };

    // 2. 摊平成候选集。零命中时把**全部**条目给模型 ——
    //    「搜不到」时让模型看全表，比让它对着空候选编答案安全。
    let mut candidates: Vec<crate::index::IndexEntry> = index
        .lookup(&req.question, MAX_CANDIDATE_PAGES)
        .into_iter()
        .cloned()
        .collect();
    let fell_back_to_full_index = candidates.is_empty() && !index.is_empty();
    if fell_back_to_full_index {
        candidates = index.entries().take(MAX_CANDIDATE_PAGES).cloned().collect();
    }

    // 3. 读候选页面全文，并把**目录项与它解析到的页面绑成一对**。
    //    索引里可能有已删除页面 → 跳过，不让一次坏条目打断整次查询；
    //    更重要的是：一条解析不到页面的目录项**不能**进上下文，
    //    否则模型会拿到一个指向不存在页面的候选并照着编答案。
    let mut pairs: Vec<(crate::index::IndexEntry, Page)> = Vec::new();
    for e in &candidates {
        if let Ok(p) = page_by_title(store, &e.title) {
            pairs.push((e.clone(), p));
        }
    }
    let candidates_len_used = pairs.len();

    let schema_text = store.read_schema(DEFAULT_SCHEMA_FILE)?;
    let ctx = query_context(req.question.clone(), &pairs, schema_text);

    // 4. 交给模型综合。模型不碰文件系统。
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

    // 5. 记 log.md（规格 §四：query 也要进日志）。
    //    ⚠️ 三个数都写出来（送进模型的 / 索引里共有的）：
    //    两者不等 = 索引里有悬挂条目，只报前者会让人以为「没有失效条目」。
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

/// 归档一个查询答案为新页面（规格 §3.2 第 4 步）。
///
/// 走与 ingest **完全相同**的受控写入路径：
/// 校验内容合法性 → 路径校验 → 写盘 → 重建 `index.md` → 追加 `log.md`。
/// ⚠️ 不另开一条「归档专用」的写入口 —— 那会变成绕过校验的后门。
pub fn archive_answer(
    store: &WikiStore,
    write: &KnowledgePage,
    date: Date,
) -> Result<ArchiveOutcome, WikiError> {
    store.ensure_layout()?;
    // 先过 `page_from_wire`：模型给的归档建议若不是合法页面，
    // 就在**写盘之前**拒掉。默默存一个空页面 = 资料库被无声污染。
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

/// 归档结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveOutcome {
    /// 写入的页面路径。
    pub path: String,
    /// 重建后的索引条目数。
    pub index_entries: usize,
    /// 追加的日志条目。
    pub log_entry: LogEntry,
}

fn candidates_len(store: &WikiStore) -> usize {
    // 只用于日志展示；读不到索引时如实报 0，不伪造数字。
    match store.read_index() {
        Ok(Some(t)) => WikiIndex::parse(&t).map(|i| i.len()).unwrap_or(0),
        _ => 0,
    }
}

/// 按标题找页面（走受控的相对路径）。
///
/// ⚠️ 逐页读取只为拿标题：wiki 在数百页规模，且**不缓存** ——
/// 缓存会让「文件是真相源」变成「进程内有一份可能过期的真相」
/// （规格 §6.2）。
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
    // 悬挂条目（索引里有、文件已删）由调用方跳过，不打断整次查询。
    Err(WikiError::NotFound(format!(
        "没有标题为「{title}」的页面（索引可能已过期，跑一次 ingest 会重建它）"
    )))
}

/// 把 `AdapterError` 归类（供调用方决定是否重试）。
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
