//! Markdown + YAML frontmatter + `[[wikilink]]` 解析（Obsidian 兼容子集）。
//!
//! 依据 `docs/XU_WIKI_SPEC.md` §6.3「页面格式」。
//!
//! # 为什么手写解析器而不引 markdown crate
//!
//! **派工硬约束**：`crates/quill-wiki` 不新增外部 crate
//! （`Cargo.lock` 现状零新增外部 crate；本 crate 只允许依赖
//! `quill-adapters` / `quill-domain`）。
//! `pulldown-cmark` / `serde_yaml` / `gray_matter` 全部不在依赖表内。
//!
//! 观察到的另一条理由：wiki 只需要 frontmatter 的**极小子集**
//! （标量 + 行内数组 + 缩进列表），不需要完整 YAML。
//! 完整 YAML 的锚点、别名、多文档、类型推断在这里全是无用复杂度。
//!
//! # 降级策略（铁律七：失败必须自诊断，不得 panic）
//!
//! **坏 frontmatter 不让整个页面不可读**，而是产出
//! [`Page::warnings`]，让调用方能报告"这个页面 frontmatter 是坏的"
//! 而不是"解析失败"。理由：一个手写的 LLM 产出里 frontmatter 写歪
//! 是常态，若因此整页读不出来，lint 就永远看不到那页的问题。
//!
//! 但**「降级」不等于「静默」**：每个降级都对应一条 [`PageWarning`]，
//! lint 会把它报成 `BadFrontmatter`。

use crate::date::Date;

// ─────────────────────────── 页面类型 ───────────────────────────

/// 页面类型（`docs/XU_WIKI_SPEC.md` §二 层 2，六种）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PageType {
    /// 来源摘要（一次 ingest 对一篇来源的提炼）。
    Summary,
    /// 实体（人 / 组织 / 产品 / 系统）。
    Entity,
    /// 概念（抽象概念、方法论）。
    Concept,
    /// 对比页。
    Comparison,
    /// 概览页（某个领域的总入口）。
    Overview,
    /// 综合页（跨页综合出的结论）。
    Synthesis,
}

impl PageType {
    /// frontmatter 里的字面量。
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

    /// 解析 frontmatter 里的 `type:` 值。
    ///
    /// 大小写不敏感（Obsidian 侧的用户手改常见大小写混用）。
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

    /// 六种类型的全集，供 lint 与测试遍历。
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

// ─────────────────────────── frontmatter ───────────────────────────

/// frontmatter 里出现、但本解析器不认识（或解析失败）的键值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownField {
    /// 原始键名。
    pub key: String,
    /// 原始值（未解析）。
    pub value: String,
}

/// 解析后的 frontmatter。
///
/// ⚠️ 字段全部是 `Option` / `Vec`：**没有**「解析成功」与「解析失败」
/// 的二分。每个字段独立降级，使一条坏 `tags:` 不会连带丢掉 `title:`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    /// 页面标题（wikilink 的默认目标）。
    pub title: Option<String>,
    /// 页面类型。
    pub page_type: Option<PageType>,
    /// 标签。
    pub tags: Vec<String>,
    /// 创建日期。
    pub created: Option<Date>,
    /// 更新日期。
    pub updated: Option<Date>,
    /// 来源数（`source_count`）。
    pub source_count: Option<u32>,
    /// 未识别 / 解析失败的键值（**不丢弃**，供 lint 报告与后续修复）。
    pub unknown: Vec<UnknownField>,
}

/// 页面降级告警。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageWarning {
    /// 完全没有 frontmatter（文件不以 `---` 开头）。
    MissingFrontmatter,
    /// 有 `---` 开头但没有闭合的 `---`。
    UnterminatedFrontmatter,
    /// frontmatter 某行既不是 `key: value` 也不是列表项。
    MalformedFrontmatterLine { line_no: usize, raw: String },
    /// `type:` 的值不是六种之一。
    UnknownPageType { raw: String },
    /// 日期字段不是 `YYYY-MM-DD` 或不存在该日。
    BadDate {
        key: String,
        raw: String,
        reason: String,
    },
    /// `source_count:` 不是非负整数。
    BadSourceCount { raw: String },
    /// `[[wikilink]]` 没有闭合的 `]]`。
    UnclosedWikilink { at: usize },
    /// `[[wikilink]]` 内部为空。
    EmptyWikilink { at: usize },
    /// `[[` 内部又出现 `[[`（嵌套 wikilink）。
    ///
    /// ⚠️ **Obsidian 不支持嵌套 wikilink**，本解析器也不支持。
    /// 明确降级为「整条丢弃 + 告警」，而不是硬解出 `A [[B` 这种垃圾目标 ——
    /// 垃圾目标会进图里成为一条指向不存在页面的假边。
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

