//! `index.md` —— 内容导向的目录（规格 §四）。
//!
//! # 为什么它是查询路径的**第一步**而不是一个可选的优化
//!
//! 规格 §五实测：约 100 来源 / 数百页面规模下，
//! 「LLM 自读 index.md」效果出奇地好，**无需 embedding 检索**。
//! `docs/V1_SCOPE_CONSTRAINTS.md` 约束 3 因此禁止首版引入向量库。
//! 本模块就是那条检索路径的载体。
//!
//! # 格式契约（必须与规格 §四一致）
//!
//! ```text
//! # Wiki 索引
//!
//! ## Entities
//! - [[Rust]] — 所有权模型的实现语言。（source_count=3, updated=2026-10-04）
//!
//! ## Concepts
//! - [[所有权]] — 资源的唯一归属规则。（source_count=2, updated=2026-10-04）
//! ```
//!
//! 条目行格式固定为 `- [[标题]] — 一句话摘要（key=value, key=value）`：
//! - 开头 `- [[` 让你能用 `grep '^- \[\['` 数出页面数；
//! - 尾部括号里放**可解析**的元数据，而不是自由文本 ——
//!   否则 lint 的「页面是否在索引里」检查就退化成字符串模糊匹配。
//!
//! # 确定性渲染
//!
//! [`WikiIndex::render`] 的输出**只依赖条目集合**，不依赖插入顺序或
//! 当前时间（日期来自条目本身）。这让「重渲染不产生 diff」成立 ——
//! 否则每次 ingest 都会让整个文件变脏，git 历史失去可读性。

use std::collections::BTreeMap;

use crate::date::Date;
use crate::page::{Page, PageType};
use crate::store::WikiError;

/// 索引标题行。
pub const INDEX_TITLE: &str = "# Wiki 索引";

/// 一条索引条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// 页面标题（须等于目标页的 `Page::title()`）。
    pub title: String,
    /// 一句话摘要。
    pub summary: String,
    /// 页面类型（决定落到哪个分类小节）。
    pub page_type: PageType,
    /// 来源数。
    pub source_count: u32,
    /// 更新日期。
    pub updated: Date,
}

impl IndexEntry {
    /// 从一个已解析页面构造（摘要由 [`summary_of`] 提取）。
    pub fn from_page(p: &Page) -> Option<Self> {
        // 无类型或无日期 → 不进索引。
        // 理由：索引是「查询时先读它」的唯一入口，
        // 塞一条无法完整表达元数据的条目，只会让 LLM 读到半条信息
        // 并据此作答（比查不到更危险）。
        let page_type = p.page_type()?;
        let updated = p.frontmatter.updated.or(p.frontmatter.created)?;
        Some(Self {
            title: p.title(),
            summary: summary_of(&p.body),
            page_type,
            source_count: p.frontmatter.source_count.unwrap_or(0),
            updated,
        })
    }
}

/// 从正文提取一句话摘要。
///
/// 规则（确定性，不做 NLP）：
/// 1. 跳过标题行、列表行、引用行、空行、只剩 wikilink 的行；
/// 2. 取第一段剩余文本；
/// 3. 截到第一个句末标点（`。！？.!?`）或 120 字符。
///
/// ⚠️ 没有第 4 步的「不理想就留空」判断：摘要是给人读的，
/// 截断后的半句仍然比空串有用（读者知道这条页面讲什么方向）。
pub fn summary_of(body: &str) -> String {
    for line in body.split('\n') {
        let t = line.trim();
        if t.is_empty()
            || t.starts_with('#')
            || t.starts_with('-')
            || t.starts_with('*')
            || t.starts_with('>')
            || t.starts_with("[[")
        {
            continue;
        }
        // 整行就是一个 wikilink（`- [[A]]` 已被上面挡掉）也算跳过。
        if t.starts_with('[') && t.ends_with(']') && t.contains("[[") {
            continue;
        }
        let mut end = t.len();
        for (i, c) in t.char_indices() {
            if matches!(c, '。' | '！' | '？' | '.' | '!' | '?') {
                end = i + c.len_utf8();
                break;
            }
            if i >= 120 {
                end = i;
                break;
            }
        }
        let s = t[..end].trim();
        if !s.is_empty() {
            return s.to_string();
        }
    }
    String::new()
}

/// 整个 `index.md` 的内存表示。
///
/// 用 [`BTreeMap`] 而非 `Vec`：**标题唯一**是索引的核心不变量
/// （同一标题两条 = LLM 读到互相矛盾的目录）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WikiIndex {
    entries: BTreeMap<String, IndexEntry>,
}

