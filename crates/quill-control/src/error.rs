use std::fmt;

use quill_adapters::AdapterError;

pub const DOCTOR_CMD: &str = "quill doctor";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    UsernameInvalid {
        raw: String,

        reason: &'static str,
    },

    UsernameTaken {
        username_norm: String,
    },

    DisplayNameInvalid {
        raw: String,

        reason: &'static str,
    },

    PasswordTooShort {
        len: usize,

        min: usize,
    },

    PasswordTooLong {
        len: usize,

        max: usize,
    },

    PasswordEqualsUsername,

    CredentialsRejected,

    AccountDisabled {
        username_norm: String,
    },

    AccountLocked {
        username_norm: String,

        until_ms: i64,
    },

    NotAnOwner {
        operation: &'static str,
    },

    FirstOwnerExists,

    TokenMalformed,

    SessionUnknown,

    SessionExpired {
        expired_at_ms: i64,
    },

    SessionRevoked {
        reason: String,
    },

    SessionReuseDetected {
        family: String,
    },

    InviteMalformed,

    InviteUnknown,

    InviteExpired {
        expired_at_ms: i64,
    },

    InviteExhausted {
        max_uses: i32,
    },

    InviteRevoked,

    UserNotFound,

    SelfDisableForbidden,

    InviteUsesOutOfRange {
        got: i32,

        max: i32,
    },

    InvariantBroken {
        detail: String,
    },

    Storage {
        detail: String,
    },
}

impl ControlError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UsernameInvalid { .. } => "username_invalid",
            Self::UsernameTaken { .. } => "username_taken",
            Self::DisplayNameInvalid { .. } => "display_name_invalid",
            Self::PasswordTooShort { .. } => "password_too_short",
            Self::PasswordTooLong { .. } => "password_too_long",
            Self::PasswordEqualsUsername => "password_equals_username",
            Self::CredentialsRejected => "credentials_rejected",
            Self::AccountDisabled { .. } => "account_disabled",
            Self::AccountLocked { .. } => "account_locked",
            Self::NotAnOwner { .. } => "not_an_owner",
            Self::FirstOwnerExists => "first_owner_exists",
            Self::TokenMalformed => "token_malformed",
            Self::SessionUnknown => "session_unknown",
            Self::SessionExpired { .. } => "session_expired",
            Self::SessionRevoked { .. } => "session_revoked",
            Self::SessionReuseDetected { .. } => "session_reuse_detected",
            Self::InviteMalformed => "invite_malformed",
            Self::InviteUnknown => "invite_unknown",
            Self::InviteExpired { .. } => "invite_expired",
            Self::InviteExhausted { .. } => "invite_exhausted",
            Self::InviteRevoked => "invite_revoked",
            Self::UserNotFound => "user_not_found",
            Self::SelfDisableForbidden => "self_disable_forbidden",
            Self::InviteUsesOutOfRange { .. } => "invite_uses_out_of_range",
            Self::InvariantBroken { .. } => "invariant_broken",
            Self::Storage { .. } => "storage_error",
        }
    }

    pub fn fix_command(&self) -> String {
        match self {
            Self::UsernameInvalid { .. }
            | Self::UsernameTaken { .. }
            | Self::PasswordTooShort { .. }
            | Self::PasswordTooLong { .. }
            | Self::PasswordEqualsUsername
            | Self::DisplayNameInvalid { .. } => {
                format!("{DOCTOR_CMD} --section=users")
            }
            Self::CredentialsRejected => format!("{DOCTOR_CMD} --section=auth"),
            Self::AccountDisabled { .. } => format!("{DOCTOR_CMD} --section=users"),
            Self::AccountLocked { .. } => format!("{DOCTOR_CMD} --section=auth"),
            Self::NotAnOwner { .. } | Self::FirstOwnerExists => {
                format!("{DOCTOR_CMD} --section=users")
            }
            Self::TokenMalformed
            | Self::SessionUnknown
            | Self::SessionExpired { .. }
            | Self::SessionRevoked { .. }
            | Self::SessionReuseDetected { .. } => {
                format!("{DOCTOR_CMD} --section=auth")
            }
            Self::InviteMalformed
            | Self::InviteUnknown
            | Self::InviteExpired { .. }
            | Self::InviteExhausted { .. }
            | Self::InviteRevoked
            | Self::InviteUsesOutOfRange { .. } => {
                format!("{DOCTOR_CMD} --section=invites")
            }
            Self::UserNotFound | Self::SelfDisableForbidden => {
                format!("{DOCTOR_CMD} --section=users")
            }
            Self::InvariantBroken { .. } | Self::Storage { .. } => {
                format!("{DOCTOR_CMD} --section=db")
            }
        }
    }
}

