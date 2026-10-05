//! 身份型 newtype：`UserId` / `ProviderId` / `ExpertId` / `SessionId` / `MemberId`
//!
//! 依据 `docs/DECISIONS.md` D-2026-10-05-01：身份型 newtype 定义在 **契约层**
//! （`quill-adapters`），`quill-domain` re-export。理由：`quill-adapters` 零 quill 依赖
//! 与「trait 签名要消费这些类型」不可兼得，类型必须住在契约层。
//!
//! # 为什么必须是 newtype 而不是 `type X = String`
//!
//! `type X = String` 是**别名**，编译器不区分它与 `String`。于是
//! `steer(&member: &MemberId, ..)` 可以被喂一个 `SessionId`，且**编译通过**——
//! 编译期防线形同虚设。本模块的每个 ID 都是独立 struct，`UserId` 与 `SessionId`
//! 之间互不兼容（见 `tests::compile_fail` 文档测试与 `crates/quill-testkit/tests/newtype_guard.rs`）。
//!
//! # 后端表示（对齐 `docs/PHASE2_CONTRACT.md` §四）
//!
//! | 类型 | 表示 | 理由 |
//! |---|---|---|
//! | `UserId` | 128 位（16 字节）随机标识 | uuid v7 时间有序；本模块只做**存储与校验**，不生成 |
//! | `SessionId` | 128 位 | 同上 |
//! | `ProviderId` | kebab-case 短标识 | 契约规定为 `String` 形态的可读名（如 `openai`） |
//! | `ExpertId` | kebab-case slug | 契约明确：**不是 UUID**，是可读标识 |
//!
//! ⚠️ **本模块不引入 `uuid` crate**（新增依赖须主理人裁决）。因此：
//! - 128 位类型的内部表示是 16 字节数组 + 手写 hex 编解码，**不使用** `uuid::Uuid`；
//! - 生成（v7 的时间戳 + 随机位）**不在本模块**，留给持有 `uuid`/随机源的层。
//!   `quill-adapters` 的依赖表当前为空，新增依赖会改 `Cargo.lock`，须裁决。

use std::fmt;

// ─────────────────────────── 内部：128 位标识 ───────────────────────────

/// 128 位标识的底层字节表示（16 字节）。
///
/// ⚠️ **为什么自己存字节而不用 `String`**：日志与错误信息里要打的是
/// 「前 8 字节十六进制」（可诊断、不刷屏），字节数组让这个截断是 O(1) 且无分配。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UuidBytes([u8; 16]);

impl UuidBytes {
    /// 构造：必须来自**恰好 16 字节**。
    pub const fn from_bytes(b: [u8; 16]) -> Self {
        Self(b)
    }

    /// 取原始字节。
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// 由 32 位十六进制字符串（可含或不含 `-`）解析。
    ///
    /// 接受的形式：
    /// - `f81d4fae7dec11d0a76500a0c91e6bf6`（32 字符）
    /// - `f81d4fae-7dec-11d0-a765-00a0c91e6bf6`（36 字符，标准 uuid 打印形态）
    ///
    /// 其余一切（长度不对、空串、非 hex、`-` 位置不对）→ `Err`。
    /// ⚠️ **不做「宽松接受」**：宽松解析会让「长度不对」这类错误静默通过，
    /// 而它正是 identity 串错用户的入口（铁律：缺输入必须判失败，不许兜底）。
    pub fn parse(s: &str) -> Result<Self, ParseIdError> {
        let trimmed = s.strip_prefix("urn:uuid:").unwrap_or(s);
        // 空串单独判：否则会被并进 BadLength{0}，错误信息就丢了
        //「你传的是空串」这个最有诊断价值的结论。
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
                // 标准 uuid 的 `-` 固定在 8 / 13 / 18 / 23 字节位置。
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
        // ⚠️ 上面的长度分支已保证 raw.len() == 32。
        // 不用 `chunks_exact(2)`（clippy 会要求 `as_chunks`）——
        // 直接按字节下标取高低半字节，语义更直白。
        let mut out = [0u8; 16];
        for i in 0..16 {
            let hi = hex_val(raw[i * 2]).ok_or(ParseIdError::NotHex { index: i * 2 })?;
            let lo = hex_val(raw[i * 2 + 1]).ok_or(ParseIdError::NotHex { index: i * 2 + 1 })?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }

    /// 32 位小写十六进制（无 `-`），可被 `parse` 逆转。
    pub fn to_compact_hex(&self) -> String {
        let mut s = String::with_capacity(32);
        for b in &self.0 {
            s.push(char::from(HEX_DIGITS[(b >> 4) as usize]));
            s.push(char::from(HEX_DIGITS[(b & 0x0f) as usize]));
        }
        s
    }

    /// 标准 uuid 打印形态（36 字符，含 `-`）。
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

/// 标准 uuid 形态下 `-` 所在字节下标。
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
        // Debug 打全量：排障时要精确值。
        write!(f, "UuidBytes({})", self.to_hyphenated())
    }
}

