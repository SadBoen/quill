
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateError {

    BadLength { got: usize },

    BadShape(String),

    NotNumeric { field: &'static str, value: String },

    MonthOutOfRange(u8),

    DayOutOfRange { year: u16, month: u8, day: u8 },

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

        _ => 0,
    }
}

impl Date {

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

        Self::new(year as u16, month as u8, day as u8)
    }

    pub fn year(&self) -> u16 {
        self.year
    }

    pub fn month(&self) -> u8 {
        self.month
    }

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

        assert!(Date::parse("2024-02-29").is_ok());
        assert!(Date::parse("2026-02-29").is_err());
    }
}