// ─────────────────────────── wikilink ───────────────────────────

/// 一条 `[[wikilink]]`。
///
/// 支持 Obsidian 两种写法：
/// - `[[目标]]`
/// - `[[目标|显示文本]]`
///
/// ⚠️ 目标里的 `#锚点` 被切到 [`Wikilink::anchor`]：
/// 它是**页内锚点**，不是另一个页面。若不切走，孤儿检测会把
/// `[[Rust#借用检查]]` 当成指向「名为 `Rust#借用检查` 的页面」的链接，
/// 从而误报「链接指向不存在的页面」。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Wikilink {
    /// 目标页面标题（`#锚点` 已切走）。
    pub target: String,
    /// `|` 之后的显示文本。
    pub alias: Option<String>,
    /// `#` 之后的页内锚点。
    pub anchor: Option<String>,
    /// 在**正文**（不含 frontmatter）中的字符偏移。
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

// ─────────────────────────── 页面 ───────────────────────────

/// 一个已解析的 wiki 页面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// 页面在 wiki 层里的相对路径（如 `concepts/rust-ownership.md`）。
    ///
    /// ⚠️ **相对 wiki 根，不含用户目录** —— 用户隔离由
    /// [`crate::store::WikiStore`] 的根路径承担，不靠路径里嵌 user_id。
    pub path: String,
    /// frontmatter。
    pub frontmatter: Frontmatter,
    /// 正文（不含 frontmatter）。
    pub body: String,
    /// 正文里的 wikilink（按出现顺序）。
    pub links: Vec<Wikilink>,
    /// 降级告警。
    pub warnings: Vec<PageWarning>,
}

impl Page {
    /// 页面标题：frontmatter 的 `title`，缺则退回**文件名**（去扩展名）。
    ///
    /// ⚠️ 退回文件名而不是空串：Obsidian 里「无 frontmatter 的裸文件」
    /// 仍以文件名参与链接。返回空串会让所有这类页面挤成同一个空标题，
    /// 孤儿检测随之失去意义。
    pub fn title(&self) -> String {
        self.frontmatter
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.file_stem().to_string())
    }

    /// 路径的文件名主干（去 `.md`、去目录）。
    pub fn file_stem(&self) -> &str {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        name.strip_suffix(".md").unwrap_or(name)
    }

    /// 页面类型；frontmatter 没写或写坏时为 `None`。
    pub fn page_type(&self) -> Option<PageType> {
        self.frontmatter.page_type
    }

    /// 是否存在降级告警（lint 的 `BadFrontmatter` 据此判定）。
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

// ─────────────────────────── 解析实现 ───────────────────────────