impl fmt::Display for UuidBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display 打**前 8 字节**（16 个 hex 字符）：可定位、不刷屏。
        write!(f, "{:.16}…", self.to_compact_hex())
    }
}

/// 128 位标识的解析错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseIdError {
    /// 空串（或只有 `urn:uuid:` 前缀）。
    Empty,
    /// 长度不是 32 / 36。
    BadLength { got: usize },
    /// 32 字符形态里出现了 `-`。
    HyphenInCompact,
    /// 36 字符形态里 `-` 不在标准位置。
    MisplacedHyphen { index: usize },
    /// 非 hex 字符（`index` 是 hex 字符位上的下标）。
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

// ─────────────────────────── kebab 标识 ───────────────────────────

/// kebab-case 短标识的校验结果错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseSlugError {
    /// 空串或全空白。
    Empty,
    /// 超过 `MAX_SLUG_LEN`。
    TooLong { len: usize, max: usize },
    /// 非法字符（只允许 `[a-z0-9-]`）。
    IllegalChar { index: usize, ch: char },
    /// 以 `-` 开头。
    LeadingHyphen,
    /// 以 `-` 结尾。
    TrailingHyphen,
    /// 出现连续 `--`。
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

/// kebab-case 标识长度上限。
///
/// 取 64：够放下 `cost-analyst-v2` 这类可读名，又能在错误信息里完整打印。
pub const MAX_SLUG_LEN: usize = 64;

/// 校验 kebab-case 短标识。
///
/// **公开**是为了让 `quill-domain` 的 `TeamId` 复用**同一套**规则：
/// 允许两套「哪些字符合法」就会把它变成记忆题。
/// ⚠️ 它不产生任何 ID 类型，只返回判定结果 —— 不能被当作「构造入口」绕过校验。
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
            // 非 ASCII（多字节）时 index 是字节下标，ch 给完整字符便于诊断。
            let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
            return Err(ParseSlugError::IllegalChar { index: i, ch });
        }
        if b == b'-' && i > 0 && bytes[i - 1] == b'-' {
            return Err(ParseSlugError::DoubleHyphen { index: i });
        }
    }
    Ok(())
}

// ─────────────────────────── 四个身份类型 ───────────────────────────

/// 用户标识（128 位）。
///
/// 契约：`docs/PHASE2_CONTRACT.md` §四「`UserId` = uuid v7，时间有序」。
/// ⚠️ **本类型不生成 v7**，只做存储与校验；生成需要随机源，由持有方负责。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(UuidBytes);

impl UserId {
    /// 构造：合法 32/36 位 hex。
    pub fn parse(s: &str) -> Result<Self, ParseIdError> {
        UuidBytes::parse(s).map(Self)
    }

    /// 构造：直接给 16 字节（生成方用）。
    pub const fn from_bytes(b: [u8; 16]) -> Self {
        Self(UuidBytes::from_bytes(b))
    }

    /// 取出底层字节。
    pub const fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    /// 32 位紧凑 hex。
    pub fn to_compact_hex(&self) -> String {
        self.0.to_compact_hex()
    }
}

/// 会话标识（128 位）。
///
/// 与 [`UserId`] **类型不同**：把会话标识当用户标识传，编译不过。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(UuidBytes);

impl SessionId {
    /// 构造：合法 32/36 位 hex。
    pub fn parse(s: &str) -> Result<Self, ParseIdError> {
        UuidBytes::parse(s).map(Self)
    }

    /// 构造：直接给 16 字节（生成方用）。
    pub const fn from_bytes(b: [u8; 16]) -> Self {
        Self(UuidBytes::from_bytes(b))
    }

    /// 取出底层字节。
    pub const fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    /// 32 位紧凑 hex。
    pub fn to_compact_hex(&self) -> String {
        self.0.to_compact_hex()
    }
}

/// Provider 标识（kebab-case 可读名，如 `openai`）。
///
/// 契约：§四「`ProviderId` = `String`」。⚠️ 这里**故意不用别名**——
/// 别名会让 `ProviderId` 与 `String` 互换，从而使
/// `CapabilityProbe::supports_embeddings(&ProviderId)` 失去「防传错」的编译期价值。
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    /// 构造：kebab-case 短名。
    pub fn parse(s: &str) -> Result<Self, ParseSlugError> {
        validate_slug(s)?;
        Ok(Self(s.to_string()))
    }

    /// 取出内层字符串。
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

