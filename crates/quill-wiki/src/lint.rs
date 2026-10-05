//! 体检规则引擎（规格 §三 3.3 Lint）。
//!
//! # v1 覆盖范围与**不覆盖**的范围
//!
//! 规格 §3.3 列了 6 项。逐项交代本版是否覆盖、以及为什么：
//!
//! | 规格项 | v1 | 载体 |
//! |---|---|---|
//! | 无入链的孤儿页面 | ✅ | [`Rule::OrphanPage`] |
//! | 缺失的交叉引用 | ✅ | [`Rule::BrokenLink`] + [`Rule::MissingCrossReference`] |
//! | 页面间矛盾 | ✅（可计算的部分） | [`Rule::Contradiction`] |
//! | 被新来源取代的过时断言 | ✅（可计算的部分） | [`Rule::Contradiction`] 的 `updated` 方向 |
//! | 被提及但缺页面的重要概念 | ✅ | [`Rule::BrokenLink`]（未解析链接 = 缺页面） |
//! | 可通过搜索填补的数据空白 | ⚠️ 部分 | [`Rule::MissingSourceTrace`] |
//!
//! ## 「矛盾」为什么能算出来，而不用 LLM
//!
//! 语义矛盾（「A 页说 X 成立，B 页说 X 不成立」）需要推理，不是 grep。
//! 但 v1 能算的是**结构矛盾**，它们同样是真问题且不需要模型：
//!
//! 1. **同一主体两处定义**（`Contradiction` / `DuplicateSubject`）——
//!    两个页面的标题规范化（忽略大小写、空格、连字符）后相同。
//!    这是矛盾的**根源**：读者/LLM 读到两份定义，无从判断哪个有效。
//! 2. **同一断言键给出不同值**（`Contradiction` / `ConflictingAssertion`）——
//!    两页正文里有 `键: 值` 形态的断言行，同键不同值。
//! 3. **可溯源性被破坏**（`MissingSourceTrace`）——
//!    规格 §6.4 约束 3「每个页面必须可溯源」。若 `source_count: 3`
//!    却一条 `../raw/` 引用都没有，两种解释（引用被手工删了 / 计数写错了）
//!    都会让读者拿不到原始文档。
//!
//! ⚠️ **明确不覆盖**：同一事实在两页里用**不同措辞**表达出的矛盾。
//! 那需要 LLM 逐对比对，成本是 lint 的主要开销（规格 §九）。
//! 本模块**不假装**能做这件事 —— 上表「可计算的部分」就是这个边界。
//! 要扩展它，正确做法是调 [`quill_adapters::KnowledgeBackend::lint_semantics`]
//! 让模型补一轮语义比对，而不是在规则里堆启发式。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::graph::LinkGraph;
use crate::index::WikiIndex;
use crate::page::{Page, PageType};

/// 触发「缺失交叉引用」的**最少**共同标签数。
///
/// 1 会把「都打了 rust 标签」的两页判成缺引用（噪声）；
/// 3 在标签稀疏的 wiki 里几乎永不触发（漏报）。
/// 2 是本 wiki 实际标签密度下的平衡点，**可被 [`LintConfig`] 覆盖**。
pub const DEFAULT_MIN_SHARED_TAGS: usize = 2;

/// 规则 ID（`src` 是产出它的解析告警，见 [`Rule::BadFrontmatter`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    /// 无入链的孤儿页面。
    OrphanPage,
    /// 链接指向不存在的页面（= 被提及但缺页面）。
    BrokenLink,
    /// 共享标签却互不链接的页面（= 缺失的交叉引用）。
    MissingCrossReference,
    /// 页面间矛盾。
    Contradiction,
    /// 声明了来源却无法溯源。
    MissingSourceTrace,
    /// frontmatter 降级（解析告警）。
    BadFrontmatter,
    /// 有页面没进 `index.md`。
    MissingIndexEntry,
    /// 页面重名（`index.md` 会给读者两条同名条目）。
    DuplicateTitle,
    /// 页面缺少 `type`（六种之一）。
    MissingPageType,
}