/// 解析一个 wiki 页面文件。
///
/// `path` 是**页面的相对路径**，仅用于 [`Page::title`] 的文件名回退与
/// 诊断定位；它不参与解析，也不决定文件读哪儿。
pub fn parse_page(path: &str, text: &str) -> Page {
    let mut warnings = Vec::new();
    // 先按行切，保留行号以便诊断定位。
    let normalized = text.replace("\r\n", "\n");
    let had_final_newline = normalized.ends_with('\n');
    let lines: Vec<&str> = normalized.split('\n').collect();

    // frontmatter 必须从**第一行**开始（Obsidian 兼容）——
    // 允许前导空行不算，否则「空行 + 才是 frontmatter」会让
    // 判定依赖不可见的空白，lint 报告的形态难以复现。
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

    // 去掉正文**开头**的空行：frontmatter 与正文之间的那一个空行是分隔符，
    // 不是正文内容。不去掉的话 `parse → render → parse` 会让空行逐次累积。
    let mut body_start = body_start;
    while body_start < lines.len() && lines[body_start].trim().is_empty() {
        body_start += 1;
    }

    // 去掉正文末尾的单个换行（文件规范要求以换行结尾），
    // 让「同一个页面写两次得到同一个字符串」成立。
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

/// 解析 `---` ... `---` 之间的 frontmatter 块。
///
/// `start` 是 `---` 之后那一行的下标。返回 `(frontmatter, 正文起始行)`。
fn parse_frontmatter(
    lines: &[&str],
    start: usize,
    warnings: &mut Vec<PageWarning>,
) -> (Frontmatter, usize) {
    let mut fm = Frontmatter::default();
    // 当前正在收集哪个键的列表项（`tags:` 之后的 `- a` / `- b`）。
    let mut list_key: Option<String> = None;

    let mut i = start;
    let body_start = loop {
        if i >= lines.len() {
            // 走到文件末尾都没有闭合 `---`。
            warnings.push(PageWarning::UnterminatedFrontmatter);
            // ⚠️ 降级：已解析出的键**保留**，正文从 frontmatter 之后继续。
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

        // 非 key: value / 非列表项 / 非注释 —— 它**不是** frontmatter。
        //
        // ⚠️ 只有在「往后找不到闭合 `---`」时才认定 frontmatter 到此为止。
        // 若后面确有闭合 `---`，说明这行只是写歪了的一行字段：
        // 继续当字段处理并告警（不提前结束，否则会漏掉后面的合法字段）。
        if !trimmed.starts_with("- ") && !trimmed.contains(':') && !closing_exists_after(lines, i) {
            warnings.push(PageWarning::UnterminatedFrontmatter);
            break i;
        }
        i += 1;

        // 缩进列表项：`- foo` 或 `  - foo`
        let is_item = trimmed.starts_with("- ");
        if is_item {
            let item = trimmed[2..].trim();
            match list_key.as_deref() {
                Some("tags") => {
                    // 引号包裹的标签去掉引号。
                    let item = item.trim_matches(['"', '\'']);
                    if !item.is_empty() {
                        fm.tags.push(item.to_string());
                    }
                }
                Some(other) => {
                    // 列表挂在别的键上 → 不是本解析器支持的形态。
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

        // `key: value` / `key:`（空值意味着后面可能跟列表）
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
            // 空值：开一个可能延续的列表。
            list_key = Some(key);
            continue;
        }
        list_key = None;
        apply_field(&mut fm, &key, value, warnings);
    };

    (fm, body_start)
}

/// `i` 之后是否还有闭合 `---`（不含 `i` 自身）。
fn closing_exists_after(lines: &[&str], i: usize) -> bool {
    lines.iter().skip(i + 1).any(|l| l.trim() == "---")
}

/// 把一条 `key: value` 落到 [`Frontmatter`] 上。
fn apply_field(fm: &mut Frontmatter, key: &str, value: &str, warnings: &mut Vec<PageWarning>) {
    // 值两端的引号统一剥掉（`title: "A"` 与 `title: 'A'` 都接受）。
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
            // 行内数组：`tags: [a, b]`（规格 §6.3 的写法）
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
            // 坏日期不猜、不填默认值：一律留空 + 记录原文 + 告警。
            // 填默认日期会让「这页从没更新过」与「这页日期写坏了」
            // 在下游不可区分 —— 而后者是需要人修的真问题。
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

/// 围栏标记（` ``` ` 或 `~~~`）。
fn fence_marker_of(trimmed: &str) -> Option<&'static str> {
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// 抽取正文中的 `[[wikilink]]`。
///
/// ⚠️ **代码块内的 `[[...]]` 不算链接**：`[[` 出现在围栏内时是示例文本。
/// 不排除会让「文档里举例 `[[X]]`」凭空造出一条边，而图的边直接决定孤儿判定
/// —— 假边 = 假孤儿 = 假闸门（AGENTS.md 八类之 1）。
pub fn parse_wikilinks(body: &str, warnings: &mut Vec<PageWarning>) -> Vec<Wikilink> {
    let mut out = Vec::new();
    // 当前未闭合的围栏标记；`None` = 不在围栏内。
    let mut open_fence: Option<&'static str> = None;
    let mut offset = 0usize;

    for line in body.split('\n') {
        let t = line.trim_start();
        match open_fence {
            Some(marker) => {
                // 围栏内：只有「同种标记、且标记之后无内容」这一行才闭合围栏。
                if t.starts_with(marker) && t[marker.len()..].trim().is_empty() {
                    open_fence = None;
                }
                // 围栏行与围栏内容都**不**扫链接。
            }
            None => match fence_marker_of(t) {
                Some(marker) => open_fence = Some(marker),
                None => scan_line(line, offset, &mut out, warnings),
            },
        }
        // +1 是被 split 吃掉的换行符。
        offset += line.len() + 1;
    }
    out
}

/// 扫描单行的 wikilink。`base` 是该行在正文中的起始偏移。
fn scan_line(line: &str, base: usize, out: &mut Vec<Wikilink>, warnings: &mut Vec<PageWarning>) {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] != b'[' || bytes[i + 1] != b'[' {
            i += 1;
            continue;
        }
        let open = i;
        // 找闭合的 `]]`。同一行内闭合（Obsidian 允许跨行，
        // 但跨行链接在 LLM 产出里极罕见且会与代码块判定互相干扰，
        // 跨行时按「未闭合」降级告警）。
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

        // 嵌套 `[[`：`[[A [[B]] C]]`。整条丢弃（见 PageWarning::NestedWikilink）。
        if inner.contains("[[") {
            warnings.push(PageWarning::NestedWikilink { at: base + open });
            i = close + 2;
            continue;
        }

        // `目标|显示文本`
        let (target_part, alias) = match inner.split_once('|') {
            Some((t, a)) => (t, Some(a.trim().to_string())),
            None => (inner, None),
        };
        // `目标#锚点` —— 锚点是页内的，不是另一个页面。
        let (target, anchor) = match target_part.split_once('#') {
            Some((t, a)) => (t.trim().to_string(), Some(a.trim().to_string())),
            None => (target_part.trim().to_string(), None),
        };
        let alias = alias.filter(|a| !a.is_empty());

        if target.is_empty() {
            // `[[#锚点]]` 是**页内**锚点引用，不指向别的页面 →
            // 不产生边（否则每个页面都会有一条指向自己的边）。
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

/// 把一个页面序列化回 Markdown（含 frontmatter）。
///
/// 供 ingest 编排写回页面用。**格式必须与解析器对称**，
/// 否则「解析 → 改 → 写回」会让页面每次 ingest 都掉一点内容。
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
        // 逐字取自 docs/XU_WIKI_SPEC.md §6.3 的页面格式
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
        // frontmatter 不得漏进正文
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
        // 有 key 但没闭合 —— 关键断言：**已解析的 title 不能丢**。
        let p = parse_page("a.md", "---\ntitle: A\ntype: entity\n\n正文仍在。\n");
        assert!(p.warnings.contains(&PageWarning::UnterminatedFrontmatter));
        assert_eq!(p.frontmatter.title.as_deref(), Some("A"));
        assert_eq!(p.page_type(), Some(PageType::Entity));
        // 未解析的行落在正文里，不凭空消失
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
        // 坏日期原文进 unknown，不丢
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
        // 嵌套写法 `[[外层 [[内层]] 收尾]]`：Obsidian 不支持 → 整条丢弃 + 告警。
        // 关键断言：**不能**硬解出 `外层 [[内层` 这种垃圾目标 ——
        // 垃圾目标会变成一条指向不存在页面的假边，直接污染孤儿判定。
        let text = "---\ntitle: 嵌套用例\ntype: concept\n---\n\n真链接 [[A]]，嵌套写法 [[外层 [[内层]] 收尾]] 结束，未闭合 [[B";
        let p = parse_page("a.md", text);
        assert_eq!(p.links.len(), 1);
        assert_eq!(p.links[0].target, "A");
        // ⚠️ `at` 是**字节**偏移（中文 3 字节），不是字符偏移。
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
        // `[[#小节]]` 是页内锚点 —— 若当成链接会造出一条指向自己的边
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
