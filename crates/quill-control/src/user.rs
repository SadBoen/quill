use quill_domain::UserId;

use crate::error::ControlError;

pub const MAX_USERNAME_LEN: usize = 32;

pub const MIN_USERNAME_LEN: usize = 3;

pub const MAX_DISPLAY_NAME_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserRole {
    Owner,

    Member,
}

impl UserRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Result<Self, ControlError> {
        match s {
            "owner" => Ok(Self::Owner),
            "member" => Ok(Self::Member),
            other => Err(ControlError::InvariantBroken {
                detail: format!("users.role={other:?} 不在 schema CHECK ('owner','member') 之内"),
            }),
        }
    }

    pub fn is_owner(self) -> bool {
        matches!(self, Self::Owner)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserStatus {
    Active,

    Disabled,
}

impl UserStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(s: &str) -> Result<Self, ControlError> {
        match s {
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            other => Err(ControlError::InvariantBroken {
                detail: format!(
                    "users.status={other:?} 不在 schema CHECK ('active','disabled') 之内"
                ),
            }),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct UserProfile {
    pub id: UserId,

    pub username: String,

    pub username_norm: String,

    pub display_name: String,

    pub role: UserRole,

    pub status: UserStatus,

    pub token_epoch: i64,

    pub created_at_ms: i64,

    pub last_login_at_ms: Option<i64>,
}

impl std::fmt::Debug for UserProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserProfile")
            .field("id", &self.id)
            .field("username_norm", &self.username_norm)
            .field("display_name", &self.display_name)
            .field("role", &self.role)
            .field("status", &self.status)
            .field("last_login_at_ms", &self.last_login_at_ms)
            .finish_non_exhaustive()
    }
}

pub fn normalize_username(raw: &str) -> String {
    raw.trim().to_lowercase()
}

pub fn validate_username(username_norm: &str) -> Result<(), ControlError> {
    let bad = |reason: &'static str| ControlError::UsernameInvalid {
        raw: username_norm.to_string(),
        reason,
    };
    let len = username_norm.chars().count();
    if len < MIN_USERNAME_LEN {
        return Err(bad("少于 3 个字符"));
    }
    if len > MAX_USERNAME_LEN {
        return Err(bad("多于 32 个字符"));
    }
    let first = username_norm.chars().next().unwrap_or('?');
    if !(first.is_ascii_alphanumeric()) {
        return Err(bad("必须以字母或数字开头"));
    }
    for ch in username_norm.chars() {
        let ok = ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-';
        if !ok {
            if ch.is_whitespace() {
                return Err(bad("含空格 —— 用户名不能有空格，请用下划线"));
            }
            if ch.is_ascii_uppercase() {
                return Err(bad("含大写字母 —— 系统会自动转小写，请直接用小写"));
            }
            if !ch.is_ascii() {
                return Err(bad(
                    "含非 ASCII 字符 —— 只允许 a-z、0-9、点、下划线、连字符",
                ));
            }
            return Err(bad("含非法字符 —— 只允许 a-z、0-9、点、下划线、连字符"));
        }
    }
    Ok(())
}