impl Rule {
    /// 稳定 ID（用于报告与测试断言，**不得随文案改动**）。
    pub const fn id(self) -> &'static str {
        match self {
            Self::OrphanPage => "orphan-page",
            Self::BrokenLink => "broken-link",
            Self::MissingCrossReference => "missing-cross-reference",
            Self::Contradiction => "contradiction",
            Self::MissingSourceTrace => "missing-source-trace",
            Self::BadFrontmatter => "bad-frontmatter",
            Self::MissingIndexEntry => "missing-index-entry",
            Self::DuplicateTitle => "duplicate-title",
            Self::MissingPageType => "missing-page-type",
        }
    }

    /// 全部规则（供 runner 遍历与覆盖率自检）。
    pub const ALL: [Rule; 9] = [
        Self::OrphanPage,
        Self::BrokenLink,
        Self::MissingCrossReference,
        Self::Contradiction,
        Self::MissingSourceTrace,
        Self::BadFrontmatter,
        Self::MissingIndexEntry,
        Self::DuplicateTitle,
        Self::MissingPageType,
    ];
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// 矛盾的具体种类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contradiction {
    /// 同一主体两处定义（标题规范化后相同）。
    DuplicateSubject { first: String, second: String },
    /// 同一断言键给出不同值。
    ConflictingAssertion {
        first_page: String,
        second_page: String,
        key: String,
        first_value: String,
        second_value: String,
        /// 断言较旧的一侧 —— 即「被新来源取代的过时断言」。
        stale_page: String,
        fresh_page: String,
    },
}

/// 一条发现。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// 触发的规则。
    pub rule: Rule,
    /// 相关页面路径。
    pub pages: Vec<String>,
    /// 人类可读说明（中文，含**该做什么**，铁律七）。
    pub message: String,
    /// 矛盾详情（非矛盾规则为 `None`）。
    pub contradiction: Option<Contradiction>,
}

impl Finding {
    fn new(rule: Rule, pages: Vec<String>, message: String) -> Self {
        Self {
            rule,
            pages,
            message,
            contradiction: None,
        }
    }

    fn contradiction(pages: Vec<String>, message: String, detail: Contradiction) -> Self {
        Self {
            rule: Rule::Contradiction,
            pages,
            message,
            contradiction: Some(detail),
        }
    }
}

/// lint 配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintConfig {
    /// 触发「缺失交叉引用」的最少共同标签数。
    pub min_shared_tags: usize,
    /// 是否把重名页面报为矛盾。
    pub detect_duplicate_subject: bool,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            min_shared_tags: DEFAULT_MIN_SHARED_TAGS,
            detect_duplicate_subject: true,
        }
    }
}

/// 覆盖统计。
///
/// ⚠️ **存在的唯一理由是铁律十六**：「已检查 0 条」与
/// 「检查了 8 条、0 条有问题」在屏幕上必须能区分。
/// 尤其 [`LintReport::assertions_checked`] —— 矛盾检测读的是
/// `键: 值` 断言行；一个断言行都没有的 wiki 会得到 0 条矛盾，
/// 而那**不是**「没有矛盾」，是「没东西可比」。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LintStats {
    /// 参与体检的知识页面数（不含 `index.md` / `log.md`）。
    pub pages_checked: usize,
    /// 检查过的链接数（出边 + 未解析）。
    pub links_checked: usize,
    /// 检查过的 `键: 值` 断言行数。
    pub assertions_checked: usize,
    /// 参与两两比对的页面对数。
    pub page_pairs_checked: usize,
    /// 结构文件是否读到（`index.md`）。
    pub index_loaded: bool,
}

/// 体检报告。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LintReport {
    /// 发现，按 (规则, 页面) 排序 —— 排序保证同一 wiki 的报告可比对。
    pub findings: Vec<Finding>,
    /// 覆盖统计。
    pub stats: LintStats,
}

impl LintReport {
    /// 某条规则的发现数。
    pub fn count(&self, rule: Rule) -> usize {
        self.findings.iter().filter(|f| f.rule == rule).count()
    }

    /// 是否有「有问题」。
    ///
    /// ⚠️ **只回答「有没有问题」，不回答「检查充分吗」** ——
    /// 后者看 [`LintReport::stats`]（铁律十六）。
    pub fn has_problems(&self) -> bool {
        !self.findings.is_empty()
    }

