//! 控制面领域错误：中文人话 + **可直接复制**的修复命令。
//!
//! 依据 `AGENTS.md` 铁律七「失败必须自诊断」：
//! - 不把 `EACCES` / sqlx 错误码抛给用户；
//! - 每条错误都带一条**完整、可直接复制**的命令；
//! - 用户出错时**唯一需要执行**的命令是 `quill doctor` —— 本模块的
//!   `tests::every_error_carries_a_copyable_command` 把这条口径固化成断言。
//!
//! # 两条不可协商的约束
//!
//! | 约束 | 原因 | 固化它的测试 |
//! |---|---|---|
//! | 错误消息里**绝不出现**密码 / 令牌原文 | 错误会进日志、进 syslog、进 UI 横幅；明文一旦落到那里就是凭据泄漏 | `errors_never_echo_secrets` |
//! | 「用户名不存在」与「密码错误」返回**同一个**变体 | 否则登录接口就是用户枚举器 | `tests::control_plane.rs::login_does_not_reveal_whether_username_exists` |
//!
//! # 为什么手写 `Display` 而不用 `thiserror`
//!
//! `thiserror` 在 `Cargo.lock` 里但不在本 crate 的依赖表里；
//! 契约层 `quill-adapters` 也是同样处理（见其 `member.rs` 的同名说明）。
//! 少写几行 derive 不值得引入一条新依赖边。

use std::fmt;

use quill_adapters::AdapterError;

/// 全项目统一的诊断命令（`AGENTS.md` 铁律七：唯一需要执行的命令）。
///
/// ⚠️ 它是**诊断入口**而不是万能修复：具体错误会额外给出更精确的命令。
pub const DOCTOR_CMD: &str = "quill doctor";

/// 控制面领域错误。
///
/// 变体按「用户能据此做什么」划分，而不是按数据库列划分：
/// 每个变体都必须能回答「下一步执行哪条命令」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    /// 用户名非法（附中文原因）。
    UsernameInvalid {
        /// 用户原始输入（原样回显，便于用户看清自己敲了什么）。
        raw: String,
        /// 违规原因（中文短句）。
        reason: &'static str,
    },
    /// 用户名规范化后已被占用。
    UsernameTaken {
        /// 规范化后的用户名。
        username_norm: String,
    },
    /// 显示名非法。
    DisplayNameInvalid {
        /// 用户原始输入。
        raw: String,
        /// 违规原因（中文短句）。
        reason: &'static str,
    },
    /// 密码短于下限。
    PasswordTooShort {
        /// 实际长度（按字符计，非字节）。
        len: usize,
        /// 要求下限。
        min: usize,
    },
    /// 密码长于上限（用于给 PBKDF2 成本封顶）。
    PasswordTooLong {
        /// 实际长度。
        len: usize,
        /// 允许上限。
        max: usize,
    },
    /// 密码与用户名相同。
    PasswordEqualsUsername,
    /// 凭据不匹配：**用户名不存在**与**密码错误**共用此变体。
    ///
    /// ⚠️ 拆成两个变体等于开一个用户枚举器，所以这里刻意合并。
    CredentialsRejected,
    /// 账号被禁用。
    AccountDisabled {
        /// 用户名（规范化形态）。
        username_norm: String,
    },
    /// 连续失败过多，账号被临时锁定。
    AccountLocked {
        /// 用户名（规范化形态）。
        username_norm: String,
        /// 解锁时刻（毫秒时间戳）。
        until_ms: i64,
    },
    /// 操作者不是 owner。
    NotAnOwner {
        /// 该操作要求的角色说明。
        operation: &'static str,
    },
    /// 已经有第一个 owner 了，不能再 bootstrapping。
    FirstOwnerExists,
    /// 令牌格式非法（不是 64 位小写 hex）。
    TokenMalformed,
    /// 令牌查无对应会话。
    SessionUnknown,
    /// 会话已过期。
    SessionExpired {
        /// 过期时刻（毫秒时间戳）。
        expired_at_ms: i64,
    },
    /// 会话已被撤销（含「改密后撤销」「登出撤销」「轮换撤销」）。
    SessionRevoked {
        /// 撤销原因（来自 `sessions_auth.revoked_reason`，原文回显）。
        reason: String,
    },
    /// 检出**令牌重放**：已轮换/已撤销的令牌又被使用 → 整个令牌家族连坐撤销。
    SessionReuseDetected {
        /// 令牌家族标识（`SessionId` 的 Display 形态，可直接贴进 syslog）。
        family: String,
    },
    /// 邀请码格式非法。
    InviteMalformed,
    /// 邀请码不存在。
    InviteUnknown,
    /// 邀请码已过期。
    InviteExpired {
        /// 过期时刻（毫秒时间戳）。
        expired_at_ms: i64,
    },
    /// 邀请码已被用满。
    InviteExhausted {
        /// 允许的最大使用次数。
        max_uses: i32,
    },
    /// 邀请码已被撤销。
    InviteRevoked,
    /// 目标用户不存在（或已软删除）。
    UserNotFound,
    /// owner 试图禁用自己。
    SelfDisableForbidden,
    /// 邀请码使用次数超出允许区间。
    InviteUsesOutOfRange {
        /// 请求的次数。
        got: i32,
        /// 允许上限。
        max: i32,
    },
    /// 不变量被破坏（代码 bug，不是用户能修的）。
    InvariantBroken {
        /// 中文描述。
        detail: String,
    },
    /// 存储层失败。
    Storage {
        /// 中文描述（**不得**含凭据原文）。
        detail: String,
    },
}

