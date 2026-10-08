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
            // 切片安全：上面的 `-` 检查保证了 4 与 7 是单字节字符，于是 0/4/5/7/8/10
            // 都落在字符边界上，这几个区间不会切在字符中间。
            let raw = &s[from..to];
            // **必须逐字节是 ASCII 数字**。`u32::from_str` 接受前导 `+`（`"+001"` → 1），
            // 于是 `"0009-+9-+9"` 会被静默归一化成 `0009-09-09` —— 收下的输入与写回的
            // 输入不是一个串。属性测试抓到并自动缩小到这条输入（queue Q089）。
            if !raw.bytes().all(|b| b.is_ascii_digit()) {
                return Err(DateError::NotNumeric {
                    field,
                    value: raw.to_string(),
                });
            }
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

    /// 从「1970-01-01 起的天数」算出日期（Howard Hinnant 的 `civil_from_days`）。
    ///
    /// 为什么单独抽一层：`today()` 依赖系统时钟，闰年/跨月这类边界**测不了**；
    /// 这一层喂进去的是固定天数，可以逐条钉住。
    pub fn from_unix_days(days: i64) -> Result<Self, DateError> {
        // 纪元先挪到 0000-03-01，让闰日落在「年」的末尾，闰年规则就只剩一条。
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]，3 月起算
        let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
        let year = if m <= 2 { y + 1 } else { y };
        Self::new(year as u16, m as u8, d as u8)
    }

    /// 今天（UTC）。资料库变更日志的日期用的就是它。
    ///
    /// 系统时钟不可用（理论上不会）时退到纪元日，而不是 panic ——
    /// 一条日志的日期不该让整次写入失败。
    pub fn today() -> Self {
        let days = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_secs() / 86_400) as i64)
            .unwrap_or(0);
        Self::from_unix_days(days).unwrap_or(Self {
            year: 1970,
            month: 1,
            day: 1,
        })
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

    #[test]
    fn unix_days_land_on_the_right_civil_date() {
        // 纪元当天、以及闰年/跨月/跨年这几处最容易算错的地方。
        assert_eq!(Date::from_unix_days(0).unwrap().to_string(), "1970-01-01");
        assert_eq!(
            Date::from_unix_days(19_723).unwrap().to_string(),
            "2024-01-01"
        );
        assert_eq!(
            Date::from_unix_days(19_782).unwrap().to_string(),
            "2024-02-29"
        );
        assert_eq!(
            Date::from_unix_days(19_783).unwrap().to_string(),
            "2024-03-01"
        );
        // 负天数（1970 之前）也要对，别在 0 附近取整取歪。
        assert_eq!(Date::from_unix_days(-1).unwrap().to_string(), "1969-12-31");
        assert_eq!(
            Date::from_unix_days(-365).unwrap().to_string(),
            "1969-01-01"
        );
        // 世纪闰年：2000-02-29 存在（1900 不是闰年，这一条正好跨过去）。
        assert_eq!(
            Date::from_unix_days(11_016).unwrap().to_string(),
            "2000-02-29"
        );
    }

    #[test]
    fn today_is_a_usable_date_and_matches_the_clock_day() {
        // 不钉具体日期（会随运行时间漂），只钉「是个合法日期、且与系统时钟同一天」。
        let today = Date::today();
        let days = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("测试机时钟应在 1970 之后")
            .as_secs() as i64
            / 86_400;
        assert_eq!(today, Date::from_unix_days(days).expect("当天必须可表示"));
        assert!(
            Date::parse(&today.to_string()).is_ok(),
            "today 必须能原样解析回来"
        );
    }
}
