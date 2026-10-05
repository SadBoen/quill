//! 摄入流程编排（规格 §三 3.1）。
//!
//! # 编排层做什么、不做什么
//!
//! **做**（确定性、可测试、不需要模型）：
//! 1. 按用户作用域读 raw 来源；
//! 2. 读 `index.md` / schema / `log.md` 尾部 / 相关页面，组装 [`quill_adapters::IngestContext`]；
//! 3. 调 [`KnowledgeBackend::plan_ingest`]，拿回一张**写入回执**（[`quill_adapters::IndexReceipt`]）；
//! 4. **逐条核对回执** —— 声称写了的页必须真的读得回来；
//! 5. 重建 `index.md`；
//! 6. 追加 `log.md`。
//!
//! **不做**：提炼、总结、决定写哪几页 —— 那是模型的活。
//!
//! # ★ 为什么「写盘」不在上面第 3~4 步里（2026-10-05 trait 迁移的连带后果）
//!
//! 契约层 [`KnowledgeBackend::plan_ingest`] 的返回类型是 [`IndexReceipt`]，
//! 字段只有 `touched: Vec<String>`（路径）与 `summary` —— **没有页面内容**。
//! 旧版返回的 `IngestPlan { writes: Vec<PageWrite> }` 带着正文，编排层才有东西可写。
//! 契约 `docs/PHASE2_CONTRACT.md` §2.2 自己画的 `index(...) -> IndexReceipt` 也是同一形态：
//! **回执是「已写入」的确认，不是内容的载体**。
//!
//! 因此现在的分工是：
//! - **模型侧（backend 实现）**：产出内容并经**工具调用**写盘 ——
//!   写入仍必须过 [`WikiStore::write_page`]，那仍是唯一的写入口，
//!   路径校验与用户隔离三道防线一道不少，「LLM 无文件写权限」没有被削弱
//!   （规格 §6.4 约束 1 说的就是「走工具调用」，不是「由本 crate 代劳」）；
//! - **本编排层**：给模型递材料，收回执，**核对回执**，重建索引，记日志。
//!
//! # 为什么必须核对回执（这是新加的闸门，不是锦上添花）
//!
//! 回执是**模型侧给的断言**，本层不能照单全收：
//! 一个只报成功、实际没写的 backend，会让 `index.md` 与 `log.md` 记下
//! 「写了 12 页」而磁盘上一页都没有 —— 一次**静默失败**，
//! 而且它的表现形式是全绿的日志，正是最难被发现的那一类。
//! 故第 4 步逐条 [`WikiStore::read_page`]：读不回来就 `Err`，
//! 并在错误里说清「已完成到哪」（铁律七）。
//!
//! # 幂等与部分失败
//!
//! - **同一次 ingest 重跑**：`upsert` 按标题覆盖、`index.md` 全量重渲染，
//!   故重跑不产生重复条目。
//! - **写入中途失败**：回执里只会出现已成功写入的路径，
//!   核对步骤会把这批路径如实记进 `IngestOutcome::written`，
//!   日志据此说明「已完成到哪」——本层不做全量回滚
//!   （一次 ingest 触及多页，全量回滚的成本高于重跑一次）。

use quill_adapters::{AdapterError, KnowledgeBackend, UserId};

use crate::backend::{ingest_context, split_receipt};
use crate::date::Date;
use crate::index::{IndexEntry, WikiIndex};
use crate::log::{LogEntry, LogOp, ParsedLog};
use crate::page::{render_page, Page};
use crate::store::{WikiError, WikiStore};

/// 摄入时喂给 LLM 的近期日志条数。
///
/// 规格 §四说 `log.md` 的用途是「给 LLM 提供最近做了什么���上下文」，
/// 并给出 `grep ... | tail -5`。**5 是规格给的数字**，不是随手取的。
pub const RECENT_LOG_ENTRIES: usize = 5;

/// schema 层规则的默认文件名。
pub const DEFAULT_SCHEMA_FILE: &str = "AGENTS.md";

/// 一次摄入的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestRequest {
    /// 待摄入来源在 raw 层的相对路径。
    pub source_rel: String,
    /// 日志日期（由调用方给 —— 本 crate 不读系统时间）。
    pub date: Date,
    /// 参与「相关页面」挑选的标题关键词；空 = 全部页面。
    pub focus: Vec<String>,
}