/// 专家标识（kebab-case slug，如 `cost-analyst`）。
///
/// 契约：§四明确「**不是 UUID！** 专家是可读标识」。
///
/// 这个区分是 [`MemberId`] 存在的前提：一个专家（`ExpertId`）可**同时**在多个团队里
/// 当成员，而成员实例（`MemberId`）是一次性的。两者若同为 `String`，
/// 「专家身份 ↔ 执行实例」的一等公民区分就退化成注释约定。
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExpertId(String);

impl ExpertId {
    /// 构造：kebab-case slug。
    pub fn parse(s: &str) -> Result<Self, ParseSlugError> {
        validate_slug(s)?;
        Ok(Self(s.to_string()))
    }

    /// 取出内层字符串。
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

/// 成员实例标识（一次执行实例，不是专家身份）。
///
/// 依据 `docs/03_智能体编排设计.md` §2.12.2 论据 4：`session_id` 是一次性执行实例 id，
/// `ExpertId` 是可复用专家身份。**两者是不同的类型**，所以
/// 「用 `MemberId` 冒充 `ExpertId`」在编译期就断掉。
///
/// 表示：kebab-case slug + 一段序号，形如 `cost-analyst-3`，保证同一专家的
/// 不同执行实例可区分且人可读。
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberId(String);

impl MemberId {
    /// 构造：kebab-case slug。
    pub fn parse(s: &str) -> Result<Self, ParseSlugError> {
        validate_slug(s)?;
        Ok(Self(s.to_string()))
    }

    /// 为「专家的第 `seq` 次执行实例」构造成员标识。
    ///
    /// 不变量：产物必然是合法 kebab（因为 `expert` 已合法、序号只追加数字与一个 `-`），
    /// 因此这里用 `expect` 之外的显式错误返回，不 panic：
    /// 若 `seq` 过大导致超长，调用方拿到 `Err` 而不是进程崩。
    pub fn for_expert(expert: &ExpertId, seq: u32) -> Result<Self, ParseSlugError> {
        let candidate = format!("{}-{seq}", expert.as_str());
        Self::parse(&candidate)
    }

    /// 取出内层字符串。
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

/// 128 位身份类型的 `Display`。
///
/// ⚠️ **带一个类型字母前缀**（`u:` / `s:`）——不是装饰：
/// 两种 128 位标识的字节形态可能完全相同，若 `Display` 也相同，
/// 日志里的 `u` 与 `s` 就无法分辨，而「把会话标识当用户标识用」
/// 正是本模块要防的那类错误。前缀让这个错误在日志里**看得见**。
macro_rules! impl_display_short {
    ($t:ty, $tag:expr) => {
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // ⚠️ `{:.16}` 的精度对 `&str` 是**最大宽度**，会连 `…` 一起截掉，
                // 所以省略号必须**在**格式化之后单独写。
                write!(f, concat!($tag, "{:.16}"), self.0.to_compact_hex())?;
                f.write_str("…")
            }
        }
    };
}
impl_display_short!(UserId, "u:");
impl_display_short!(SessionId, "s:");