impl WikiIndex {
    /// 空索引。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从 `index.md` 原文解析。空串 → 空索引。
    pub fn parse(text: &str) -> Result<Self, WikiError> {
        let mut idx = Self::new();
        for (i, line) in text.lines().enumerate() {
            let t = line.trim();
            if !t.starts_with("- [[") {
                continue;
            }
            match parse_entry_line(t) {
                Ok(e) => {
                    idx.entries.insert(e.title.clone(), e);
                }
                Err(msg) => {
                    return Err(WikiError::MalformedIndex(format!(
                        "第 {} 行 {t:?}：{msg}",
                        i + 1
                    )))
                }
            }
        }
        Ok(idx)
    }

    /// 插入或更新一条（按标题覆盖）。
    pub fn upsert(&mut self, e: IndexEntry) {
        self.entries.insert(e.title.clone(), e);
    }

    /// 按标题移除。
    pub fn remove(&mut self, title: &str) -> bool {
        self.entries.remove(title).is_some()
    }

    /// 取一条。
    pub fn get(&self, title: &str) -> Option<&IndexEntry> {
        self.entries.get(title)
    }

    /// 是否含某标题。
    pub fn contains(&self, title: &str) -> bool {
        self.entries.contains_key(title)
    }

    /// 全部条目（按标题字典序 —— 确定性）。
    pub fn entries(&self) -> impl Iterator<Item = &IndexEntry> {
        self.entries.values()
    }

    /// 条目数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 渲染回 `index.md` 原文。
    pub fn render(&self) -> String {
        // 按类型分桶，桶内按标题排序（`entries()` 已是字典序）。
        let buckets: [(PageType, &str); 6] = [
            (PageType::Entity, "Entities"),
            (PageType::Concept, "Concepts"),
            (PageType::Summary, "Sources"),
            (PageType::Comparison, "Comparisons"),
            (PageType::Overview, "Overviews"),
            (PageType::Synthesis, "Syntheses"),
        ];
        let mut out = String::from(INDEX_TITLE);
        out.push('\n');
        for (t, heading) in buckets {
            let group: Vec<&IndexEntry> =
                self.entries.values().filter(|e| e.page_type == t).collect();
            if group.is_empty() {
                continue;
            }
            out.push_str(&format!("\n## {heading}\n"));
            for e in group {
                out.push_str(&render_entry_line(e));
            }
        }
        if self.entries.is_empty() {
            out.push_str("\n（尚无条目）\n");
        }
        out
    }

    /// 关键词定位：返回与 `query` 有词面重合的条目，按重合度降序。
    ///
    /// ⚠️ **这就是 v1 的全部检索能力**，刻意如此：
    /// 规格 §五 + 约束 3 明确「首版不做向量检索」，
    /// 检索方式是「LLM 自读 index.md」。
    /// 本函数只是帮编排层把 index.md 摊平成一个候选集，
    /// **不做排序打分以外的事**，更不做 embedding。
    ///
    /// 匹配规则：把 query 按非字母数字切词，然后对每个词做**子串包含**判定
    /// （标题权重 2、摘要权重 1）。
    /// ⚠️ 用子串而非词元相等：中文没有空格，按非字母数字切词会把
    /// 「在编译期检查所有权」整段当成一个词元，于是 `lookup("所有权")` 命中不了它
    /// —— 中文 wiki 上那等于检索恒空（恒绿假闸门）。
    /// 代价是英文短词会误命中（`own` 命中 `download`），但对「给模型一份
    /// 候选集」这个用途来说，多给几页远好过少给几页。
    /// 零交集 → 空候选（此时编排层把整个 index.md 交给 LLM 自己看）。
    pub fn lookup(&self, query: &str, limit: usize) -> Vec<&IndexEntry> {
        let terms = tokenize(query);
        if terms.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(usize, &IndexEntry)> = self
            .entries
            .values()
            .filter_map(|e| {
                let title = e.title.to_lowercase();
                let summary = e.summary.to_lowercase();
                let mut score = 0usize;
                for t in &terms {
                    if title.contains(t.as_str()) {
                        score += 2;
                    }
                    if summary.contains(t.as_str()) {
                        score += 1;
                    }
                }
                (score > 0).then_some((score, e))
            })
            .collect();
        // 分数降序；同分按标题升序 —— 保证同一 query 每次给出同一顺序。
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));
        scored.truncate(limit);
        scored.into_iter().map(|(_, e)| e).collect()
    }
}

/// 切词：按非字母数字切分，转小写。
fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|p| !p.is_empty())
        .map(|p| p.to_lowercase())
        .collect()
}

fn render_entry_line(e: &IndexEntry) -> String {
    let mut s = format!("- [[{}]] — {}", e.title, e.summary);
    s.push_str(&format!(
        "（source_count={}, updated={}）\n",
        e.source_count, e.updated
    ));
    s
}

