//! 最小日期类型（`YYYY-MM-DD`）。
//!
//! # 为什么自己写而不用 `chrono` / `time`
//!
//! **范围硬约束**：`crates/quill-wiki/Cargo.toml` **禁止新增外部 crate**
//! （见派工约束「Cargo.lock 目前零新增外部 crate」）。
//! 而 wiki 只需要 `YYYY-MM-DD` 一种形态 —— 不需要时区、不需要时刻、
//! 不需要 Duration。为它引入一个时间库，是本项目反复被点名的
//! 「过早优化」形态（`docs/V1_SCOPE_CONSTRAINTS.md` 约束 3 的同类推理）。
//!
//! ⚠️ **本类型只做存储与校验，不生成当前日期**：
//! 「今天是哪天」必须由调用方（server 层）传入，
//! 否则同一个 crate 在测试里与生产里会得到不同结果，且无法复现。

use std::fmt;

/// 日期。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

/// 日期解析失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateError {
    /// 长度不是 10。
    BadLength { got: usize },
    /// 分隔符位置不对（必须是 `YYYY-MM-DD`）。
    BadShape(String),
    /// 某一段不是数字。
    NotNumeric { field: &'static str, value: String },
    /// 月份越界（1~12）。
    MonthOutOfRange(u8),
    /// 该月没有这一天（如 2 月 30 日）。
    DayOutOfRange { year: u16, month: u8, day: u8 },
    /// 年份为 0。
    YearZero,
}

impl fmt::Display for DateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLength { got } => write!(f, "日期长度为 {got}，期望 10（YYYY-MM-DD）"),
            Self::BadShape(s) => write!(f, "日期 {s:?} 不是 YYYY-MM-DD 形态"),
            Self::NotNumeric { field, value } => {
                write!(f, "日期字段 {field} 的值 {value:?} 不是数字")
            }
            Self::MonthOutOfRange(m) => write!(f, "月份 {m} 越界（合法 1~12）"),
            Self::DayOutOfRange { year, month, day } => {
                write!(f, "{year:04}-{month:02} 没有第 {day} 天")
            }
            Self::YearZero => f.write_str("年份不得为 0"),
        }
    }
}

impl std::error::Error for DateError {}

/// 闰年判定（公历）。
///
/// 公开是为了让测试能对拍——它本身是纯函数，不产生副作用。
pub const fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && !year.is_multiple_of(100) || year.is_multiple_of(400)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        // 调用方已保证 month ∈ 1..=12；这里给一个不可能的分支而不是 panic。
        _ => 0,
    }
}

impl Date {
    /// 构造并校验。
    pub fn new(year: u16, month: u8, day: u8) -> Result<Self, DateError> {
        if year == 0 {
            return Err(DateError::YearZero);
        }
        if !(1..=12).contains(&month) {
            return Err(DateError::MonthOutOfRange(month));
        }
        if day < 1 || day > days_in_month(year, month) {
            return Err(DateError::DayOutOfRange { year, month, day });
        }
        Ok(Self { year, month, day })
    }

    /// 解析 `YYYY-MM-DD`。
    pub fn parse(s: &str) -> Result<Self, DateError> {
        if s.len() != 10 {
            return Err(DateError::BadLength { got: s.len() });
        }
        let b = s.as_bytes();
        if b[4] != b'-' || b[7] != b'-' {
            return Err(DateError::BadShape(s.to_string()));
        }
        let num = |from: usize, to: usize, field: &'static str| -> Result<u32, DateError> {
            let raw = &s[from..to];
            raw.parse::<u32>().map_err(|_| DateError::NotNumeric {
                field,
                value: raw.to_string(),
            })
        };
        let year = num(0, 4, "year")?;
        let month = num(5, 7, "month")?;
        let day = num(8, 10, "day")?;
        // 年份 4 位上限 9999，u16 装得下；月/日都是 2 位，u8 也装得下。
        // 越界判定全部交给 new —— 单一判据，避免两处各判一半。
        Self::new(year as u16, month as u8, day as u8)
    }

    /// 年。
    pub fn year(&self) -> u16 {
        self.year
    }

    /// 月（1~12）。
    pub fn month(&self) -> u8 {
        self.month
    }

    /// 日（1~31）。
    pub fn day(&self) -> u8 {
        self.day
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl std::str::FromStr for Date {
    type Err = DateError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Date::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_round_trips() {
        let d = Date::parse("2026-10-05").expect("应能解析");
        assert_eq!(d.year(), 2026);
        assert_eq!(d.month(), 10);
        assert_eq!(d.day(), 5);
        assert_eq!(d.to_string(), "2026-10-05");
    }

    #[test]
    fn rejects_bad_shapes() {
        assert!(matches!(
            Date::parse("2026-10-5"),
            Err(DateError::BadLength { .. })
        ));
        assert!(matches!(
            Date::parse("2026/10/05"),
            Err(DateError::BadShape(_))
        ));
        assert!(matches!(
            Date::parse("2026-1x-05"),
            Err(DateError::NotNumeric { .. })
        ));
    }

    #[test]
    fn rejects_impossible_days() {
        assert!(matches!(
            Date::parse("2026-13-01"),
            Err(DateError::MonthOutOfRange(13))
        ));
        assert!(matches!(
            Date::parse("2026-02-30"),
            Err(DateError::DayOutOfRange { .. })
        ));
        assert!(matches!(
            Date::parse("0000-01-01"),
            Err(DateError::YearZero)
        ));
    }

    #[test]
    fn leap_year_rules_follow_gregorian() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2026));
        assert!(!is_leap_year(1900));
        assert!(is_leap_year(2000));
        // 2 月 29 日只在闰年合法 —— 这是本函数存在的唯一理由。
        assert!(Date::parse("2024-02-29").is_ok());
        assert!(Date::parse("2026-02-29").is_err());
    }
}
