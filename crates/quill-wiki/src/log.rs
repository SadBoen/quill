//! `log.md` —— 时间导向的追加日志（规格 §四）。
//!
//! # 格式是契约，不是排版
//!
//! 规格要求 `log.md` 能被
//! `grep "^## \[" log.md | tail -5` 解析出「最近做了什么」。
//! 因此 [`LogEntry::heading`] 严格产出
//! `## [YYYY-MM-DD] <op> | <title>`，**任何**前缀改动都会让这条命令失效，
//! 而它失效时**不报错**（grep 只是少输出几行）—— 典型的静默失败。
//! 所以 [`parse_log`] 反向也只认这个前缀，并用测试锁住。
//!
//! # 追加不修改
//!
//! 日志的价值全在「历史上做过什么」。覆盖写会让「上周那次 ingest 改了什么」
//! 永久丢失，而 spec §九指出 wiki 的复利效应依赖这段历史。
//! 落盘在 [`crate::store::WikiStore::append_log`]（`O_APPEND` 语义的实现）。

use crate::date::Date;

/// 三种操作（规格 §三：ingest / query / lint）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogOp {
    /// 摄入来源。
    Ingest,
    /// 查询资料库。
    Query,
    /// 体检。
    Lint,
}

impl LogOp {
    /// 字面量。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ingest => "ingest",
            Self::Query => "query",
            Self::Lint => "lint",
        }
    }

    /// 解析。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ingest" => Some(Self::Ingest),
            "query" => Some(Self::Query),
            "lint" => Some(Self::Lint),
            _ => None,
        }
    }

    /// 三种操作全集。
    pub const ALL: [LogOp; 3] = [Self::Ingest, Self::Query, Self::Lint];
}

impl std::fmt::Display for LogOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 标题行前缀。**改它等于改契约**（见模块文档）。
pub const LOG_HEADING_PREFIX: &str = "## [";

/// 一条日志。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// 日期（由调用方传入 —— 本 crate 不取系统时间，见 [`crate::date`]）。
    pub date: Date,
    /// 操作。
    pub op: LogOp,
    /// 标题（来源名 / query 主题 / 体检范围）。
    pub title: String,
    /// 正文（可含多行）。
    pub body: String,
}

impl LogEntry {
    /// 构造（标题去首尾空白，空白标题被替换为占位符而非报错 ——
    /// 日志不该因为标题为空而写不进去，那会让一次 ingest 无痕消失）。
    pub fn new(date: Date, op: LogOp, title: impl Into<String>, body: impl Into<String>) -> Self {
        let title = title.into();
        let title = if title.trim().is_empty() {
            "(无标题)".to_string()
        } else {
            title.trim().to_string()
        };
        Self {
            date,
            op,
            title,
            body: body.into(),
        }
    }

    /// 标题行（不含换行）。
    ///
    /// ⚠️ 格式 `## [YYYY-MM-DD] op | Title` 是**契约**（见模块文档）。
    pub fn heading(&self) -> String {
        format!(
            "{}{}] {} | {}",
            LOG_HEADING_PREFIX, self.date, self.op, self.title
        )
    }

    /// 完整条目（含正文与结尾换行），供追加。
    pub fn render(&self) -> String {
        let mut s = self.heading();
        s.push('\n');
        if !self.body.trim().is_empty() {
            s.push('\n');
            s.push_str(self.body.trim_end());
            s.push('\n');
        }
        s
    }
}

/// 解析 `log.md`。
///
/// 遇到不认识的标题行**跳过并计数**（不返回错误）：
/// `log.md` 是纯追加的产物，用户可能手工加过行。
/// 但跳过数**必须**由调用方知晓，故返回 [`ParsedLog`] 而不是裸 `Vec`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedLog {
    /// 成功解析的条目（按文件顺序）。
    pub entries: Vec<LogEntry>,
    /// 无法解析的 `## [` 开头行数（**必须显眼**，铁律十六）。
    pub skipped_headings: usize,
}

impl ParsedLog {
    /// 最近 `n` 条（文件顺序即时间顺序 → 取尾部）。
    ///
    /// 对应 `grep "^## \[" log.md | tail -5`。
    pub fn tail(&self, n: usize) -> &[LogEntry] {
        let start = self.entries.len().saturating_sub(n);
        &self.entries[start..]
    }
}

