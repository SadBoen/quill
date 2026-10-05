//! 用户领域类型与用户名规范化。
//!
//! # 用户名的两段式设计（原始值 + 规范化值）
//!
//! `users` 表有两列：`username`（原样）与 `username_norm`（规范化），
//! 唯一索引建在 `username_norm` 上。这不是冗余：
//!
//! - `username` 保留用户敲的原始写法 → 界面上显示得让人认得出是自己；
//! - `username_norm` 用于查重与登录查找 → 同一账号不能因为大小写不同而存在两份。
//!
//! # 规范化规则为什么这么定
//!
//! | 规则 | 理由 |
//! |---|---|
//! | 去首尾空白 | `" zhang "` 与 `"zhang"` 是同一个人 |
//! | Unicode 小写 | 避免 `Zhang` / `zhang` / `ZHANG` 三份账号 |
//! | **拒绝内部空白** | `"zhang wei"` 不能登录（界面提示是用户名不是全名） |
//! | 只允许 `[a-z0-9._-]` | 用户名会进目录名（`quill_store::user_dir_name`）与日志；路径分隔符、空格、控制字符全部排除 |
//! | 3~32 字符 | 短了易撞名，长了不适合口述与手输 |
//! | 必须字母或数字开头 | 避免 `-abc` / `.abc` 这类在命令行与文件名里需要额外转义的形态 |
//!
//! # 显示名为什么单独校验
//!
//! `users` 表有 `CHECK(length(display_name) BETWEEN 1 AND 64)`，
//! 而**空串会通过长度检查吗？不会** —— 长度 0 落在 1..64 之外，被 CHECK 拒。
//! 但我们不靠数据库报错来告诉用户「显示名不能为空」：那会变成一条 sqlx 错误码，
//! 违反铁律七。校验放在应用层，错误信息是人话。

use quill_domain::UserId;

use crate::error::ControlError;

/// 用户名允许的最大长度（与 `CHECK(length(username_norm) BETWEEN 1 AND 64)` 不冲突）。
pub const MAX_USERNAME_LEN: usize = 32;

/// 用户名要求的最小长度。
pub const MIN_USERNAME_LEN: usize = 3;

/// 显示名长度上限（对齐 schema 的 CHECK）。
pub const MAX_DISPLAY_NAME_LEN: usize = 64;

/// 角色（v1 只两级：`V1_SCOPE_CONSTRAINTS.md` 第三节「不做 RBAC 细粒度权限」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserRole {
    /// 管理员：可建号、可发邀请码、可禁用他人。
    Owner,
    /// 普通成员：只能管自己。
    Member,
}

impl UserRole {
    /// 数据库取值（`users.role` 的 CHECK 只允许这两个）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
        }
    }

    /// 从数据库取值解析。
    pub fn parse(s: &str) -> Result<Self, ControlError> {
        match s {
            "owner" => Ok(Self::Owner),
            "member" => Ok(Self::Member),
            other => Err(ControlError::InvariantBroken {
                detail: format!("users.role={other:?} 不在 schema CHECK ('owner','member') 之内"),
            }),
        }
    }

    /// 是否是 owner。
    pub fn is_owner(self) -> bool {
        matches!(self, Self::Owner)
    }
}

/// 账号状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UserStatus {
    /// 正常，可登录。
    Active,
    /// 被管理员禁用，不可登录（**已签发的会话立即失效**）。
    Disabled,
}

impl UserStatus {
    /// 数据库取值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    /// 从数据库取值解析。
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

/// 用户公开档案。
///
/// ⚠️ **这个类型里没有任何密码字段，这是刻意的。**
///
/// 理由：它是「返回给 HTTP 层 / 前端」的类型。一旦它带上
/// `password_hash` / `password_salt`，那么任何一个 `..user` 的展开
/// 或一次 `Debug` 打印就会把离线爆破材料送到日志与浏览器。
/// 需要凭据的行由 crate 内部的 `UserCredentials` 承载，不导出。
///
/// `..user` 展开需要 `Debug`，所以本类型**必须**手写 `Debug`：
/// 它是控制面唯一会出现在 `quill-server` 日志里的用户结构。
#[derive(Clone, PartialEq, Eq)]
pub struct UserProfile {
    /// 用户标识。
    pub id: UserId,
    /// 原始用户名（用户敲的样子）。
    pub username: String,
    /// 规范化用户名（唯一）。
    pub username_norm: String,
    /// 显示名。
    pub display_name: String,
    /// 角色。
    pub role: UserRole,
    /// 状态。
    pub status: UserStatus,
    /// 凭据代次（每次改密 +1）。
    pub token_epoch: i64,
    /// 创建时刻（Unix 毫秒）。
    pub created_at_ms: i64,
    /// 最后登录时刻（Unix 毫秒）；从未登录为 `None`。
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

/// 规范化用户名：去首尾空白 + Unicode 小写。
///
/// ⚠️ **这一步不做字符集校验** —— 校验在 [`validate_username`]。
/// 分两步的理由：登录查找需要一个「即使非法也能拿去查」的名字，
/// 否则「用户名非法」会成为登录接口的用户枚举器。
pub fn normalize_username(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// 校验规范化后的用户名。
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
            // 空白单独说：这是最常见的错（打成了全名），值得更具体的提示
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

/// 校验显示名。
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
        // Unicode 小写也要生效：否则 "Ä" 与 "ä" 会变成两个不同账号
        assert_eq!(normalize_username("Ä"), normalize_username("ä"));
        assert_eq!(normalize_username("ÄÖÜ"), "äöü");
        // 只去首尾空白：内部空白保留，好让 validate 给出「含空格」的具体提示
        assert_eq!(normalize_username(" zhang wei "), "zhang wei");
    }

    #[test]
    fn valid_usernames_pass() {
        // 装置可信性：合法输入先能过，否则下面的拒绝可能在测空气
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
            // ⚠️ 必须是「ASCII 开头 + 含汉字」：若首字符就是汉字，
            //    会先被「必须以字母或数字开头」拦下，那条断言就只是在测首字符规则。
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
        // 超过 32 字符
        assert!(matches!(
            validate_username(&"a".repeat(MAX_USERNAME_LEN + 1)),
            Err(ControlError::UsernameInvalid { .. })
        ));
    }

    #[test]
    fn uppercase_is_rejected_after_normalization_but_normalization_removes_it() {
        // 这是刻意的分工：normalize 会把大写转小写，所以走到 validate 的值里
        // **不可能**含大写。保留这条断言是为了固化「大写不是错误」这个产品决定，
        // 免得后人把 normalize 改成「只 trim」然后这里静默变红。
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
        // 这些值过不了 schema 的 CHECK，所以行根本进不去；
        // 若解析层也拒绝，则「读到脏数据」与「schema 被改」两种故障可区分。
        for bad in ["admin", "OWNER", "", "owner "].iter() {
            assert!(UserRole::parse(bad).is_err(), "{bad:?} 必须被拒");
        }
        for bad in ["locked", "ACTIVE", ""].iter() {
            assert!(UserStatus::parse(bad).is_err(), "{bad:?} 必须被拒");
        }
    }
}