    /// 一行摘要（可直接进 `log.md`）。
    pub fn summary(&self) -> String {
        format!(
            "已检查 {} 页 / {} 链接 / {} 断言行 / {} 页对（index.md {}），发现 {} 条",
            self.stats.pages_checked,
            self.stats.links_checked,
            self.stats.assertions_checked,
            self.stats.page_pairs_checked,
            if self.stats.index_loaded {
                "已载入"
            } else {
                "未载入"
            },
            self.findings.len()
        )
    }

    /// 渲染成 `log.md` 正文。
    pub fn render(&self) -> String {
        let mut s = self.summary();
        for f in &self.findings {
            s.push_str(&format!(
                "\n- [{}] {}（{}）",
                f.rule.id(),
                f.message,
                f.pages.join(", ")
            ));
        }
        s
    }
}

/// 对一批页面跑体检。
///
/// `index` 传 `None` 表示**没有** `index.md`（首次运行）。
/// ⚠️ 传 `None` 不等于「索引里没有遗漏」—— 那时
/// [`Rule::MissingIndexEntry`] 无法判定，报告里会体现为
/// `index_loaded = false` 且不产生该规则发现。**这个区别必须显眼**
/// （铁律十六），不能让人误以为索引是完整的。
pub fn lint(pages: &[Page], index: Option<&WikiIndex>, cfg: &LintConfig) -> LintReport {
    let knowledge: Vec<&Page> = pages.iter().filter(|p| !is_structural(&p.path)).collect();
    let graph = LinkGraph::build(&knowledge.iter().map(|p| (*p).clone()).collect::<Vec<_>>());

    let mut findings: Vec<Finding> = Vec::new();
    let mut stats = LintStats {
        pages_checked: knowledge.len(),
        index_loaded: index.is_some(),
        ..LintStats::default()
    };

    // ── 1. 孤儿页面 ──
    for n in graph.orphans() {
        findings.push(Finding::new(
            Rule::OrphanPage,
            vec![n.path.clone()],
            format!(
                "孤儿页面「{}」没有任何其他页面的入链。请在其他相关页面加 `[[{}]]`，或确认它是否该并入别页。",
                n.title, n.title
            ),
        ));
    }

    // ── 2. 断链（= 被提及但缺页面）──
    for u in graph.unresolved_links() {
        findings.push(Finding::new(
            Rule::BrokenLink,
            vec![u.from.clone()],
            format!(
                "「{}」链到 [[{}]]，但没有这一页。请创建该页面，或修正链接。",
                u.from, u.target
            ),
        ));
    }

    // ── 3. 缺失的交叉引用 ──
    let pairs = graph.unlinked_tag_pairs(cfg.min_shared_tags);
    stats.page_pairs_checked = knowledge
        .len()
        .saturating_mul(knowledge.len().saturating_sub(1))
        / 2;
    for (a, b, tags) in &pairs {
        findings.push(Finding::new(
            Rule::MissingCrossReference,
            vec![a.clone(), b.clone()],
            format!(
                "「{a}」与「{b}」共享标签 [{}] 却互不链接。请在其中一页加一条 `[[]]`。",
                tags.join(", ")
            ),
        ));
    }

    // ── 4. 页面间矛盾 ──
    stats.assertions_checked = 0;
    if cfg.detect_duplicate_subject {
        findings.extend(duplicate_subject_findings(&knowledge));
    }
    findings.extend(conflicting_assertion_findings(&knowledge, &mut stats));

    // ── 5. 可溯源性 ──
    for p in &knowledge {
        if p.frontmatter.source_count.unwrap_or(0) > 0 && !has_raw_reference(&p.body) {
            findings.push(Finding::new(
                Rule::MissingSourceTrace,
                vec![p.path.clone()],
                format!(
                    "「{}」声明 source_count={}，但正文没有任何 `../raw/` 引用 —— 无法溯源。",
                    p.path,
                    p.frontmatter.source_count.unwrap_or(0)
                ),
            ));
        }
    }

    // ── 6. 解析降级 ──
    for p in &knowledge {
        for w in &p.warnings {
            findings.push(Finding::new(
                Rule::BadFrontmatter,
                vec![p.path.clone()],
                format!("「{}」解析降级：{w}", p.path),
            ));
        }
    }

    // ── 7. 缺页面类型 ──
    for p in &knowledge {
        if p.page_type().is_none() {
            findings.push(Finding::new(
                Rule::MissingPageType,
                vec![p.path.clone()],
                format!(
                    "「{}」没有合法的 type（六种之一：{}）。",
                    p.path,
                    PageType::ALL
                        .iter()
                        .map(|t| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                ),
            ));
        }
    }

    // ── 8. 索引遗漏 + 重名 ──
    if let Some(idx) = index {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for p in &knowledge {
            let t = p.title();
            if seen.insert(t.clone()) && !idx.contains(&t) {
                findings.push(Finding::new(
                    Rule::MissingIndexEntry,
                    vec![p.path.clone()],
                    format!(
                        "「{t}」不在 index.md 里。查询时 LLM 先读索引，缺条目等于这一页查不到。"
                    ),
                ));
            }
        }
    }

    findings.sort_by(|a, b| {
        a.rule
            .id()
            .cmp(b.rule.id())
            .then_with(|| a.pages.cmp(&b.pages))
            .then_with(|| a.message.cmp(&b.message))
    });
    findings.dedup();
    stats.links_checked = graph
        .nodes()
        .map(|n| n.out.len() + n.unresolved.len())
        .sum();

    LintReport { findings, stats }
}

fn is_structural(path: &str) -> bool {
    crate::graph::STRUCTURAL_FILES.contains(&path)
}

/// 标题规范化：忽略大小写、空格、连字符、下划线。
///
/// 目的是让 `Rust 所有权` / `rust-所有权` / `RUST所有权` 被认作同一主体。
fn normalize_subject(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn duplicate_subject_findings(pages: &[&Page]) -> Vec<Finding> {
    let mut first_by_norm: BTreeMap<String, &Page> = BTreeMap::new();
    let mut out = Vec::new();
    for p in pages {
        let norm = normalize_subject(&p.title());
        match first_by_norm.get(&norm) {
            Some(prev) => out.push(Finding::contradiction(
                vec![prev.path.clone(), p.path.clone()],
                format!(
                    "「{}」与「{}」标题规范化后相同（{}），是同一主体的两处定义 —— 读者无从判断哪份有效。请合并，或改标题。",
                    prev.path,
                    p.path,
                    norm
                ),
                Contradiction::DuplicateSubject {
                    first: prev.path.clone(),
                    second: p.path.clone(),
                },
            )),
            None => {
                first_by_norm.insert(norm, p);
            }
        }
    }
    out
}

/// 抽取 `键: 值` 形态的断言行。
///
/// 只认「以 `- ` 开头的列表项，且整行恰好一个 `:`」
/// —— 全角冒号也算。这样约束了断言的书写形态，
/// 是本 wiki 的**约定**（写在 schema 层，LLM 按它写）。
///
/// ⚠️ 因此断言数为 0 的 wiki **得不到**矛盾结论；
/// 这一点由 [`LintStats::assertions_checked`] 显式暴露，不假装覆盖。
fn extract_assertions(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in body.split('\n') {
        let t = line.trim();
        let Some(item) = t.strip_prefix("- ") else {
            continue;
        };
        let item = item.trim();
        // 恰好一个分隔符：`键: 值`
        let Some((k, v)) = item.split_once([':', '：']) else {
            continue;
        };
        if v.contains(':') || v.contains('：') {
            continue;
        }
        let (k, v) = (k.trim(), v.trim());
        if k.is_empty() || v.is_empty() || k.chars().count() > 24 {
            continue;
        }
        out.push((k.to_string(), v.to_string()));
    }
    out
}

/// 值归一化（比较用）：忽略大小写与首尾标点。
fn normalize_value(v: &str) -> String {
    v.trim()
        .trim_end_matches(['。', '.', '，', ',', '；', ';'])
        .trim()
        .to_lowercase()
}

fn conflicting_assertion_findings(pages: &[&Page], stats: &mut LintStats) -> Vec<Finding> {
    // 键 → (归一化值, 页面, 原值)
    let mut by_key: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    let mut updated_of: BTreeMap<String, String> = BTreeMap::new();
    for p in pages {
        if let Some(d) = p.frontmatter.updated.or(p.frontmatter.created) {
            updated_of.insert(p.path.clone(), d.to_string());
        }
        for (k, v) in extract_assertions(&p.body) {
            stats.assertions_checked += 1;
            by_key
                .entry(k)
                .or_default()
                .push((normalize_value(&v), p.path.clone(), v));
        }
    }

    let mut out = Vec::new();
    for (key, vals) in by_key {
        if vals.len() < 2 {
            continue;
        }
        // 同键下不同归一化值 → 矛盾。取第一对不同的即可，避免 O(k²) 噪声。
        let base = &vals[0];
        for other in &vals[1..] {
            if other.0 == base.0 {
                continue;
            }
            // 较旧的一侧 = 可能被新来源取代的过时断言
            let (stale, fresh) = match (
                updated_of.get(&base.1).map(|s| s.as_str()),
                updated_of.get(&other.1).map(|s| s.as_str()),
            ) {
                (Some(a), Some(b)) if a <= b => (&base.1, &other.1),
                _ => (&other.1, &base.1),
            };
            out.push(Finding::contradiction(
                vec![base.1.clone(), other.1.clone()],
                format!(
                    "断言「{key}」在 {} 是「{}」，在 {} 却是「{}」。请核对来源：较旧的 {stale} 可能已被取代。",
                    base.1, base.2, other.1, other.2
                ),
                Contradiction::ConflictingAssertion {
                    first_page: base.1.clone(),
                    second_page: other.1.clone(),
                    key: key.clone(),
                    first_value: base.2.clone(),
                    second_value: other.2.clone(),
                    stale_page: stale.clone(),
                    fresh_page: fresh.clone(),
                },
            ));
            break;
        }
    }
    out
}

fn has_raw_reference(body: &str) -> bool {
    body.contains("../raw/") || body.contains("(../raw")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::parse_page;

    fn p(path: &str, text: &str) -> Page {
        parse_page(path, text)
    }

    fn concept(path: &str, title: &str, body: &str, tags: &str) -> Page {
        p(
            path,
            &format!(
                "---\ntitle: {title}\ntype: concept\ntags: [{tags}]\ncreated: 2026-10-01\nupdated: 2026-10-04\n---\n\n{body}\n"
            ),
        )
    }

    fn clean_pages() -> Vec<Page> {
        // 一份「无任何问题」的 wiki：必须零发现（不误报是硬要求）
        vec![
            concept(
                "a.md",
                "A",
                "摘要甲。\n\n见 [[B]]\n\n- 来源甲: [x](../raw/x.pdf)",
                "甲, 共同",
            ),
            concept(
                "b.md",
                "B",
                "摘要乙。\n\n见 [[A]]\n\n- 来源乙: [y](../raw/y.pdf)",
                "乙, 共同",
            ),
        ]
    }

    fn index_of(pages: &[Page]) -> WikiIndex {
        let mut idx = WikiIndex::new();
        for pg in pages {
            if let Some(e) = crate::index::IndexEntry::from_page(pg) {
                idx.upsert(e);
            }
        }
        idx
    }

    #[test]
    fn clean_wiki_reports_nothing() {
        let pages = clean_pages();
        let idx = index_of(&pages);
        let r = lint(&pages, Some(&idx), &LintConfig::default());
        assert!(r.findings.is_empty(), "干净 wiki 误报了：{:#?}", r.findings);
        assert!(!r.has_problems());
    }

    #[test]
    fn stats_distinguish_zero_problems_from_nothing_checked() {
        let pages = clean_pages();
        let idx = index_of(&pages);
        let r = lint(&pages, Some(&idx), &LintConfig::default());
        // 铁律十六：必须能区分「检查了 N 条，0 条有问题」与「没检查」
        assert_eq!(r.stats.pages_checked, 2);
        assert_eq!(r.stats.links_checked, 2);
        assert!(r.stats.index_loaded);
        assert!(r.summary().contains("已检查 2 页"));
        // 断言行数必须被显式报出
        assert!(
            r.summary().contains("断言行"),
            "摘要须显式含断言行数：{}",
            r.summary()
        );
    }

    #[test]
    fn detects_orphan_page() {
        let pages = vec![
            concept("a.md", "A", "见 [[B]]", "x"),
            concept("b.md", "B", "见 [[A]]", "x"),
            concept("lonely.md", "没人链我", "正文", "y"),
        ];
        let idx = index_of(&pages);
        let r = lint(&pages, Some(&idx), &LintConfig::default());
        let orphans: Vec<&Finding> = r
            .findings
            .iter()
            .filter(|f| f.rule == Rule::OrphanPage)
            .collect();
        assert_eq!(orphans.len(), 1, "应恰好报一个孤儿：{:#?}", r.findings);
        assert_eq!(orphans[0].pages, vec!["lonely.md".to_string()]);
    }

    #[test]
    fn index_md_page_is_never_an_orphan() {
        // index.md 自身没入链；若被当孤儿，lint 每次都会报一条假问题
        let mut pages = clean_pages();
        pages.push(p("index.md", "---\ntitle: 索引\n---\n\n- [[A]]\n- [[B]]\n"));
        let r = lint(&pages, None, &LintConfig::default());
        assert_eq!(r.count(Rule::OrphanPage), 0, "{:#?}", r.findings);
        assert_eq!(r.stats.pages_checked, 2, "index.md 不该计入被检查页数");
    }

    #[test]
    fn detects_broken_link() {
        let pages = vec![concept("a.md", "A", "见 [[不存在的页]]", "x")];
        let r = lint(&pages, None, &LintConfig::default());
        assert_eq!(r.count(Rule::BrokenLink), 1, "{:#?}", r.findings);
        let f = r
            .findings
            .iter()
            .find(|f| f.rule == Rule::BrokenLink)
            .unwrap();
        assert!(f.message.contains("不存在的页"));
    }

    #[test]
    fn detects_missing_cross_reference_by_shared_tags() {
        let pages = vec![
            concept("a.md", "A", "正文", "内存, 安全"),
            concept("b.md", "B", "正文", "内存, 安全"),
        ];
        let r = lint(&pages, None, &LintConfig::default());
        assert_eq!(
            r.count(Rule::MissingCrossReference),
            1,
            "共享两个标签却互不链接应报一次：{:#?}",
            r.findings
        );
        // 只共享一个标签 → 不报（不误报）
        let one_tag = vec![
            concept("c.md", "C", "正文", "内存"),
            concept("d.md", "D", "正文", "安全"),
        ];
        assert_eq!(
            lint(&one_tag, None, &LintConfig::default()).count(Rule::MissingCrossReference),
            0
        );
    }

    #[test]
    fn detects_duplicate_subject_as_contradiction() {
        let pages = vec![
            concept("a.md", "Rust 所有权", "正文", "x"),
            concept("b.md", "rust-所有权", "正文", "x"),
        ];
        let r = lint(&pages, None, &LintConfig::default());
        let c: Vec<&Finding> = r
            .findings
            .iter()
            .filter(|f| f.rule == Rule::Contradiction)
            .collect();
        assert_eq!(c.len(), 1, "{:#?}", r.findings);
        assert!(matches!(
            c[0].contradiction,
            Some(Contradiction::DuplicateSubject { .. })
        ));
    }

    #[test]
    fn detects_conflicting_assertion_and_names_the_stale_page() {
        let pages = vec![
            p(
                "old.md",
                "---\ntitle: 旧页\ntype: entity\ncreated: 2026-10-01\nupdated: 2026-10-01\n---\n\n- 上线年份: 2024\n",
            ),
            p(
                "new.md",
                "---\ntitle: 新页\ntype: entity\ncreated: 2026-10-04\nupdated: 2026-10-04\n---\n\n- 上线年份: 2026\n",
            ),
        ];
        let r = lint(&pages, None, &LintConfig::default());
        let c: Vec<&Finding> = r
            .findings
            .iter()
            .filter(|f| f.rule == Rule::Contradiction)
            .collect();
        assert_eq!(c.len(), 1, "{:#?}", r.findings);
        match c[0].contradiction.as_ref().expect("矛盾详情") {
            Contradiction::ConflictingAssertion {
                key,
                first_value,
                second_value,
                stale_page,
                ..
            } => {
                assert_eq!(key, "上线年份");
                assert_eq!(
                    [first_value.as_str(), second_value.as_str()],
                    ["2024", "2026"]
                );
                assert_eq!(stale_page, "old.md", "过期的一侧必须是 updated 更小的");
            }
            other => panic!("矛盾种类不对：{other:?}"),
        }
        assert_eq!(r.stats.assertions_checked, 2, "两条断言都必须被计入");
    }

    #[test]
    fn same_assertion_value_is_not_a_contradiction() {
        // 归一化后相同（尾部句号不算差异）→ 不是矛盾。
        // 反例见 conflicting_assertion_and_names_the_stale_page。
        let pages = vec![
            p(
                "a.md",
                "---\ntitle: 甲\ntype: entity\nupdated: 2026-10-01\n---\n\n- 颜色: 蓝\n",
            ),
            p(
                "b.md",
                "---\ntitle: 乙\ntype: entity\nupdated: 2026-10-04\n---\n\n- 颜色: 蓝。\n",
            ),
        ];
        let r = lint(&pages, None, &LintConfig::default());
        assert_eq!(r.count(Rule::Contradiction), 0, "{:#?}", r.findings);
        assert_eq!(r.stats.assertions_checked, 2);
    }

    #[test]
    fn zero_assertions_is_visible_not_silently_clean() {
        // 没有断言行的 wiki：矛盾数为 0，但检查数也必须是 0（不是「查过没问题」）
        let pages = vec![
            concept("a.md", "A", "没有断言行", "x"),
            concept("b.md", "B", "见 [[A]]", "y"),
        ];
        let r = lint(&pages, None, &LintConfig::default());
        assert_eq!(r.count(Rule::Contradiction), 0);
        assert_eq!(r.stats.assertions_checked, 0, "0 条断言必须显示为 0");
    }

    #[test]
    fn detects_missing_source_trace() {
        let pages = vec![p(
            "a.md",
            "---\ntitle: A\ntype: concept\nupdated: 2026-10-04\nsource_count: 3\n---\n\n正文没有任何来源引用。\n",
        )];
        let r = lint(&pages, None, &LintConfig::default());
        assert_eq!(r.count(Rule::MissingSourceTrace), 1, "{:#?}", r.findings);
    }

    #[test]
    fn reports_bad_frontmatter_and_missing_type() {
        let pages = vec![p("a.md", "# 没 frontmatter\n\n正文\n")];
        let r = lint(&pages, None, &LintConfig::default());
        assert_eq!(r.count(Rule::BadFrontmatter), 1, "{:#?}", r.findings);
        assert_eq!(r.count(Rule::MissingPageType), 1, "{:#?}", r.findings);
    }

    #[test]
    fn missing_index_entry_only_when_index_loaded() {
        let pages = clean_pages();
        let full = index_of(&pages);
        assert_eq!(
            lint(&pages, Some(&full), &LintConfig::default()).count(Rule::MissingIndexEntry),
            0
        );

        let mut partial = full.clone();
        partial.remove("B");
        let r = lint(&pages, Some(&partial), &LintConfig::default());
        assert_eq!(r.count(Rule::MissingIndexEntry), 1);
        assert!(r.stats.index_loaded);

        // ⚠️ index.md 不存在时：不产生该规则发现，但 index_loaded=false 必须显眼
        let none = lint(&pages, None, &LintConfig::default());
        assert_eq!(none.count(Rule::MissingIndexEntry), 0);
        assert!(!none.stats.index_loaded);
        assert!(
            none.summary().contains("index.md 未载入"),
            "{}",
            none.summary()
        );
    }

    #[test]
    fn report_is_deterministic_and_sorted() {
        let pages = vec![
            concept("a.md", "A", "见 [[缺页]]", "内存, 安全"),
            concept("b.md", "B", "正文", "内存, 安全"),
        ];
        let cfg = LintConfig::default();
        let r1 = lint(&pages, None, &cfg);
        let r2 = lint(&pages.clone(), None, &cfg);
        assert_eq!(r1, r2, "同一输入必须产出同一报告");
        let ids: Vec<&str> = r1.findings.iter().map(|f| f.rule.id()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "发现必须按规则 ID 排序");
    }

    #[test]
    fn every_rule_has_a_stable_id() {
        for r in Rule::ALL {
            assert!(!r.id().is_empty());
            assert_eq!(r.to_string(), r.id());
        }
        // ID 不得重复（否则报告里两类问题无法区分）
        let mut ids: Vec<&str> = Rule::ALL.iter().map(|r| r.id()).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "规则 ID 有重复");
    }
}
