//! 领域模型 —— 身份类型 re-export + 领域实体。
//!
//! # 身份类型不在本 crate 定义（`docs/DECISIONS.md` D-2026-10-05-01）
//!
//! `UserId` / `ProviderId` / `ExpertId` / `SessionId` 定义在契约层 `quill-adapters`，
//! 本 crate `pub use` re-export，使 `quill_domain::UserId` 这类调用路径不破。
//! 理由：契约 trait 的签名要消费这些类型，而 `quill-adapters` 必须零 quill 依赖——
//! 类型只能住在契约层。
//!
//! ⚠️ **这是 re-export，不是别名**：`quill_domain::UserId` 与 `quill_adapters::UserId`
//! 是**同一个类型**，所以跨 crate 传递不需要转换；但它仍**不是** `String`，
//! 误用依然编译不过（见 `crates/quill-testkit/tests/newtype_guard.rs`）。

pub mod team;

pub use quill_adapters::ids::{
    ExpertId, MemberId, ParseIdError, ParseSlugError, ProviderId, SessionId, UserId, UuidBytes,
    MAX_SLUG_LEN,
};
pub use quill_adapters::member::{
    AbortScope, AdapterError, ChainCheck, ChainHop, MemberExecutor, MemberOutcome,
    MemberStartRequest, MemberStatus, Message, MessageRole,
};
pub use team::{AddOutcome, Team, TeamError, MAX_TEAM_MEMBERS};

/// 团队标识（领域实体 id，**不进契约签名**故住在这里）。
///
/// ⚠️ 与 `MemberId` 区分：`TeamId` 是团队身份（稳定、可复用），
/// `MemberId` 是一次执行实例。两者若同为 `String`，「团队 ↔ 实例」就分不清。
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TeamId(String);

/// 团队标识的构造错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamIdError {
    /// 空串或全空白。
    Empty,
    /// 非法字符 / 长度超限（复用 slug 校验口径）。
    Invalid(ParseSlugError),
}

impl std::fmt::Display for TeamIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("团队标识为空"),
            Self::Invalid(e) => write!(f, "团队标识非法：{e}"),
        }
    }
}

impl std::error::Error for TeamIdError {}

impl TeamId {
    /// 构造：kebab-case 短名。
    pub fn parse(s: &str) -> Result<Self, TeamIdError> {
        if s.trim().is_empty() {
            return Err(TeamIdError::Empty);
        }
        // 复用契约层的 slug 口径：团队名与专家名同为可读标识，
        // 允许两套规则就会让「哪些字符合法」变成记忆题。
        quill_adapters::ids::validate_slug(s).map_err(TeamIdError::Invalid)?;
        Ok(Self(s.to_string()))
    }

    /// 取出内层字符串。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TeamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Debug for TeamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TeamId({})", self.0)
    }
}

impl TryFrom<&str> for TeamId {
    type Error = TeamIdError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_id_accepts_kebab_slug() {
        let t = TeamId::parse("growth-squad").expect("应合法");
        assert_eq!(t.as_str(), "growth-squad");
        assert_eq!(t.to_string(), "growth-squad");
        assert_eq!(format!("{t:?}"), "TeamId(growth-squad)");
    }

    #[test]
    fn team_id_rejects_empty_and_whitespace() {
        assert_eq!(TeamId::parse("").unwrap_err(), TeamIdError::Empty);
        assert_eq!(TeamId::parse("   ").unwrap_err(), TeamIdError::Empty);
    }

    #[test]
    fn team_id_rejects_illegal_slug_reusing_contract_rules() {
        // 复用契约层规则意味着错误**形状**也一致（同一个 `ParseSlugError`），
        // 而不只是「也判红」。
        let err = TeamId::parse("Growth Squad").unwrap_err();
        match err {
            TeamIdError::Invalid(ParseSlugError::IllegalChar { index, ch }) => {
                // 大写 G 在第 0 位：证明规则真的来自契约层的 slug 校验，
                // 而不是本地又写了一套更宽松的。
                assert_eq!(index, 0, "首个非法字符是大写 G");
                assert_eq!(ch, 'G');
            }
            other => panic!("应为 Invalid(IllegalChar)，实际 {other:?}"),
        }
        // 对照：合法团队名必须通过（证明不是「一律拒绝」）。
        assert!(TeamId::parse("growth-squad").is_ok(), "合法团队名应通过");
    }

    #[test]
    fn reexported_ids_are_the_same_types_as_contract_layer() {
        // 关键性质：re-export 而非各自定义。若两边各定义一个同名 struct，
        // 跨 crate 传值就需要转换 —— 下面这个「无需转换」的编译事实就是证据。
        let u = quill_adapters::ids::UserId::from_bytes([7; 16]);
        let v: UserId = u;
        assert_eq!(v.as_bytes(), &[7u8; 16]);
    }
}