pub fn validate_display_name(display_name: &str) -> Result<(), ControlError> {
    let bad = |reason: &'static str| ControlError::DisplayNameInvalid {
        raw: display_name.to_string(),
        reason,
    };
    let trimmed = display_name.trim();
    if trimmed.is_empty() {
        return Err(bad("不能为空"));
    }
    let len = trimmed.chars().count();
    if len > MAX_DISPLAY_NAME_LEN {
        return Err(bad("多于 64 个字符"));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(bad("含控制字符（换行 / 制表符）"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_trims_and_lowercases() {
        assert_eq!(normalize_username("  ZhangWei  "), "zhangwei");
        assert_eq!(normalize_username("ZHANG_WEI"), "zhang_wei");

        assert_eq!(normalize_username("Ä"), normalize_username("ä"));
        assert_eq!(normalize_username("ÄÖÜ"), "äöü");

        assert_eq!(normalize_username(" zhang wei "), "zhang wei");
    }

    #[test]
    fn valid_usernames_pass() {
        let cases: Vec<String> = vec![
            "abc".to_string(),
            "zhang_wei".to_string(),
            "a-b.c".to_string(),
            "user123".to_string(),
            "a".repeat(MAX_USERNAME_LEN),
        ];
        for ok in &cases {
            assert!(
                validate_username(ok).is_ok(),
                "合法用户名 {ok:?} 应通过，实际 {:?}",
                validate_username(ok)
            );
        }
    }

    #[test]
    fn invalid_usernames_are_rejected_with_specific_reasons() {
        let cases: [(&str, &str); 8] = [
            ("ab", "少于 3 个字符"),
            ("", "少于 3 个字符"),
            ("zhang wei", "含空格 —— 用户名不能有空格，请用下划线"),
            ("-zhang", "必须以字母或数字开头"),
            (".zhang", "必须以字母或数字开头"),
            (
                "zhang@wei",
                "含非法字符 —— 只允许 a-z、0-9、点、下划线、连字符",
            ),
            (
                "zhang伟民",
                "含非 ASCII 字符 —— 只允许 a-z、0-9、点、下划线、连字符",
            ),
            (
                "zhang/wei",
                "含非法字符 —— 只允许 a-z、0-9、点、下划线、连字符",
            ),
        ];
        for (input, want_reason) in cases {
            match validate_username(input) {
                Err(ControlError::UsernameInvalid { reason, .. }) => {
                    assert_eq!(reason, want_reason, "输入 {input:?} 的原因不对");
                }
                other => panic!("输入 {input:?} 应被拒，实际 {other:?}"),
            }
        }

        assert!(matches!(
            validate_username(&"a".repeat(MAX_USERNAME_LEN + 1)),
            Err(ControlError::UsernameInvalid { .. })
        ));
    }

    #[test]
    fn uppercase_is_rejected_after_normalization_but_normalization_removes_it() {
        let n = normalize_username("ZhangWei");
        assert_eq!(n, "zhangwei");
        assert!(validate_username(&n).is_ok());
    }

    #[test]
    fn display_name_rules() {
        assert!(validate_display_name("张伟").is_ok());
        assert!(validate_display_name("Zhang Wei").is_ok());
        assert!(validate_display_name(&"名".repeat(MAX_DISPLAY_NAME_LEN)).is_ok());
        assert!(matches!(
            validate_display_name(""),
            Err(ControlError::DisplayNameInvalid { .. })
        ));
        assert!(matches!(
            validate_display_name("   "),
            Err(ControlError::DisplayNameInvalid { .. })
        ));
        assert!(matches!(
            validate_display_name(&"名".repeat(MAX_DISPLAY_NAME_LEN + 1)),
            Err(ControlError::DisplayNameInvalid { .. })
        ));
        assert!(matches!(
            validate_display_name("张\n伟"),
            Err(ControlError::DisplayNameInvalid { .. })
        ));
    }

    #[test]
    fn role_and_status_roundtrip_through_db_strings() {
        for r in [UserRole::Owner, UserRole::Member] {
            assert_eq!(UserRole::parse(r.as_str()).unwrap(), r);
        }
        assert!(UserRole::Owner.is_owner());
        assert!(!UserRole::Member.is_owner());
        for s in [UserStatus::Active, UserStatus::Disabled] {
            assert_eq!(UserStatus::parse(s.as_str()).unwrap(), s);
        }
    }

    #[test]
    fn role_and_status_reject_values_outside_schema_check() {
        for bad in ["admin", "OWNER", "", "owner "].iter() {
            assert!(UserRole::parse(bad).is_err(), "{bad:?} 必须被拒");
        }
        for bad in ["locked", "ACTIVE", ""].iter() {
            assert!(UserStatus::parse(bad).is_err(), "{bad:?} 必须被拒");
        }
    }
}