fn tail(f: &mut fmt::Formatter<'_>, cmd: &str) -> fmt::Result {
    write!(
        f,
        "\n→ 下一步：复制执行 `{cmd}`（它会打印本错误的完整诊断与修复动作）"
    )
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cmd = self.fix_command();
        let tail = |f: &mut fmt::Formatter<'_>| tail(f, &cmd);
        match self {
            Self::UsernameInvalid { raw, reason } => {
                write!(f, "用户名不合法：{reason}。你填的是「{raw}」")?;
                tail(f)
            }
            Self::UsernameTaken { username_norm } => {
                write!(
                    f,
                    "用户名「{username_norm}」已被占用（忽略大小写后相同也算占用）"
                )?;
                tail(f)
            }
            Self::DisplayNameInvalid { raw, reason } => {
                write!(f, "显示名不合法：{reason}。你填的是「{raw}」")?;
                tail(f)
            }
            Self::PasswordTooShort { len, min } => {
                write!(f, "密码太短：现在 {len} 个字符，至少要 {min} 个")?;
                tail(f)
            }
            Self::PasswordTooLong { len, max } => {
                write!(
                    f,
                    "密码太长：现在 {len} 个字符，最多 {max} 个（上限用于给哈希成本封顶）"
                )?;
                tail(f)
            }
            Self::PasswordEqualsUsername => {
                write!(f, "密码不能和用户名一样")?;
                tail(f)
            }

            Self::CredentialsRejected => {
                write!(f, "用户名或密码不正确")?;
                tail(f)
            }
            Self::AccountDisabled { username_norm } => {
                write!(
                    f,
                    "账号「{username_norm}」已被禁用，无法登录。请让管理员在用户管理里启用它"
                )?;
                tail(f)
            }
            Self::AccountLocked {
                username_norm,
                until_ms,
            } => {
                write!(
                    f,
                    "账号「{username_norm}」因连续登录失败被临时锁定，解锁时刻（Unix 毫秒）={until_ms}。"
                )?;
                tail(f)
            }
            Self::NotAnOwner { operation } => {
                write!(f, "只有 owner（管理员）才能{operation}。当前账号是普通成员")?;
                tail(f)
            }
            Self::FirstOwnerExists => {
                write!(
                    f,
                    "已经存在 owner 账号，不能再创建第一个 owner。请改用邀请码或由 owner 建号"
                )?;
                tail(f)
            }
            Self::TokenMalformed => {
                write!(
                    f,
                    "会话令牌格式非法：应为 64 位小写十六进制（32 字节）。请重新登录获取新令牌"
                )?;
                tail(f)
            }
            Self::SessionUnknown => {
                write!(f, "会话不存在（可能已被登出或从未存在），请重新登录")?;
                tail(f)
            }
            Self::SessionExpired { expired_at_ms } => {
                write!(
                    f,
                    "会话已过期（过期时刻 Unix 毫秒={expired_at_ms}），请重新登录"
                )?;
                tail(f)
            }
            Self::SessionRevoked { reason } => {
                write!(f, "会话已被撤销（原因：{reason}），请重新登录")?;
                tail(f)
            }
            Self::SessionReuseDetected { family } => {
                write!(
                    f,
                    "检出令牌重放：已作废的令牌被再次使用，令牌家族 {family} 已整体撤销。"
                )?;
                tail(f)
            }
            Self::InviteMalformed => {
                write!(
                    f,
                    "邀请码格式非法：应为 32 位小写十六进制。请向管理员重新索取邀请码"
                )?;
                tail(f)
            }
            Self::InviteUnknown => {
                write!(f, "邀请码不存在或已被撤销，请向管理员重新索取")?;
                tail(f)
            }
            Self::InviteExpired { expired_at_ms } => {
                write!(
                    f,
                    "邀请码已过期（过期时刻 Unix 毫秒={expired_at_ms}），请让管理员重新生成"
                )?;
                tail(f)
            }
            Self::InviteExhausted { max_uses } => {
                write!(
                    f,
                    "邀请码使用次数已用满（上限 {max_uses} 次），请让管理员重新生成"
                )?;
                tail(f)
            }
            Self::InviteRevoked => {
                write!(f, "邀请码已被撤销，请让管理员重新生成")?;
                tail(f)
            }
            Self::UserNotFound => {
                write!(f, "用户不存在（或已被删除）")?;
                tail(f)
            }
            Self::SelfDisableForbidden => {
                write!(
                    f,
                    "不能禁用自己 —— 那可能把最后一个管理员锁在门外。请换一个管理员账号来禁用它"
                )?;
                tail(f)
            }
            Self::InviteUsesOutOfRange { got, max } => {
                write!(
                    f,
                    "邀请码使用次数 {got} 不合法：必须在 1..={max} 之间（0 次的邀请码永远无法被使用）"
                )?;
                tail(f)
            }
            Self::InvariantBroken { detail } => {
                write!(
                    f,
                    "内部不变量被破坏：{detail}。这不是你能靠重试解决的，请把本行连同 syslog 一起反馈"
                )?;
                tail(f)
            }
            Self::Storage { detail } => {
                write!(f, "数据库操作失败：{detail}")?;
                tail(f)
            }
        }
    }
}

