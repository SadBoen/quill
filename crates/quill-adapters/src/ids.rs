use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UuidBytes([u8; 16]);

impl UuidBytes {
    pub const fn from_bytes(b: [u8; 16]) -> Self {
        Self(b)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub fn parse(s: &str) -> Result<Self, ParseIdError> {
        let trimmed = s.strip_prefix("urn:uuid:").unwrap_or(s);

        if trimmed.is_empty() {
            return Err(ParseIdError::Empty);
        }
        let compact: String = match trimmed.len() {
            32 => {
                if trimmed.contains('-') {
                    return Err(ParseIdError::HyphenInCompact);
                }
                trimmed.to_string()
            }
            36 => {
                let b = trimmed.as_bytes();

                for pos in HYPHEN_POSITIONS {
                    if b[pos] != b'-' {
                        return Err(ParseIdError::MisplacedHyphen { index: pos });
                    }
                }
                trimmed.replace('-', "")
            }
            other => return Err(ParseIdError::BadLength { got: other }),
        };
        let raw = compact.as_bytes();

        let mut out = [0u8; 16];
        for i in 0..16 {
            let hi = hex_val(raw[i * 2]).ok_or(ParseIdError::NotHex { index: i * 2 })?;
            let lo = hex_val(raw[i * 2 + 1]).ok_or(ParseIdError::NotHex { index: i * 2 + 1 })?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }

    pub fn to_compact_hex(&self) -> String {
        let mut s = String::with_capacity(32);
        for b in &self.0 {
            s.push(char::from(HEX_DIGITS[(b >> 4) as usize]));
            s.push(char::from(HEX_DIGITS[(b & 0x0f) as usize]));
        }
        s
    }

    pub fn to_hyphenated(&self) -> String {
        let h = self.to_compact_hex();
        format!(
            "{}-{}-{}-{}-{}",
            &h[0..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..32]
        )
    }
}

const HYPHEN_POSITIONS: [usize; 4] = [8, 13, 18, 23];
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

impl fmt::Debug for UuidBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UuidBytes({})", self.to_hyphenated())
    }
}

impl fmt::Display for UuidBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.16}…", self.to_compact_hex())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseIdError {
    Empty,

    BadLength { got: usize },

    HyphenInCompact,

    MisplacedHyphen { index: usize },

    NotHex { index: usize },
}

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "标识为空串"),
            Self::BadLength { got } => write!(
                f,
                "标识长度 {got} 非法：应为 32 位 hex 或 36 位带连字符形态"
            ),
            Self::HyphenInCompact => write!(f, "32 位紧凑形态里不允许出现 `-`"),
            Self::MisplacedHyphen { index } => {
                write!(f, "第 {index} 个字节处应为 `-`，但不是")
            }
            Self::NotHex { index } => write!(f, "第 {index} 位不是十六进制字符"),
        }
    }
}

impl std::error::Error for ParseIdError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseSlugError {
    Empty,

    TooLong { len: usize, max: usize },

    IllegalChar { index: usize, ch: char },

    LeadingHyphen,

    TrailingHyphen,

    DoubleHyphen { index: usize },
}

impl fmt::Display for ParseSlugError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "标识为空串"),
            Self::TooLong { len, max } => write!(f, "标识长 {len} 超过上限 {max}"),
            Self::IllegalChar { index, ch } => {
                write!(f, "第 {index} 位字符 {ch:?} 非法：只允许 [a-z0-9-]")
            }
            Self::LeadingHyphen => write!(f, "标识不得以 `-` 开头"),
            Self::TrailingHyphen => write!(f, "标识不得以 `-` 结尾"),
            Self::DoubleHyphen { index } => write!(f, "第 {index} 位出现连续 `--`"),
        }
    }
}

impl std::error::Error for ParseSlugError {}

pub const MAX_SLUG_LEN: usize = 64;

