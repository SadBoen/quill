
use crate::date::Date;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PageType {

    Summary,

    Entity,

    Concept,

    Comparison,

    Overview,

    Synthesis,
}

impl PageType {

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Entity => "entity",
            Self::Concept => "concept",
            Self::Comparison => "comparison",
            Self::Overview => "overview",
            Self::Synthesis => "synthesis",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "summary" => Some(Self::Summary),
            "entity" => Some(Self::Entity),
            "concept" => Some(Self::Concept),
            "comparison" => Some(Self::Comparison),
            "overview" => Some(Self::Overview),
            "synthesis" => Some(Self::Synthesis),
            _ => None,
        }
    }

    pub const ALL: [PageType; 6] = [
        Self::Summary,
        Self::Entity,
        Self::Concept,
        Self::Comparison,
        Self::Overview,
        Self::Synthesis,
    ];
}

impl std::fmt::Display for PageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for PageType {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PageType::parse(s).ok_or(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownField {

    pub key: String,

    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {

    pub title: Option<String>,

    pub page_type: Option<PageType>,

    pub tags: Vec<String>,

    pub created: Option<Date>,

    pub updated: Option<Date>,

    pub source_count: Option<u32>,

    pub unknown: Vec<UnknownField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageWarning {

    MissingFrontmatter,

    UnterminatedFrontmatter,

    MalformedFrontmatterLine { line_no: usize, raw: String },

    UnknownPageType { raw: String },

    BadDate {
        key: String,
        raw: String,
        reason: String,
    },

    BadSourceCount { raw: String },

    UnclosedWikilink { at: usize },

    EmptyWikilink { at: usize },

    NestedWikilink { at: usize },
}

impl std::fmt::Display for PageWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFrontmatter => f.write_str("缺少 frontmatter（文件未以 --- 开头）"),
            Self::UnterminatedFrontmatter => f.write_str("frontmatter 缺少闭合的 ---"),
            Self::MalformedFrontmatterLine { line_no, raw } => {
                write!(f, "frontmatter 第 {line_no} 行不是 key: value：{raw:?}")
            }
            Self::UnknownPageType { raw } => {
                write!(f, "type: {raw:?} 不是六种页面类型之一")
            }
            Self::BadDate { key, raw, reason } => {
                write!(f, "{key}: {raw:?} 不是合法日期（{reason}）")
            }
            Self::BadSourceCount { raw } => {
                write!(f, "source_count: {raw:?} 不是非负整数")
            }
            Self::UnclosedWikilink { at } => {
                write!(f, "位置 {at} 的 [[wikilink]] 缺少闭合的 ]]")
            }
            Self::EmptyWikilink { at } => write!(f, "位置 {at} 存在空的 [[]]"),
            Self::NestedWikilink { at } => write!(
                f,
                "位置 {at} 的 wikilink 内部又出现 [[：Obsidian 不支持嵌套 wikilink，该条已丢弃"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Wikilink {

    pub target: String,

    pub alias: Option<String>,

    pub anchor: Option<String>,

    pub offset: usize,
}

impl std::fmt::Display for Wikilink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[[{}", self.target)?;
        if let Some(a) = &self.anchor {
            write!(f, "#{a}")?;
        }
        if let Some(a) = &self.alias {
            write!(f, "|{a}")?;
        }
        f.write_str("]]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {

    pub path: String,

    pub frontmatter: Frontmatter,

    pub body: String,

    pub links: Vec<Wikilink>,

    pub warnings: Vec<PageWarning>,
}

impl Page {

    pub fn title(&self) -> String {
        self.frontmatter
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.file_stem().to_string())
    }

    pub fn file_stem(&self) -> &str {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        name.strip_suffix(".md").unwrap_or(name)
    }

    pub fn page_type(&self) -> Option<PageType> {
        self.frontmatter.page_type
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

pub fn parse_page(path: &str, text: &str) -> Page {
    let mut warnings = Vec::new();

    let normalized = text.replace("\r\n", "\n");
    let had_final_newline = normalized.ends_with('\n');
    let lines: Vec<&str> = normalized.split('\n').collect();

    let fm_start = if lines.first().is_some_and(|l| l.trim() == "---") {
        Some(1usize)
    } else {
        if !normalized.trim().is_empty() {
            warnings.push(PageWarning::MissingFrontmatter);
        }
        None
    };

    let (frontmatter, body_start) = match fm_start {
        Some(start) => parse_frontmatter(&lines, start, &mut warnings),
        None => (Frontmatter::default(), 0),
    };

    let mut body_start = body_start;
    while body_start < lines.len() && lines[body_start].trim().is_empty() {
        body_start += 1;
    }

    let mut body = if body_start >= lines.len() {
        String::new()
    } else {
        lines[body_start..].join("\n")
    };
    if had_final_newline && body.ends_with('\n') {
        body.pop();
    }

    let links = parse_wikilinks(&body, &mut warnings);

    Page {
        path: path.to_string(),
        frontmatter,
        body,
        links,
        warnings,
    }
}

fn parse_frontmatter(
    lines: &[&str],
    start: usize,
    warnings: &mut Vec<PageWarning>,
) -> (Frontmatter, usize) {
    let mut fm = Frontmatter::default();

    let mut list_key: Option<String> = None;

    let mut i = start;
    let body_start = loop {
        if i >= lines.len() {

            warnings.push(PageWarning::UnterminatedFrontmatter);

            break i;
        }
        if lines[i].trim() == "---" {
            break i + 1;
        }
        let raw = lines[i];
        let line_no = i + 1;

        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        if !trimmed.starts_with("- ") && !trimmed.contains(':') && !closing_exists_after(lines, i) {
            warnings.push(PageWarning::UnterminatedFrontmatter);
            break i;
        }
        i += 1;

        let is_item = trimmed.starts_with("- ");
        if is_item {
            let item = trimmed[2..].trim();
            match list_key.as_deref() {
                Some("tags") => {

                    let item = item.trim_matches(['"', '\'']);
                    if !item.is_empty() {
                        fm.tags.push(item.to_string());
                    }
                }
                Some(other) => {

                    fm.unknown.push(UnknownField {
                        key: other.to_string(),
                        value: format!("[列表项 {item}]"),
                    });
                }
                None => warnings.push(PageWarning::MalformedFrontmatterLine {
                    line_no,
                    raw: raw.to_string(),
                }),
            }
            continue;
        }

        let Some((k, v)) = trimmed.split_once(':') else {
            warnings.push(PageWarning::MalformedFrontmatterLine {
                line_no,
                raw: raw.to_string(),
            });
            continue;
        };
        let key = k.trim().to_string();
        let value = v.trim();
        if key.is_empty() {
            warnings.push(PageWarning::MalformedFrontmatterLine {
                line_no,
                raw: raw.to_string(),
            });
            continue;
        }

        if value.is_empty() {

            list_key = Some(key);
            continue;
        }
        list_key = None;
        apply_field(&mut fm, &key, value, warnings);
    };

    (fm, body_start)
}

fn closing_exists_after(lines: &[&str], i: usize) -> bool {
    lines.iter().skip(i + 1).any(|l| l.trim() == "---")
}

fn apply_field(fm: &mut Frontmatter, key: &str, value: &str, warnings: &mut Vec<PageWarning>) {

    let unquoted = value.trim();
    let unquoted = unquoted
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            unquoted
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(unquoted);

    match key {
        "title" => {
            if !unquoted.is_empty() {
                fm.title = Some(unquoted.to_string());
            }
        }
        "type" => match PageType::parse(unquoted) {
            Some(t) => fm.page_type = Some(t),
            None => warnings.push(PageWarning::UnknownPageType {
                raw: unquoted.to_string(),
            }),
        },
        "tags" => {

            if let Some(inner) = unquoted.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                for part in inner.split(',') {
                    let t = part.trim().trim_matches(['"', '\'']);
                    if !t.is_empty() {
                        fm.tags.push(t.to_string());
                    }
                }
            } else if !unquoted.is_empty() {
                fm.tags.push(unquoted.to_string());
            }
        }
        "created" | "updated" => {

            match Date::parse(unquoted) {
                Ok(d) => {
                    if key == "created" {
                        fm.created = Some(d);
                    } else {
                        fm.updated = Some(d);
                    }
                }
                Err(e) => {
                    fm.unknown.push(UnknownField {
                        key: key.to_string(),
                        value: unquoted.to_string(),
                    });
                    warnings.push(PageWarning::BadDate {
                        key: key.to_string(),
                        raw: unquoted.to_string(),
                        reason: e.to_string(),
                    });
                }
            }
        }
        "source_count" => match unquoted.parse::<u32>() {
            Ok(n) => fm.source_count = Some(n),
            Err(_) => warnings.push(PageWarning::BadSourceCount {
                raw: unquoted.to_string(),
            }),
        },
        other => fm.unknown.push(UnknownField {
            key: other.to_string(),
            value: unquoted.to_string(),
        }),
    }
}

fn fence_marker_of(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

pub fn parse_wikilinks(body: &str, warnings: &mut Vec<PageWarning>) -> Vec<Wikilink> {
    let mut out = Vec::new();

    let mut open_fence: Option<&'static str> = None;
    let mut offset = 0usize;

    for line in body.split('\n') {
        let t = line.trim_start();
        match open_fence {
            Some(marker) => {

                if t.starts_with(marker) && t[marker.len()..].trim().is_empty() {
                    open_fence = None;
                }

            }
            None => match fence_marker_of(t) {
                Some(marker) => open_fence = Some(marker),
                None => scan_line(line, offset, &mut out, warnings),
            },
        }

        offset += line.len() + 1;
    }
    out
}

fn scan_line(line: &str, base: usize, out: &mut Vec<Wikilink>, warnings: &mut Vec<PageWarning>) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] != b'[' || bytes[i + 1] != b'[' {
            i += 1;
            continue;
        }
        let open = i;

        let Some(rel_close) = line[i + 2..].find("]]") else {
            warnings.push(PageWarning::UnclosedWikilink { at: base + open });
            return;
        };
        let close = i + 2 + rel_close;
        let inner = &line[i + 2..close];

        if inner.trim().is_empty() {
            warnings.push(PageWarning::EmptyWikilink { at: base + open });
            i = close + 2;
            continue;
        }

        if inner.contains("[[") {
            warnings.push(PageWarning::NestedWikilink { at: base + open });
            i = close + 2;
            continue;
        }

        let (target_part, alias) = match inner.split_once('|') {
            Some((t, a)) => (t, Some(a.trim().to_string())),
            None => (inner, None),
        };

        let (target, anchor) = match target_part.split_once('#') {
            Some((t, a)) => (t.trim().to_string(), Some(a.trim().to_string())),
            None => (target_part.trim().to_string(), None),
        };
        let alias = alias.filter(|a| !a.is_empty());

        if target.is_empty() {

            warnings.push(PageWarning::EmptyWikilink { at: base + open });
            i = close + 2;
            continue;
        }

        out.push(Wikilink {
            target,
            alias,
            anchor,
            offset: base + open,
        });
        i = close + 2;
    }
}

pub fn render_page(page: &Page) -> String {
    let mut s = String::from("---\n");
    let fm = &page.frontmatter;
    if let Some(t) = &fm.title {
        s.push_str(&format!("title: {t}\n"));
    }
    if let Some(t) = fm.page_type {
        s.push_str(&format!("type: {t}\n"));
    }
    if !fm.tags.is_empty() {
        s.push_str(&format!("tags: [{}]\n", fm.tags.join(", ")));
    }
    if let Some(d) = fm.created {
        s.push_str(&format!("created: {d}\n"));
    }
    if let Some(d) = fm.updated {
        s.push_str(&format!("updated: {d}\n"));
    }
    if let Some(n) = fm.source_count {
        s.push_str(&format!("source_count: {n}\n"));
    }
    for u in &fm.unknown {
        s.push_str(&format!("{}: {}\n", u.key, u.value));
    }
    s.push_str("---\n");
    if !page.body.is_empty() {
        s.push('\n');
        s.push_str(&page.body);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spec_example() {

        let text = "---\n\
title: Rust 所有权\n\
type: concept\n\
tags: [内存, 安全]\n\
created: 2026-10-04\n\
updated: 2026-10-04\n\
source_count: 3\n\
---\n\
\n\
# Rust 所有权\n\
\n\
正文内容，引用来源用 [来源名](../raw/xxx.pdf) 格式。\n\
\n\
## 相关\n\
- [[相关概念A]]\n\
- [[相关实体B]]\n";
        let p = parse_page("concepts/rust-ownership.md", text);
        assert!(p.warnings.is_empty(), "不该有告警：{:?}", p.warnings);
        assert_eq!(p.frontmatter.title.as_deref(), Some("Rust 所有权"));
        assert_eq!(p.page_type(), Some(PageType::Concept));
        assert_eq!(p.frontmatter.tags, vec!["内存", "安全"]);
        assert_eq!(p.frontmatter.source_count, Some(3));
        assert_eq!(
            p.frontmatter.created,
            Some(Date::parse("2026-10-04").unwrap())
        );
        assert_eq!(p.links.len(), 2);
        assert_eq!(p.links[0].target, "相关概念A");
        assert_eq!(p.links[1].target, "相关实体B");
        assert!(p.body.starts_with("# Rust 所有权"));

        assert!(!p.body.contains("source_count"));
    }

    #[test]
    fn no_frontmatter_degrades_to_filename_title() {
        let p = parse_page("concepts/裸页面.md", "# 裸页面\n\n正文。\n");
        assert_eq!(p.warnings, vec![PageWarning::MissingFrontmatter]);
        assert_eq!(p.title(), "裸页面");
        assert!(p.page_type().is_none());
    }

    #[test]
    fn unterminated_frontmatter_keeps_what_it_parsed() {

        let p = parse_page("a.md", "---\ntitle: A\ntype: entity\n\n正文仍在。\n");
        assert!(p.warnings.contains(&PageWarning::UnterminatedFrontmatter));
        assert_eq!(p.frontmatter.title.as_deref(), Some("A"));
        assert_eq!(p.page_type(), Some(PageType::Entity));

        assert!(p.body.contains("正文仍在。"));
    }

    #[test]
    fn bad_fields_degrade_independently() {
        let text =
            "---\ntitle: T\ntype: 不是类型\ncreated: 2026-13-99\nsource_count: 三个\n---\n\n正文\n";
        let p = parse_page("x.md", text);
        assert_eq!(p.frontmatter.title.as_deref(), Some("T"));
        assert!(
            p.page_type().is_none(),
            "type 坏 → None，但不能连带 title 坏"
        );
        assert!(matches!(
            p.warnings
                .iter()
                .find(|w| matches!(w, PageWarning::UnknownPageType { .. })),
            Some(PageWarning::UnknownPageType { .. })
        ));
        assert!(p
            .warnings
            .iter()
            .any(|w| matches!(w, PageWarning::BadDate { .. })));
        assert!(p
            .warnings
            .iter()
            .any(|w| matches!(w, PageWarning::BadSourceCount { .. })));

        assert!(p.frontmatter.unknown.iter().any(|u| u.key == "created"));
    }

    #[test]
    fn wikilink_alias_and_anchor() {
        let p = parse_page(
            "a.md",
            "见 [[目标|显示]] 与 [[另一页#小节]] 与 [[第三页#节|别名]]。\n",
        );
        assert_eq!(p.links.len(), 3);
        assert_eq!(p.links[0].target, "目标");
        assert_eq!(p.links[0].alias.as_deref(), Some("显示"));
        assert_eq!(p.links[1].target, "另一页");
        assert_eq!(p.links[1].anchor.as_deref(), Some("小节"));
        assert_eq!(p.links[2].target, "第三页");
        assert_eq!(p.links[2].anchor.as_deref(), Some("节"));
        assert_eq!(p.links[2].alias.as_deref(), Some("别名"));
    }

    #[test]
    fn nested_and_broken_wikilinks() {

        let text = "---\ntitle: 嵌套用例\ntype: concept\n---\n\n真链接 [[A]]，嵌套写法 [[外层 [[内层]] 收尾]] 结束，未闭合 [[B";
        let p = parse_page("a.md", text);
        assert_eq!(p.links.len(), 1);
        assert_eq!(p.links[0].target, "A");

        assert_eq!(
            p.warnings,
            vec![
                PageWarning::NestedWikilink { at: 31 },
                PageWarning::UnclosedWikilink { at: 79 },
            ],
            "两种降级都必须被报出：{:?}",
            p.warnings
        );
    }

    #[test]
    fn page_internal_anchor_is_not_a_link_edge() {

        let p = parse_page("a.md", "见 [[#小节]]。\n");
        assert!(p.links.is_empty(), "页内锚点不该产生边：{:?}", p.links);
    }

    #[test]
    fn code_fence_wikilinks_are_not_edges() {
        let text = "真链接 [[A]]\n\n```markdown\n假链接 [[B]]\n```\n\n另一个真链接 [[C]]\n";
        let p = parse_page("a.md", text);
        let targets: Vec<&str> = p.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, vec!["A", "C"], "围栏内的 [[B]] 不该被当成边");
    }

    #[test]
    fn crlf_is_normalized() {
        let p = parse_page("a.md", "---\r\ntitle: T\r\n---\r\n\r\n正文\r\n");
        assert_eq!(p.frontmatter.title.as_deref(), Some("T"));
        assert_eq!(p.body, "正文");
    }

    #[test]
    fn round_trips_through_render() {
        let text = "---\ntitle: T\ntype: comparison\ntags: [a, b]\ncreated: 2026-01-02\n---\n\n# T\n\n正文 [[A]]\n";
        let p = parse_page("t.md", text);
        let rendered = render_page(&p);
        let again = parse_page("t.md", &rendered);
        assert_eq!(
            p.frontmatter, again.frontmatter,
            "render 丢字段：\n{rendered}"
        );
        assert_eq!(p.body, again.body);
        assert_eq!(p.links, again.links);
    }

    #[test]
    fn block_list_tags_and_malformed_lines() {
        let text = "---\ntitle: T\ntags:\n  - 甲\n  - 乙\n这一行没有冒号\n---\n\n正文\n";
        let p = parse_page("a.md", text);
        assert_eq!(p.frontmatter.tags, vec!["甲", "乙"]);
        assert!(p
            .warnings
            .iter()
            .any(|w| matches!(w, PageWarning::MalformedFrontmatterLine { .. })));
    }

    #[test]
    fn all_six_page_types_round_trip() {
        for t in PageType::ALL {
            let text = format!("---\ntitle: T\ntype: {t}\n---\n\n正文\n");
            let p = parse_page("a.md", &text);
            assert_eq!(p.page_type(), Some(t), "类型 {t} 解析失败");
            assert!(p.warnings.is_empty());
        }
        assert_eq!(PageType::ALL.len(), 6);
    }
}
