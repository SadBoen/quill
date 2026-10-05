
use std::collections::BTreeMap;

use crate::date::Date;
use crate::page::{Page, PageType};
use crate::store::WikiError;

pub const INDEX_TITLE: &str = "# Wiki 索引";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {

    pub title: String,

    pub summary: String,

    pub page_type: PageType,

    pub source_count: u32,

    pub updated: Date,
}

impl IndexEntry {

    pub fn from_page(p: &Page) -> Option<Self> {

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WikiIndex {
    entries: BTreeMap<String, IndexEntry>,
}

impl WikiIndex {

    pub fn new() -> Self {
        Self::default()
    }

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

    pub fn upsert(&mut self, e: IndexEntry) {
        self.entries.insert(e.title.clone(), e);
    }

    pub fn remove(&mut self, title: &str) -> bool {
        self.entries.remove(title).is_some()
    }

    pub fn get(&self, title: &str) -> Option<&IndexEntry> {
        self.entries.get(title)
    }

    pub fn contains(&self, title: &str) -> bool {
        self.entries.contains_key(title)
    }

    pub fn entries(&self) -> impl Iterator<Item = &IndexEntry> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn render(&self) -> String {

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

        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title)));
        scored.truncate(limit);
        scored.into_iter().map(|(_, e)| e).collect()
    }
}

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

    let summary = tail.trim().trim_start_matches('—').trim().to_string();

    Ok(IndexEntry {
        title,
        summary,

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
            "- [[]] — x（source_count=1, updated=2026-10-04）",
            "- [[A]] — x（updated=2026-13-40）",
            "- [[A]] — x（source_count=多, updated=2026-10-04）",
            "- [[A]] — x",
            "- [[A]] — x（未知键=1, updated=2026-10-04）",
        ];
        for line in bad {
            let r = WikiIndex::parse(line);
            assert!(r.is_err(), "坏行 {line:?} 未被拒绝");
        }
    }

    #[test]
    fn parse_ignores_non_entry_lines_without_erroring() {

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

        assert_eq!(back.render(), text);
    }
}