pub fn validate_slug(s: &str) -> Result<(), ParseSlugError> {
    if s.is_empty() {
        return Err(ParseSlugError::Empty);
    }
    if s.trim().is_empty() {
        return Err(ParseSlugError::Empty);
    }
    if s.len() > MAX_SLUG_LEN {
        return Err(ParseSlugError::TooLong {
            len: s.len(),
            max: MAX_SLUG_LEN,
        });
    }
    if s.starts_with('-') {
        return Err(ParseSlugError::LeadingHyphen);
    }
    if s.ends_with('-') {
        return Err(ParseSlugError::TrailingHyphen);
    }
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let ok = b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
        if !ok {
            let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
            return Err(ParseSlugError::IllegalChar { index: i, ch });
        }
        if b == b'-' && i > 0 && bytes[i - 1] == b'-' {
            return Err(ParseSlugError::DoubleHyphen { index: i });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(UuidBytes);

impl UserId {
    pub fn parse(s: &str) -> Result<Self, ParseIdError> {
        UuidBytes::parse(s).map(Self)
    }

    pub const fn from_bytes(b: [u8; 16]) -> Self {
        Self(UuidBytes::from_bytes(b))
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    pub fn to_compact_hex(&self) -> String {
        self.0.to_compact_hex()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(UuidBytes);

impl SessionId {
    pub fn parse(s: &str) -> Result<Self, ParseIdError> {
        UuidBytes::parse(s).map(Self)
    }

    pub const fn from_bytes(b: [u8; 16]) -> Self {
        Self(UuidBytes::from_bytes(b))
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    pub fn to_compact_hex(&self) -> String {
        self.0.to_compact_hex()
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn parse(s: &str) -> Result<Self, ParseSlugError> {
        validate_slug(s)?;
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProviderId({})", self.0)
    }
}

impl TryFrom<&str> for ProviderId {
    type Error = ParseSlugError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

impl TryFrom<String> for ProviderId {
    type Error = ParseSlugError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExpertId(String);

impl ExpertId {
    pub fn parse(s: &str) -> Result<Self, ParseSlugError> {
        validate_slug(s)?;
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExpertId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ExpertId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ExpertId({})", self.0)
    }
}

impl TryFrom<&str> for ExpertId {
    type Error = ParseSlugError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

impl TryFrom<String> for ExpertId {
    type Error = ParseSlugError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberId(String);

impl MemberId {
    pub fn parse(s: &str) -> Result<Self, ParseSlugError> {
        validate_slug(s)?;
        Ok(Self(s.to_string()))
    }

    pub fn for_expert(expert: &ExpertId, seq: u32) -> Result<Self, ParseSlugError> {
        let candidate = format!("{}-{seq}", expert.as_str());
        Self::parse(&candidate)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MemberId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for MemberId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MemberId({})", self.0)
    }
}

impl TryFrom<&str> for MemberId {
    type Error = ParseSlugError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

macro_rules! impl_display_short {
    ($t:ty, $tag:expr) => {
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($tag, "{:.16}"), self.0.to_compact_hex())?;
                f.write_str("…")
            }
        }
    };
}
impl_display_short!(UserId, "u:");
impl_display_short!(SessionId, "s:");

macro_rules! impl_debug_full {
    ($t:ty) => {
        impl fmt::Debug for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($t), self.0.to_hyphenated())
            }
        }
    };
}
impl_debug_full!(UserId);
impl_debug_full!(SessionId);

#[cfg(test)]
mod tests {
    use super::*;

    const HEX32: &str = "f81d4fae7dec11d0a76500a0c91e6bf6";
    const HYPH36: &str = "f81d4fae-7dec-11d0-a765-00a0c91e6bf6";

    #[test]
    fn uuiddigest_parses_compact_and_hyphenated_to_same_bytes() {
        let a = UserId::parse(HEX32).expect("32 位 hex 应合法");
        let b = UserId::parse(HYPH36).expect("36 位带连字符应合法");
        assert_eq!(a, b, "两种打印形态必须解析到同一标识");
        assert_eq!(a.to_compact_hex(), HEX32, "紧凑 hex 应与输入逐字符相同");
    }

    #[test]
    fn uuiddigest_parse_accepts_uppercase_hex_and_normalises_to_lower() {
        let lower = UserId::parse(HEX32).expect("小写应合法");
        let upper = UserId::parse(&HEX32.to_ascii_uppercase()).expect("大写 hex 也应合法");
        assert_eq!(lower, upper, "hex 大小写不应改变标识语义");
        assert_eq!(
            upper.to_compact_hex(),
            HEX32,
            "规范化后必须输出小写，避免同一标识两种字符串形态"
        );
    }

    #[test]
    fn uuiddigest_parse_accepts_urn_prefix() {
        let plain = UserId::parse(HEX32).expect("应合法");
        let urn = UserId::parse(&format!("urn:uuid:{HEX32}")).expect("urn 前缀应被剥掉");
        assert_eq!(plain, urn);
    }

    #[test]
    fn uuiddigest_rejects_empty_string() {
        assert_eq!(UserId::parse("").unwrap_err(), ParseIdError::Empty);
    }

    #[test]
    fn uuiddigest_rejects_whitespace_only_string() {
        assert_eq!(
            UserId::parse("   ").unwrap_err(),
            ParseIdError::BadLength { got: 3 }
        );
    }

    #[test]
    fn uuiddigest_rejects_wrong_length() {
        for (input, want) in [("f81d4fae", 8usize), ("f81d4fae-7dec-11d0-a765", 23)] {
            assert_eq!(input.len(), want, "本用例的字面量长度与预期不符");
            assert_eq!(
                UserId::parse(input).unwrap_err(),
                ParseIdError::BadLength { got: want },
                "输入 {input:?} 的长度 {want} 应被判非法"
            );
        }

        let short = &HEX32[..31];
        assert_eq!(short.len(), 31, "切片长度应为 31");
        assert_eq!(
            UserId::parse(short).unwrap_err(),
            ParseIdError::BadLength { got: 31 }
        );
        let long = format!("{HEX32}0");
        assert_eq!(long.len(), 33, "拼接后长度应为 33");
        assert_eq!(
            UserId::parse(&long).unwrap_err(),
            ParseIdError::BadLength { got: 33 }
        );
    }

    #[test]
    fn uuiddigest_rejects_non_hex_character() {
        assert_eq!(
            UserId::parse("z81d4fae7dec11d0a76500a0c91e6bf6").unwrap_err(),
            ParseIdError::NotHex { index: 0 }
        );

        assert_eq!(
            UserId::parse("g81d4fae7dec11d0a76500a0c91e6bf6").unwrap_err(),
            ParseIdError::NotHex { index: 0 }
        );
    }

    #[test]
    fn uuiddigest_rejects_hyphen_in_compact_form() {
        let bad = "f81d4fae7dec11d0a765-0a0c91e6bf6";
        assert_eq!(bad.len(), 32, "本用例依赖输入长度恰为 32");
        assert_eq!(
            UserId::parse(bad).unwrap_err(),
            ParseIdError::HyphenInCompact
        );
    }

    #[test]
    fn uuiddigest_rejects_misplaced_hyphen_in_36_form() {
        let bad = "f81d4fa-7dec-11d0-a765-00a0c91e6bf6x";
        assert_eq!(bad.len(), 36, "本用例依赖输入长度恰为 36");
        assert_eq!(
            UserId::parse(bad).unwrap_err(),
            ParseIdError::MisplacedHyphen { index: 8 },
            "第 8 字节应为 `-`，实际是 `7`，故是位置错而非长度错"
        );

        let bad2 = "f81d4fae-7dec-11d0-a765-00a0c91e6bfx";
        assert_eq!(bad2.len(), 36, "本用例依赖输入长度恰为 36");
        assert_eq!(
            UserId::parse(bad2).unwrap_err(),
            ParseIdError::NotHex { index: 31 },
            "剥掉连字符后的 compact 形态第 31 位是 x"
        );
    }

    #[test]
    fn uuiddigest_roundtrip_hyphenated_is_reparseable() {
        let id = SessionId::parse(HYPH36).expect("应合法");
        let printed = id.0.to_hyphenated();
        assert_eq!(printed, HYPH36, "打印形态须与标准 uuid 一致");
        assert_eq!(SessionId::parse(&printed).unwrap(), id, "打印→重解析须幂等");
    }

    #[test]
    fn uuiddigest_display_shows_only_first_8_bytes() {
        let id = SessionId::parse(HEX32).expect("应合法");
        let shown = id.to_string();
        assert_eq!(
            shown, "s:f81d4fae7dec11d0…",
            "Display 必须是「类型前缀 + 前 8 字节」"
        );
        assert_eq!(shown.chars().count(), 19, "2 前缀 + 16 hex + 省略号");
        assert!(
            !shown.contains(&HEX32[16..]),
            "后 8 字节不得出现在 Display 里：日志会被 128 位铺满"
        );
    }

    #[test]
    fn uuiddigest_display_prefix_distinguishes_the_two_128_bit_types() {
        let a = UserId::from_bytes([0xab; 16]);
        let b = SessionId::from_bytes([0xab; 16]);
        assert_ne!(
            a.to_string(),
            b.to_string(),
            "两种类型的 Display 必须可区分"
        );
        assert!(a.to_string().starts_with("u:"));
        assert!(b.to_string().starts_with("s:"));

        assert_eq!(
            a.to_string()[2..],
            b.to_string()[2..],
            "除前缀外内容应一致（同一批字节）"
        );
    }

    #[test]
    fn uuiddigest_debug_shows_full_value_for_diagnosis() {
        let id = UserId::parse(HYPH36).expect("应合法");
        let dbg = format!("{id:?}");
        assert_eq!(dbg, format!("UserId({HYPH36})"), "Debug 必须打全量");
        assert!(
            dbg.starts_with("UserId(") && dbg.contains(HYPH36),
            "Debug 必须带上类型名，排障时能区分 UserId 与 SessionId"
        );
    }

    #[test]
    fn uuiddigest_display_prefix_is_distinguishable_between_two_ids() {
        let a = SessionId::parse(HEX32).expect("应合法");
        let b = SessionId::parse("00000000000000000000000000000001").expect("应合法");
        assert_ne!(a.to_string(), b.to_string());
        assert_ne!(a, b);
    }

    #[test]
    fn uuiddigest_from_bytes_equals_parse_of_same_hex() {
        let parsed = UserId::parse(HEX32).expect("应合法");
        let raw: [u8; 16] = *parsed.as_bytes();
        assert_eq!(UserId::from_bytes(raw), parsed, "两种构造入口须一致");
    }

    #[test]
    fn uuiddigest_error_display_names_the_actual_bad_input_shape() {
        let msg = ParseIdError::BadLength { got: 7 }.to_string();
        assert!(msg.contains('7'), "错误信息须含实际长度：{msg}");
        let msg2 = ParseIdError::NotHex { index: 4 }.to_string();
        assert!(msg2.contains('4'), "错误信息须含出错下标：{msg2}");
    }

    #[test]
    fn slug_accepts_lowercase_alnum_and_single_hyphen() {
        for good in ["openai", "cost-analyst", "a1", "x-9-y", "0123456789"] {
            assert!(
                ProviderId::parse(good).is_ok(),
                "{good:?} 是合法 kebab 标识，不该被拒"
            );
        }
    }

    #[test]
    fn expert_and_provider_with_same_text_are_distinct_types() {
        let e = ExpertId::parse("cost-analyst").expect("应合法");
        let p = ProviderId::parse("cost-analyst").expect("应合法");
        assert_eq!(e.as_str(), p.as_str(), "两者承载同一字符串");

        let p2 = ProviderId::try_from(e.as_str()).expect("字符串转 ProviderId 应可行");
        assert_eq!(p2, p);
    }

    #[test]
    fn member_id_for_expert_appends_sequence_and_stays_valid() {
        let e = ExpertId::parse("cost-analyst").expect("应合法");
        let m1 = MemberId::for_expert(&e, 1).expect("应合法");
        let m2 = MemberId::for_expert(&e, 2).expect("应合法");
        assert_eq!(m1.as_str(), "cost-analyst-1");
        assert_eq!(m2.as_str(), "cost-analyst-2");
        assert_ne!(m1, m2, "同一专家的不同执行实例必须是不同 MemberId");
    }

    #[test]
    fn member_id_for_expert_reports_error_when_result_exceeds_slug_limit() {
        let long = ExpertId::parse(&"a".repeat(MAX_SLUG_LEN)).expect("边界长度应合法");
        let err = MemberId::for_expert(&long, 42).unwrap_err();
        assert_eq!(
            err,
            ParseSlugError::TooLong {
                len: MAX_SLUG_LEN + 3,
                max: MAX_SLUG_LEN
            }
        );
    }

    #[test]
    fn slug_rejects_empty_and_whitespace_only() {
        assert_eq!(ExpertId::parse("").unwrap_err(), ParseSlugError::Empty);
        assert_eq!(ExpertId::parse("  ").unwrap_err(), ParseSlugError::Empty);
    }

    #[test]
    fn slug_rejects_uppercase() {
        assert!(matches!(
            ProviderId::parse("OpenAI"),
            Err(ParseSlugError::IllegalChar { index: 0, ch: 'O' })
        ));
    }

    #[test]
    fn slug_rejects_illegal_characters() {
        for (input, idx) in [("cost analyst", 4), ("cost_analyst", 4), ("a/b", 1)] {
            assert!(
                matches!(
                    ExpertId::parse(input),
                    Err(ParseSlugError::IllegalChar { index, .. }) if index == idx
                ),
                "{input:?} 应在第 {idx} 位被判非法"
            );
        }
    }

    #[test]
    fn slug_rejects_non_ascii() {
        let err = ExpertId::parse("成本").unwrap_err();
        match err {
            ParseSlugError::IllegalChar { index, ch } => {
                assert_eq!(index, 0);
                assert_eq!(ch, '成');
            }
            other => panic!("应为 IllegalChar，实际 {other:?}"),
        }
    }

    #[test]
    fn slug_rejects_hyphen_position_errors() {
        assert_eq!(
            MemberId::parse("-lead").unwrap_err(),
            ParseSlugError::LeadingHyphen
        );
        assert_eq!(
            MemberId::parse("trail-").unwrap_err(),
            ParseSlugError::TrailingHyphen
        );
        assert_eq!(
            MemberId::parse("a--b").unwrap_err(),
            ParseSlugError::DoubleHyphen { index: 2 }
        );
    }

    #[test]
    fn slug_rejects_over_length() {
        let s = "a".repeat(MAX_SLUG_LEN + 1);
        assert_eq!(
            ExpertId::parse(&s).unwrap_err(),
            ParseSlugError::TooLong {
                len: MAX_SLUG_LEN + 1,
                max: MAX_SLUG_LEN
            }
        );

        assert!(ExpertId::parse(&"a".repeat(MAX_SLUG_LEN)).is_ok());
    }

    #[test]
    fn slug_error_display_includes_offending_position() {
        let msg = ParseSlugError::DoubleHyphen { index: 7 }.to_string();
        assert!(msg.contains('7'), "错误信息须含出错位置：{msg}");
        let msg2 = ParseSlugError::IllegalChar { index: 2, ch: '_' }.to_string();
        assert!(msg2.contains('_'), "错误信息须含出错字符：{msg2}");
    }

    #[test]
    fn slug_display_and_debug_round_trip_the_slug() {
        let e = ExpertId::parse("cost-analyst").expect("应合法");
        assert_eq!(e.to_string(), "cost-analyst");
        assert_eq!(format!("{e:?}"), "ExpertId(cost-analyst)");
        let p = ProviderId::parse("openai").expect("应合法");
        assert_eq!(format!("{p:?}"), "ProviderId(openai)");
        let m = MemberId::parse("cost-analyst-1").expect("应合法");
        assert_eq!(format!("{m:?}"), "MemberId(cost-analyst-1)");
    }

    #[test]
    fn parse_attempts_are_countable_and_all_reach_a_verdict() {
        let slug_inputs = ["", "   ", "-x", "x-", "a--b", "A", "成本", &"a".repeat(65)];
        let n = slug_inputs.len();
        assert_eq!(n, 8, "已检查：slug 清单共 8 项");
        let mut ok = 0usize;
        let mut err = 0usize;
        for input in slug_inputs {
            match ExpertId::parse(input) {
                Ok(_) => ok += 1,
                Err(_) => err += 1,
            }
        }
        assert_eq!(ok + err, n, "每个输入都必须有判定，无漏判");
        assert_eq!(ok, 0, "本清单全是无效标识，不应有任何一个通过");
        assert_eq!(err, n, "已检查 {n} 个，{n} 个判红");

        let good = ["openai", "cost-analyst", "a1", "x-9-y"];
        let g = good.len();
        let mut ok2 = 0usize;
        for input in good {
            if ExpertId::parse(input).is_ok() {
                ok2 += 1;
            }
        }
        assert_eq!(ok2, g, "已检查 {g} 个合法标识，{g} 个应通过（不误报）");
    }
}
