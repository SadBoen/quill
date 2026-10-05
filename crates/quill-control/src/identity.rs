use sha2::{Digest, Sha256};

use crate::user::normalize_username;
use crate::ControlError;
use crate::UserId;

pub fn invalid_identity(detail: impl Into<String>) -> ControlError {
    ControlError::InvariantBroken {
        detail: detail.into(),
    }
}

/// 全项目唯一的「用户名 → 用户 ID」推导。
///
/// 服务端（QUILL_TOKENS 里写 @alice）和命令行（--as alice）都必须走这里，
/// 否则同一个人会在库里留下两行，所有跨用户外键还会指向一个不存在的用户。
pub fn derive_user_id(name: &str) -> Result<UserId, ControlError> {
    let norm = normalize_username(name);
    if norm.is_empty() {
        return Err(invalid_identity("用户名归一后为空串"));
    }
    let digest = Sha256::digest(format!("quill-user:{norm}").as_bytes());
    let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    UserId::parse(&hex).map_err(|e| invalid_identity(format!("由用户名 {name:?} 派生的 ID 非法：{e}")))
}

/// 令牌条目里写的身份：显式 ID，或以 `@` 开头的用户名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenSubject {
    Id(UserId),

    Named(String),
}

impl TokenSubject {
    pub fn resolve(&self) -> Result<UserId, ControlError> {
        match self {
            Self::Id(id) => Ok(*id),
            Self::Named(n) => derive_user_id(n),
        }
    }
}

pub fn parse_token_subject(raw: &str) -> Result<TokenSubject, ControlError> {
    let t = raw.trim();
    if let Some(name) = t.strip_prefix('@') {
        if name.trim().is_empty() {
            return Err(invalid_identity("令牌身份写成了 `@`，后面没跟用户名"));
        }
        Ok(TokenSubject::Named(name.trim().to_string()))
    } else {
        UserId::parse(t)
            .map(TokenSubject::Id)
            .map_err(|e| invalid_identity(format!("令牌身份 {t:?} 既不是 @用户名 也不是合法 ID：{e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPLICIT: &str = "0192b7c8-0000-7000-8000-000000000001";

    #[test]
    fn same_name_always_derives_the_same_id() {
        assert_eq!(
            derive_user_id("alice").expect("alice"),
            derive_user_id("alice").expect("alice"),
            "同名必须同 ID，否则 CLI 与服务端会分裂成两个人"
        );
    }

    #[test]
    fn derivation_is_case_and_space_insensitive_like_login() {
        assert_eq!(
            derive_user_id("Alice").expect("大写"),
            derive_user_id("  alice  ").expect("带空格"),
            "大小写/空白归一后必须与登录口径一致"
        );
    }

    #[test]
    fn different_names_diverge() {
        assert_ne!(
            derive_user_id("alice").expect("a"),
            derive_user_id("bob").expect("b")
        );
    }

    #[test]
    fn cli_and_token_named_subject_resolve_to_the_same_person() {
        let via_cli = derive_user_id("alice").expect("cli");
        let via_token = parse_token_subject("@alice")
            .expect("解析令牌身份")
            .resolve()
            .expect("解析出 ID");
        assert_eq!(via_cli, via_token, "CLI 与服务端令牌必须指向同一个用户行");
    }

    #[test]
    fn explicit_id_form_still_parses_and_is_not_reinterpreted() {
        let s = parse_token_subject(EXPLICIT).expect("显式 ID");
        assert_eq!(s, TokenSubject::Id(UserId::parse(EXPLICIT).unwrap()));
    }

    #[test]
    fn at_sign_without_name_is_rejected() {
        assert!(parse_token_subject("@").is_err());
        assert!(parse_token_subject("@  ").is_err());
    }

    #[test]
    fn garbage_subject_is_rejected_rather_than_silently_defaulted() {
        assert!(parse_token_subject("not-a-user").is_err());
        assert!(parse_token_subject("").is_err());
    }
}
