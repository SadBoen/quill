//! `quill-control` —— 多用户控制面（账号 / 鉴权 / 会话管理）。
//!
//! 依据 `docs/PHASE2_CONTRACT.md` §一：依赖方向为
//! `quill-adapters` / `quill-domain` / `quill-store`，
//! **禁止**依赖 `vendor/goose`、`quill-agent`、`quill-wiki`、`quill-server`。
//! 该约束由 `scripts/check-crate-deps.sh`（G40）强制。
//!
//! # 实现了什么
//!
//! | 能力 | 入口 |
//! |---|---|
//! | 首个 owner 引导 | [`ControlPlane::create_first_owner`] |
//! | owner 建号 | [`ControlPlane::create_user`] |
//! | 邀请码兑换建号 | [`ControlPlane::redeem_invite`] |
//! | 登录签发会话 | [`ControlPlane::login`] |
//! | 令牌轮换 + 重放检出 | [`ControlPlane::refresh`] |
//! | 登出 | [`ControlPlane::logout`] |
//! | 会话校验（过期 / 撤销 / 改密失效） | [`ControlPlane::authenticate`] |
//! | 改密并撤销全部会话 | [`ControlPlane::change_password`] |
//! | 用户隔离（自己 or owner） | [`ControlPlane::get_user`] / [`list_users`] |
//! | 邀请码签发 / 撤销 / 列出 | [`create_invite`](ControlPlane::create_invite) 等 |
//!
//! # 三个必须先读的限制（都写在了对应模块顶部）
//!
//! 1. **熵源是真 CSPRNG**：[`OsEntropySource`] 直接读操作系统内核熵池
//!    （Linux 上即 `getrandom(2)`），会话令牌 / 邀请码 / 密码盐都出自它。
//!    熵源失败时**进程终止**，且**绝不**回退到弱随机 —— 详见 [`secret`] 模块文档
//!    「熵源失败的处理」一节（并注意它与铁律七「配置错误不得 panic」的边界）。
//!    旧的伪源 `NonCsprngEntropySource` 已降级为 `#[cfg(test)]`，生产路径编译不到它。
//! 2. **密码哈希是 PBKDF2-HMAC-SHA256，而 schema 默认是 `argon2id`**：
//!    没有 `argon2` crate 且不许新增依赖，所以用标准 PBKDF2 顶上
//!    （`argon2` 缺席，自创方案更不可接受）。参数编进 `password_algo` 字符串，
//!    将来加 argon2 分支是**加分支**而不是重写。见 [`password`] 模块文档。
//! 3. **禁用 / 锁定会泄露「该用户名存在」**：明知有代价仍保留可区分报错，
//!    因为对「输对密码却被告知密码错」的用户来说，那违反铁律七。
//!    见 [`service`] 模块文档的「已知局限」。
//!
//! # 时间与随机源都是注入的
//!
//! [`Clock`] 与 [`SecretSource`] 是 trait，测试注入 [`clock::ManualClock`]
//! 与确定性熵源，于是「过期」「锁定」「重放」这些时间相关逻辑
//! 全部可以**不 sleep 精确断言**（不会 flaky）。
//! 本 crate **没有** `static` 单例（边界规则 7 面 B）。
//!
//! # 隔离性
//!
//! 本 crate 内的隔离入口是 [`ControlPlane::get_user`] /
//! [`ControlPlane::list_users`] / [`ControlPlane::revoke_all_sessions`]：
//! 非 owner 只能对自己操作。集成测试 `tests/control_plane.rs`
//! 里有「A 用户读不到 B 用户」的断言（契约 §八 的验收项）。

pub mod clock;
pub mod error;
pub mod password;
pub mod repo;
pub mod secret;
pub mod service;
pub mod user;

pub use clock::{Clock, ManualClock, SystemClock};
pub use error::ControlError;
pub use password::{
    constant_time_eq, hmac_sha256, pbkdf2_sha256, validate_password, PasswordDigest,
    PasswordHasher, Pbkdf2Params, DK_BYTES, MAX_PASSWORD_LEN, MIN_PASSWORD_LEN, PBKDF2_ALGO_PREFIX,
    SALT_BYTES,
};
pub use secret::{
    entropy_unavailable_message, invite_code_digest, new_invite_code, new_session_token,
    session_token_digest, OsEntropySource, SecretSource, INVITE_CODE_BYTES, INVITE_CODE_HEX_LEN,
    TOKEN_BYTES, TOKEN_HEX_LEN,
};
// 🔴 `NonCsprngEntropySource` **刻意不在这里 re-export**：它是 `#[cfg(test)]`，
// 生产构建里根本不存在（误用即编译错误）。见 secret 模块文档与 D-2026-10-05-08。
pub use service::{
    AuthPolicy, AuthSession, Authenticated, ControlPlane, InviteSummary, IssuedInvite,
    LogoutOutcome, RegistrationRequest, DEFAULT_LOCALE, MAX_INVITE_USES,
};
pub use user::{
    normalize_username, validate_display_name, validate_username, UserProfile, UserRole,
    UserStatus, MAX_DISPLAY_NAME_LEN, MAX_USERNAME_LEN, MIN_USERNAME_LEN,
};

// 身份类型从契约层 re-export：调用方不必再引 `quill-adapters`，
// 但**类型仍是同一个**（`quill_domain::UserId` 与 `quill_adapters::UserId` 互通）。
pub use quill_domain::{SessionId, UserId, UuidBytes};