/// 一次摄入的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestOutcome {
    /// 经**回执核对**确认已落盘的页面路径（按回执顺序）。
    ///
    /// ⚠️ 只收**读得回来**的路径：一个声称写了 12 页却只落地 3 页的 backend，
    /// 在这里会得到 3，而不是一份好看的 12。
    pub written: Vec<String>,
    /// 重建后 `index.md` 的条目数。
    pub index_entries: usize,
    /// 追加到 `log.md` 的那一条。
    pub log_entry: Option<LogEntry>,
}

impl IngestOutcome {
    /// 摄入触及的页面数。
    ///
    /// 规格 §3.1 指出「一个来源可能触及 10-15 个 wiki 页面」，
    /// 故本数**不是**质量指标，只是给日志与 UI 用的观测量。
    pub fn touched_count(&self) -> usize {
        self.written.len()
    }
}

/// 执行一次摄入。
pub async fn ingest<B: KnowledgeBackend>(
    store: &WikiStore,
    backend: &B,
    user: UserId,
    req: &IngestRequest,
) -> Result<IngestOutcome, WikiError> {
    // 0. 布局先建好（幂等）—— 否则第一次摄入必然 Io(NotFound)。
    store.ensure_layout()?;

    // 1. 读来源。raw 层不可写，LLM 也只读它。
    let source_text = store.read_raw(&req.source_rel)?;

    // 2. 组装上下文。
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

    // 3. 让模型侧经**工具调用**写盘，收回一张回执。它碰不到文件系统句柄，
    //    写入路径由实现方持有的 WikiStore 承担（见模块文档「★」一节）。
    let receipt = backend
        .plan_ingest(user, ctx)
        .await
        .map_err(|e| ingest_backend_error(&e))?;
    let (touched, summary) = split_receipt(receipt);

    // 4. 核对回执：声称写了的页必须真的读得回来。
    //    ⚠️ 越权/非法路径**原样上抛**、不并进 Backend —— 那是安全事件，
    //    归错类会让人以为「只是模型没写成功」而放松警惕。
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

    // 5. 重建 index.md：全量重渲染而非增量补丁。
    //    理由：增量补丁在「页面被改名/删除」时会留下悬挂条目，
    //    而悬挂条目会让查询读到不存在的页面 —— 静默的错误答案。
    let pages_after = store.load_all_pages()?;
    let idx = build_index(&pages_after);
    outcome.index_entries = idx.len();
    store.write_index(&idx.render())?;

    // 6. 追加 log.md。
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

/// 全量重建索引（只收「类型 + 日期齐全」的页面）。
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

/// 挑「可能相关」的既有页面。
///
/// ⚠️ 判定完全确定（关键词命中 + 共享标签），
/// 目的是**给模型一个封闭的候选集**而不是整个 wiki ——
/// 整个 wiki 全量塞进上下文会让 ingest 的成本随页面数线性爆炸。
/// 选错的后果是模型少看了一页，而不是出错。
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
                // 无关键词时只按来源文件名粗筛，避免全量。
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

/// 读回刚写入的页面（供调用方校验 / 展示）。
pub fn reload(store: &WikiStore, rel: &str) -> Result<Page, WikiError> {
    store.read_page(rel)
}

/// 把一页渲染回 Markdown 原文（供外部工具使用）。
pub fn render(p: &Page) -> String {
    render_page(p)
}

/// 层的可写性说明（`raw` 只读）——给工具层生成权限表用。
pub fn writable_layers() -> [&'static str; 1] {
    [crate::store::DIR_WIKI]
}

/// schema 层文件名（供工具层暴露只读访问）。
pub fn schema_file() -> &'static str {
    DEFAULT_SCHEMA_FILE
}

/// 把 backend 错误转成本层错误，**保留可直接执行的修复建议**（铁律七）。
///
/// ⚠️ 归为 [`WikiError::Backend`] 而**不是** `Io`：
/// `Io` 会让上层以为「磁盘坏了，去查文件」，
/// 而真因在 provider 侧 —— 归错类会把人引向错误的排查方向。
fn ingest_backend_error(e: &AdapterError) -> WikiError {
    WikiError::Backend(format!(
        "模型侧摄入规划失败：{e}。请检查 provider 配置与凭据（用 `quill doctor` 诊断）"
    ))
}
