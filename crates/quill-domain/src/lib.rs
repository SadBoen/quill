
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

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TeamId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamIdError {

    Empty,

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

    pub fn parse(s: &str) -> Result<Self, TeamIdError> {
        if s.trim().is_empty() {
            return Err(TeamIdError::Empty);
        }

        quill_adapters::ids::validate_slug(s).map_err(TeamIdError::Invalid)?;
        Ok(Self(s.to_string()))
    }

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

        let err = TeamId::parse("Growth Squad").unwrap_err();
        match err {
            TeamIdError::Invalid(ParseSlugError::IllegalChar { index, ch }) => {

                assert_eq!(index, 0, "首个非法字符是大写 G");
                assert_eq!(ch, 'G');
            }
            other => panic!("应为 Invalid(IllegalChar)，实际 {other:?}"),
        }

        assert!(TeamId::parse("growth-squad").is_ok(), "合法团队名应通过");
    }

    #[test]
    fn reexported_ids_are_the_same_types_as_contract_layer() {

        let u = quill_adapters::ids::UserId::from_bytes([7; 16]);
        let v: UserId = u;
        assert_eq!(v.as_bytes(), &[7u8; 16]);
    }
}