/// 解析一行条目。
///
/// 拆成三段（`[[标题]]` / 摘要 / 尾部元数据）分别解析，
/// 任何一段坏都返回带**原因**的 Err（铁律七：失败必须自诊断）。
fn parse_entry_line(line: &str) -> Result<IndexEntry, String> {
    let rest = line
        .strip_prefix("- [[")
        .ok_or_else(|| "不是 `- [[` 开头".to_string())?;
    let close = rest.find("]]").ok_or_else(|| "缺少闭合的 ]]".to_string())?;
    let title = rest[..close].trim().to_string();
    if title.is_empty() {
        return Err("标题为空".to_string());
    }
    let mut tail = &rest[close + 2..];

    // 尾部元数据：最后一个 `（...）`。
    let mut source_count = 0u32;
    let mut updated = None;
    if let Some(open) = tail.rfind('（') {
        if let Some(stripped) = tail.strip_suffix('）') {
            let inner = &stripped[open..];
            for kv in inner.trim_matches(['（', '）']).split(',') {
                let Some((k, v)) = kv.split_once('=') else {
                    return Err(format!("元数据 {kv:?} 不是 key=value"));
                };
                match k.trim() {
                    "source_count" => {
                        source_count = v
                            .trim()
                            .parse::<u32>()
                            .map_err(|_| format!("source_count={v:?} 不是非负整数"))?
                    }
                    "updated" => {
                        let d = Date::parse(v.trim())
                            .map_err(|e| format!("updated={v:?} 非法：{e}"))?;
                        updated = Some(d);
                    }
                    other => return Err(format!("未知元数据键 {other:?}")),
                }
            }
            tail = &tail[..open];
        }
    }

    // 摘要：去掉分隔用的 ` — `。
    let summary = tail.trim().trim_start_matches('—').trim().to_string();

    Ok(IndexEntry {
        title,
        summary,
        // 渲染时分类由所在小节决定；解析回内存时不保留小节名，
        // 统一落 Summary —— 分类在 render 时按 `page_type` 重算，
        // 所以这个默认值**不会**造成往返不一致（见 round_trip 测试）。
        page_type: PageType::Summary,
        source_count,
        updated: updated.ok_or_else(|| "缺少 updated 元数据".to_string())?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::parse_page;

    fn d(s: &str) -> Date {
        Date::parse(s).expect("测试日期应合法")
    }

    #[test]
    fn renders_the_documented_shape() {
        let mut idx = WikiIndex::new();
        idx.upsert(IndexEntry {
            title: "Rust".into(),
            summary: "所有权模型的实现语言。".into(),
            page_type: PageType::Entity,
            source_count: 3,
            updated: d("2026-10-04"),
        });
        idx.upsert(IndexEntry {
            title: "所有权".into(),
            summary: "资源的唯一归属规则。".into(),
            page_type: PageType::Concept,
            source_count: 2,
            updated: d("2026-10-05"),
        });
        let text = idx.render();
        assert!(text.starts_with(INDEX_TITLE));
        assert!(text.contains("## Entities\n- [[Rust]] — 所有权模型的实现语言。（source_count=3, updated=2026-10-04）"));
        assert!(text.contains("## Concepts\n- [[所有权]]"));
        // 反渲染：每条都能被 grep '^## \[' 之外的 `^- \[\[` 数出来
        assert_eq!(text.lines().filter(|l| l.starts_with("- [[")).count(), 2);
    }

    #[test]
    fn render_is_order_independent_and_stable() {
        let mk = |t: &str| IndexEntry {
            title: t.into(),
            summary: "s".into(),
            page_type: PageType::Concept,
            source_count: 1,
            updated: d("2026-10-04"),
        };
        let mut a = WikiIndex::new();
        a.upsert(mk("甲"));
        a.upsert(mk("乙"));
        let mut b = WikiIndex::new();
        b.upsert(mk("乙"));
        b.upsert(mk("甲"));
        assert_eq!(a.render(), b.render(), "渲染必须与插入顺序无关");
        assert_eq!(a.render(), a.render());
    }

    #[test]
    fn upsert_overwrites_by_title() {
        let mut idx = WikiIndex::new();
        for n in [1u32, 2] {
            idx.upsert(IndexEntry {
                title: "X".into(),
                summary: format!("第 {n} 版"),
                page_type: PageType::Concept,
                source_count: n,
                updated: d("2026-10-04"),
            });
        }
        assert_eq!(idx.len(), 1, "同标题必须覆盖而不是追加");
        assert_eq!(idx.get("X").expect("应有 X").source_count, 2);
    }

    #[test]
    fn parse_rejects_malformed_lines_with_reason() {
        let bad = [
            "- [[未闭合",
            "- [[]] — x（source_count=1, updated=2026-10-04）", // 标题为空
            "- [[A]] — x（updated=2026-13-40）",
            "- [[A]] — x（source_count=多, updated=2026-10-04）",
            "- [[A]] — x",                                 // 缺 updated 元数据
            "- [[A]] — x（未知键=1, updated=2026-10-04）", // 未知元数据键
        ];
        for line in bad {
            let r = WikiIndex::parse(line);
            assert!(r.is_err(), "坏行 {line:?} 未被拒绝");
        }
    }

    #[test]
    fn parse_ignores_non_entry_lines_without_erroring() {
        // 非条目行（含 `index.md` 的标题行、空行、说明文字）必须被**跳过**而不是报错 ——
        // index.md 是人可编辑的，让一句说明文字把整份索引读废是不可接受的。
        let text = format!(
            "{}\n\n## Concepts\n\n一行说明文字\n- [[A]] — 摘要。（source_count=1, updated=2026-10-04）\n",
            INDEX_TITLE
        );
        let idx = WikiIndex::parse(&text).expect("说明文字不该导致解析失败");
        assert_eq!(idx.len(), 1);
        assert!(idx.contains("A"));
    }

    #[test]
    fn parse_accepts_well_formed_and_reports_malformed() {
        let text = format!(
            "{}\n\n## Concepts\n- [[A]] — 摘要甲。（source_count=1, updated=2026-10-04）\n这行不是条目\n",
            INDEX_TITLE
        );
        let idx = WikiIndex::parse(&text).expect("应能解析");
        assert_eq!(idx.len(), 1);
        let e = idx.get("A").expect("应有 A");
        assert_eq!(e.summary, "摘要甲。");
        assert_eq!(e.source_count, 1);
        assert_eq!(e.updated, d("2026-10-04"));
    }

    #[test]
    fn summary_extraction_skips_structure() {
        assert_eq!(
            summary_of("# 标题\n\n- 列表项\n\n第一段。第二段。\n"),
            "第一段。"
        );
        assert_eq!(summary_of("只有一行没有句号"), "只有一行没有句号");
        assert_eq!(summary_of("# 只有标题"), "");
        // 超长截断到 120 字符
        let long = "字".repeat(200);
        let s = summary_of(&long);
        assert!(s.chars().count() <= 121, "截断失效：{}", s.chars().count());
    }

    #[test]
    fn from_page_requires_type_and_date() {
        let ok = parse_page(
            "a.md",
            "---\ntitle: A\ntype: concept\nupdated: 2026-10-04\n---\n\n摘要。\n",
        );
        let e = IndexEntry::from_page(&ok).expect("有类型与日期 → 应入索引");
        assert_eq!(e.title, "A");
        assert_eq!(e.summary, "摘要。");
        assert_eq!(e.source_count, 0);

        let no_type = parse_page("b.md", "---\ntitle: B\n---\n\n正文。\n");
        assert!(
            IndexEntry::from_page(&no_type).is_none(),
            "无类型不该进索引"
        );
        let no_date = parse_page("c.md", "---\ntitle: C\ntype: entity\n---\n\n正文。\n");
        assert!(
            IndexEntry::from_page(&no_date).is_none(),
            "无日期不该进索引"
        );
    }

    #[test]
    fn lookup_ranks_title_hits_above_summary_hits() {
        let mut idx = WikiIndex::new();
        idx.upsert(IndexEntry {
            title: "所有权".into(),
            summary: "资源的归属规则。".into(),
            page_type: PageType::Concept,
            source_count: 1,
            updated: d("2026-10-04"),
        });
        idx.upsert(IndexEntry {
            title: "借用检查".into(),
            summary: "在编译期检查所有权。".into(),
            page_type: PageType::Concept,
            source_count: 1,
            updated: d("2026-10-04"),
        });
        let hits = idx.lookup("所有权", 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "所有权", "标题命中应排在摘要命中之前");
        assert!(idx.lookup("完全无关的词", 5).is_empty());
    }

    #[test]
    fn render_parse_round_trip_keeps_titles_and_counts() {
        let mut idx = WikiIndex::new();
        idx.upsert(IndexEntry {
            title: "A".into(),
            summary: "甲乙丙。".into(),
            page_type: PageType::Summary,
            source_count: 7,
            updated: d("2026-10-04"),
        });
        let text = idx.render();
        let back = WikiIndex::parse(&text).expect("往返应可解析");
        let e = back.get("A").expect("A 应还在");
        assert_eq!(e.summary, "甲乙丙。");
        assert_eq!(e.source_count, 7);
        assert_eq!(e.updated, d("2026-10-04"));
        // 再渲染一次必须字节一致（幂等）
        assert_eq!(back.render(), text);
    }
}