/// 解析 `log.md` 全文。
pub fn parse_log(text: &str) -> ParsedLog {
    let mut out = ParsedLog::default();
    let mut current: Option<(Date, LogOp, String, String)> = None;

    let flush = |cur: &mut Option<(Date, LogOp, String, String)>, out: &mut ParsedLog| {
        if let Some((date, op, title, body)) = cur.take() {
            out.entries.push(LogEntry {
                date,
                op,
                title,
                body,
            });
        }
    };

    for line in text.lines() {
        if line.starts_with(LOG_HEADING_PREFIX) {
            flush(&mut current, &mut out);
            match parse_heading(line) {
                Some((date, op, title)) => current = Some((date, op, title, String::new())),
                None => out.skipped_headings += 1,
            }
            continue;
        }
        // 标题行之前的内容（日志文件顶部的说明文字）不属于任何条目 → 丢弃。
        if let Some((.., body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(&mut current, &mut out);
    out
}

/// 解析标题行：`## [YYYY-MM-DD] op | Title`。
fn parse_heading(line: &str) -> Option<(Date, LogOp, String)> {
    let rest = line.strip_prefix(LOG_HEADING_PREFIX)?;
    let (date_part, rest) = rest.split_once(']')?;
    let date = Date::parse(date_part.trim()).ok()?;
    let rest = rest.trim();
    let (op_part, title) = rest.split_once('|')?;
    let op = LogOp::parse(op_part)?;
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    Some((date, op, title.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date {
        Date::parse(s).expect("测试日期应合法")
    }

    #[test]
    fn heading_matches_the_grep_contract() {
        let e = LogEntry::new(d("2026-04-02"), LogOp::Ingest, "Article Title", "");
        assert_eq!(e.heading(), "## [2026-04-02] ingest | Article Title");
        // 契约：这一行必须能被 `grep "^## \["` 命中
        assert!(e.heading().starts_with(LOG_HEADING_PREFIX));
    }

    #[test]
    fn render_and_parse_round_trip() {
        let e = LogEntry::new(
            d("2026-10-04"),
            LogOp::Ingest,
            "某来源.pdf",
            "新建 summary 1 页，更新 3 页。\n\n- A\n- B",
        );
        let text = e.render();
        assert!(text.starts_with("## [2026-10-04] ingest | 某来源.pdf\n"));
        let parsed = parse_log(&text);
        assert_eq!(parsed.skipped_headings, 0);
        assert_eq!(parsed.entries.len(), 1);
        let back = &parsed.entries[0];
        assert_eq!(back.date, d("2026-10-04"));
        assert_eq!(back.op, LogOp::Ingest);
        assert_eq!(back.title, "某来源.pdf");
        assert!(back.body.contains("新建 summary 1 页"));
    }

    #[test]
    fn multi_entry_log_parses_in_order() {
        let text = "\
# 说明文字，不属于任何条目

## [2026-10-01] ingest | 甲

第一条

## [2026-10-02] query | 乙

第二条

## [2026-10-03] lint | 全库

第三条
";
        let p = parse_log(text);
        assert_eq!(p.skipped_headings, 0);
        assert_eq!(p.entries.len(), 3);
        assert_eq!(p.entries[0].op, LogOp::Ingest);
        assert_eq!(p.entries[1].op, LogOp::Query);
        assert_eq!(p.entries[2].op, LogOp::Lint);
        assert_eq!(p.entries[0].body.trim(), "第一条");
        // tail 对应 `grep | tail -N`
        let t = p.tail(2);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].title, "乙");
        assert_eq!(t[1].title, "全库");
    }

    #[test]
    fn all_three_ops_round_trip() {
        for op in LogOp::ALL {
            let e = LogEntry::new(d("2026-10-04"), op, "标题", "正文");
            let p = parse_log(&e.render());
            assert_eq!(p.entries.len(), 1, "op {op} 往返失败");
            assert_eq!(p.entries[0].op, op);
        }
    }

    #[test]
    fn malformed_headings_are_counted_not_silently_dropped() {
        let text = "\
## [2026-13-99] ingest | 坏日期

## [2026-10-04] 不是操作 | 坏操作

## [2026-10-04] ingest | 好的

## [2026-10-04] ingest |

## [2026-10-04] ingest | 也好的
";
        let p = parse_log(text);
        assert_eq!(p.entries.len(), 2, "好条目应照常解析");
        assert_eq!(p.skipped_headings, 3, "坏标题数必须被计入而不是消失");
    }

    #[test]
    fn empty_title_is_replaced_not_dropped() {
        let e = LogEntry::new(d("2026-10-04"), LogOp::Query, "   ", "正文");
        assert_eq!(e.title, "(无标题)");
        let p = parse_log(&e.render());
        assert_eq!(p.entries.len(), 1, "空标题不该让整条日志消失");
    }

    #[test]
    fn tail_clamps_on_short_log() {
        let p = parse_log("");
        assert!(p.tail(5).is_empty());
    }
}