impl std::error::Error for ControlError {}

impl From<ControlError> for AdapterError {
    fn from(e: ControlError) -> Self {
        let code = e.code();
        match e {
            ControlError::CredentialsRejected
            | ControlError::TokenMalformed
            | ControlError::SessionUnknown
            | ControlError::SessionExpired { .. }
            | ControlError::SessionRevoked { .. } => Self::Unauthorized(code.to_string()),
            ControlError::NotAnOwner { .. } | ControlError::AccountDisabled { .. } => {
                Self::Forbidden(code.to_string())
            }
            ControlError::UserNotFound
            | ControlError::InviteUnknown
            | ControlError::InviteExpired { .. }
            | ControlError::InviteExhausted { .. }
            | ControlError::InviteRevoked
            | ControlError::InviteMalformed => Self::NotFound(code.to_string()),
            ControlError::UsernameTaken { .. }
            | ControlError::FirstOwnerExists
            | ControlError::AccountLocked { .. }
            | ControlError::SessionReuseDetected { .. }
            | ControlError::UsernameInvalid { .. }
            | ControlError::DisplayNameInvalid { .. }
            | ControlError::PasswordTooShort { .. }
            | ControlError::PasswordTooLong { .. }
            | ControlError::PasswordEqualsUsername
            | ControlError::SelfDisableForbidden
            | ControlError::InviteUsesOutOfRange { .. } => Self::Conflict(code.to_string()),
            ControlError::Storage { .. } => Self::Storage(code.to_string()),
            ControlError::InvariantBroken { .. } => Self::Internal(code.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_of_each() -> Vec<ControlError> {
        vec![
            ControlError::UsernameInvalid {
                raw: "张 伟".into(),
                reason: "含空格",
            },
            ControlError::UsernameTaken {
                username_norm: "zhang_wei".into(),
            },
            ControlError::DisplayNameInvalid {
                raw: String::new(),
                reason: "不能为空",
            },
            ControlError::PasswordTooShort { len: 5, min: 12 },
            ControlError::PasswordTooLong { len: 500, max: 256 },
            ControlError::PasswordEqualsUsername,
            ControlError::CredentialsRejected,
            ControlError::AccountDisabled {
                username_norm: "zhang_wei".into(),
            },
            ControlError::AccountLocked {
                username_norm: "zhang_wei".into(),
                until_ms: 1_700_000_000_000,
            },
            ControlError::NotAnOwner {
                operation: "创建用户",
            },
            ControlError::FirstOwnerExists,
            ControlError::TokenMalformed,
            ControlError::SessionUnknown,
            ControlError::SessionExpired {
                expired_at_ms: 1_700_000_000_000,
            },
            ControlError::SessionRevoked {
                reason: "已登出".to_string(),
            },
            ControlError::SessionReuseDetected {
                family: "s:0123456789abcdef…".into(),
            },
            ControlError::InviteMalformed,
            ControlError::InviteUnknown,
            ControlError::InviteExpired {
                expired_at_ms: 1_700_000_000_000,
            },
            ControlError::InviteExhausted { max_uses: 1 },
            ControlError::InviteRevoked,
            ControlError::UserNotFound,
            ControlError::SelfDisableForbidden,
            ControlError::InviteUsesOutOfRange { got: 0, max: 1_000 },
            ControlError::InvariantBroken {
                detail: "会话行缺 user_id".into(),
            },
            ControlError::Storage {
                detail: "唯一约束冲突 ux_users_username_norm".into(),
            },
        ]
    }

    #[test]
    fn every_error_carries_a_copyable_command() {
        let all = one_of_each();
        assert_eq!(all.len(), 26, "变体数变了，请同步本测试的样本清单");
        for e in &all {
            let cmd = e.fix_command();
            assert!(!cmd.contains('\n'), "[{}] 命令不是单行：{cmd}", e.code());
            assert!(
                cmd.starts_with("quill "),
                "[{}] 命令不以 quill 开头：{cmd}",
                e.code()
            );
            assert!(
                cmd.contains("doctor"),
                "[{}] 命令未指向 doctor：{cmd}",
                e.code()
            );
            assert!(
                !cmd.ends_with(' '),
                "[{}] 命令尾部有多余空格：{cmd}",
                e.code()
            );
        }
    }

    #[test]
    fn every_error_display_points_at_the_same_command() {
        for e in one_of_each() {
            let msg = e.to_string();
            assert!(
                msg.contains(&e.fix_command()),
                "[{}] 文案没给出 fix_command() 的那条命令：\n{msg}",
                e.code()
            );

            assert!(
                !msg.contains("SQLITE_"),
                "[{}] 文案漏出数据库错误码：{msg}",
                e.code()
            );
        }
    }

    #[test]
    fn errors_never_echo_secrets() {
        let secretish = "hunter2-correct-horse";
        let tokenish = "a".repeat(64);
        let cases = [
            ControlError::UsernameInvalid {
                raw: "bad name".into(),
                reason: "含空格",
            },
            ControlError::InviteUnknown,
            ControlError::SessionUnknown,
            ControlError::TokenMalformed,
        ];
        for e in &cases {
            let msg = e.to_string();
            assert!(
                !msg.contains(secretish),
                "[{}] 文案里出现了密码：{msg}",
                e.code()
            );
            assert!(
                !msg.contains(&tokenish),
                "[{}] 文案里出现了令牌：{msg}",
                e.code()
            );
        }

        assert!(cases[0].to_string().contains("bad name"));
    }

    #[test]
    fn codes_are_unique_and_snake_case() {
        let all = one_of_each();
        let mut seen: Vec<&str> = all.iter().map(|e| e.code()).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "错误码重复：机器侧无法区分");
        for c in seen {
            assert!(
                c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
                "错误码不是小写 snake_case：{c}"
            );
        }
    }

    #[test]
    fn adapter_error_carries_code_not_prose() {
        let a: AdapterError = ControlError::CredentialsRejected.into();
        match a {
            AdapterError::Unauthorized(payload) => {
                assert_eq!(payload, "credentials_rejected");

                assert!(
                    payload.is_ascii(),
                    "AdapterError 载荷混进了非 ASCII（疑似中文文案）：{payload}"
                );
            }
            other => panic!("凭据错误应映射为 Unauthorized，实际 {other:?}"),
        }
        let b: AdapterError = ControlError::NotAnOwner {
            operation: "创建用户",
        }
        .into();
        assert!(matches!(b, AdapterError::Forbidden(_)));
        let c: AdapterError = ControlError::Storage { detail: "x".into() }.into();
        assert!(matches!(c, AdapterError::Storage(_)));
    }
}