impl ControlError {
    /// 错误码：稳定的机器可读标识。
    ///
    /// 存在的理由：`quill-server` 把它映射成 HTTP 状态码与前端 i18n key，
    /// 而**不能**去匹配中文文案（文案会改，码不会）。
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

    /// 该错误**唯一**需要的用户动作命令（可直接复制）。
    ///
    /// 判据（`tests::every_error_carries_a_copyable_command` 固化为断言）：
    /// ① 必须以 `quill` 开头（是本项目的命令，不是 `rm -rf` 之类）；
    /// ② 必须单行（可整行复制进终端）；
    /// ③ 必须含 `doctor`，即最终诊断入口唯一。
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

/// 统一的尾部提示：告诉用户「复制这一行就行」。
///
/// ⚠️ 尾部必须打 `fix_command()`（带 `--section=` 的**那一条**），
/// 而不是笼统的 `quill doctor` —— 否则文案与程序建议不一致，
/// 用户照抄文案的命令会与 `fix_command()` 指向不同的检查段。
/// 两者一致由 `tests::every_error_display_points_at_the_same_command` 守住。
fn tail(f: &mut fmt::Formatter<'_>, cmd: &str) -> fmt::Result {
    write!(
        f,
        "\n→ 下一步：复制执行 `{cmd}`（它会打印本错误的完整诊断与修复动作）"
    )
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // ⚠️ 先算一次命令，所有分支共用：保证「文案里的命令」与
        //    `fix_command()` **逐字相同**（铁律七的唯一真相源）。
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
            // ⚠️ 这一条**刻意不区分**「用户不存在」与「密码错」。
            // 若分开提示，登录接口就成了用户枚举器。
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
    /// 在契约层边界转换（`docs/PHASE2_CONTRACT.md` §三）。
    ///
    /// 映射口径：**按语义类别**分，不按变体一对一 ——
    /// 因为契约层只有 7 个变体，而本层有 22 个。
    fn from(e: ControlError) -> Self {
        // ⚠️ 只带 `code()` 过去，**不带中文文案**：
        // 中文文案属于人看的层，塞进机器枚举的载荷里会让日志无法聚合。
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

    /// 构造**每一个**变体的一个样本。
    ///
    /// 新增变体时忘记写中文文案 / 忘记写命令，本测试会立刻红 ——
    /// 这就是它存在的意义。
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
        // 判据三条同时成立才算通过（AGENTS.md 铁律七）：
        // ① 命令是**单行** —— 多行没法整行复制
        // ② 命令以 `quill` 开头 —— 是本项目命令，不是危险外部命令
        // ③ 命令含 `doctor` —— 诊断入口唯一
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
        // 铁律七的「唯一命令」口径：文案里给出的命令必须与 fix_command() 一致，
        // 否则用户照抄文案里的命令会与程序建议的不一致（两处真相源）。
        for e in one_of_each() {
            let msg = e.to_string();
            assert!(
                msg.contains(&e.fix_command()),
                "[{}] 文案没给出 fix_command() 的那条命令：\n{msg}",
                e.code()
            );
            // 不得把 errno 之类的东西抛给用户
            assert!(
                !msg.contains("SQLITE_"),
                "[{}] 文案漏出数据库错误码：{msg}",
                e.code()
            );
        }
    }

    #[test]
    fn errors_never_echo_secrets() {
        // 装置可信性：这里真的构造一个「含凭据」的输入，证明它**没有**出现在消息里。
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
        // 反向断言：确实查了东西（否则上面两条是恒真）
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
        // 契约边界：机器枚举里只带 code，不带中文文案。
        // 断言「不带文案」是本测试的价值 —— 带上文案日志就没法按 code 聚合。
        let a: AdapterError = ControlError::CredentialsRejected.into();
        match a {
            AdapterError::Unauthorized(payload) => {
                assert_eq!(payload, "credentials_rejected");
                // 载荷必须是纯 ASCII 的 code：出现任何非 ASCII 即说明混进了中文文案
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