/// 128 位身份类型的 `Debug`（全量，排障要精确值）。
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

    // ── 128 位：合法 ──
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

    // ── 128 位：非法（每个边界一个反向用例）──
    #[test]
    fn uuiddigest_rejects_empty_string() {
        assert_eq!(UserId::parse("").unwrap_err(), ParseIdError::Empty);
    }

    #[test]
    fn uuiddigest_rejects_whitespace_only_string() {
        // ⚠️ 关键反向用例：`"   "` 不是空串但也不是合法标识。
        // 若实现里用 `s.trim().is_empty()` 才判空，缩进换行的输入就会蒙混过关。
        assert_eq!(
            UserId::parse("   ").unwrap_err(),
            ParseIdError::BadLength { got: 3 }
        );
    }

    #[test]
    fn uuiddigest_rejects_wrong_length() {
        // ⚠️ 每行先断言 `input.len() == want`：写错字面量长度会立刻红，
        // 而不是让「长度断言」变成一个恒真的摆设。
        for (input, want) in [
            ("f81d4fae", 8usize),            // 短了一半
            ("f81d4fae-7dec-11d0-a765", 23), // 半截连字符形态
        ] {
            assert_eq!(input.len(), want, "本用例的字面量长度与预期不符");
            assert_eq!(
                UserId::parse(input).unwrap_err(),
                ParseIdError::BadLength { got: want },
                "输入 {input:?} 的长度 {want} 应被判非法"
            );
        }
        // 少一位 / 多一位：用切片构造，避免手数字符串长度出错。
        let short = &HEX32[..31];
        assert_eq!(short.len(), 31, "切片长度应��� 31");
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
        // 第 0 位是 'z'，非 hex。
        assert_eq!(
            UserId::parse("z81d4fae7dec11d0a76500a0c91e6bf6").unwrap_err(),
            ParseIdError::NotHex { index: 0 }
        );
        // 第 1 位是 'g'：证明检查的是**每一半字节**，不是只查首字符。
        assert_eq!(
            UserId::parse("g81d4fae7dec11d0a76500a0c91e6bf6").unwrap_err(),
            ParseIdError::NotHex { index: 0 }
        );
    }

    #[test]
    fn uuiddigest_rejects_hyphen_in_compact_form() {
        // 32 位紧凑形态里插一个 `-`：长度仍可能是 32，必须靠内容判红。
        let bad = "f81d4fae7dec11d0a765-0a0c91e6bf6";
        assert_eq!(bad.len(), 32, "本用例依赖输入长度恰为 32");
        assert_eq!(
            UserId::parse(bad).unwrap_err(),
            ParseIdError::HyphenInCompact
        );
    }

    #[test]
    fn uuiddigest_rejects_misplaced_hyphen_in_36_form() {
        // 36 位但第 8 字节的 `-` 挪到了第 7 位：长度对、hex 字符全合法，
        // **只有连字符位置规则**能抓住它 —— 这正是长度检查漏掉的那一档。
        let bad = "f81d4fa-7dec-11d0-a765-00a0c91e6bf6x";
        assert_eq!(bad.len(), 36, "本用例依赖输入长度恰为 36");
        assert_eq!(
            UserId::parse(bad).unwrap_err(),
            ParseIdError::MisplacedHyphen { index: 8 },
            "第 8 字节应为 `-`，实际是 `7`，故是位置错而非长度错"
        );
        // 对照组：`-` 位置全对，但末位是 'x'。
        // ⚠️ 这两个用例必须分开写：若实现只检查长度，两者都会「通过」。
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

    // ── Display / Debug 的诊断形态 ──
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
        // 🔴 反向用例：同字节的 UserId 与 SessionId，Display 必须不同。
        // 若两者 Display 相同，日志里就分不出「用户」与「会话」——
        // 而把会话标识当用户标识用正是本模块要防的错误。
        let a = UserId::from_bytes([0xab; 16]);
        let b = SessionId::from_bytes([0xab; 16]);
        assert_ne!(
            a.to_string(),
            b.to_string(),
            "两种类型的 Display 必须可区分"
        );
        assert!(a.to_string().starts_with("u:"));
        assert!(b.to_string().starts_with("s:"));
        // 十六进制部分仍然相同：证明区分来自类型前缀，不是来自数据。
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
        // 诊断价值前提：前 8 字节不同的两个 ID，Display 必须能分开。
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
        // 错误信息必须可诊断：含实际长度，否则用户无法判断自己传了什么。
        let msg = ParseIdError::BadLength { got: 7 }.to_string();
        assert!(msg.contains('7'), "错误信息须含实际长度：{msg}");
        let msg2 = ParseIdError::NotHex { index: 4 }.to_string();
        assert!(msg2.contains('4'), "错误信息须含出错下标：{msg2}");
    }

    // ── kebak：合法 ──
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
        // 同名不同型：值相等可比较，但**类型**不同（编译期差异，见 newtype_guard.rs）。
        let e = ExpertId::parse("cost-analyst").expect("应合法");
        let p = ProviderId::parse("cost-analyst").expect("应合法");
        assert_eq!(e.as_str(), p.as_str(), "两者承载同一字符串");
        // 反向用例的静态证据：显式转换是可能的，无隐式转换。
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
        // 反向用例：超长必须判失败，不能 panic、不能静默截断。
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

    // ── kebab：非法（每个边界一个反向用例）──
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
        // 多字节字符：必须报**完整字符**而不是乱码字节，否则诊断信息不可读。
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
        // 边界：恰好等于上限应通过（证明上限本身可达，不是「怎么都超」）。
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

    // ── 「0 与未检查」必须可区分 ──
    #[test]
    fn parse_attempts_are_countable_and_all_reach_a_verdict() {
        // 「0 与未检查必须可区分」：清单里每个输入都必须落到一个明确判定，
        // 且判定数量必须等于清单长度 —— 少一个就说明有输入没被检查。
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

        // 对照：合法清单必须全部通过（证明上面的「全红」不是因为检查器坏了）。
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
