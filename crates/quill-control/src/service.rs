//! 控制面服务：注册 / 登录 / 登出 / 会话校验 / 邀请码。
//!
//! # 分层
//!
//! ```text
//! ControlPlane（本模块：业务规则 + 事务边界）
//!     ↓ 只调函数，不自己发 SQL
//! repo（src/repo.rs：数据访问 + 行装配）
//!     ↓
//! quill-store（连接层 + schema 约束）
//! ```
//!
//! # 四个必须显式说明的安全取舍
//!
//! | 取舍 | 选择 | 代价（诚实记录） |
//! |---|---|---|
//! | 用户名不存在 vs 密码错 | 返回**同一个** [`ControlError::CredentialsRejected`]，并消耗等价 CPU 时间 | 已用 `burn_equivalent_time` 补平计时差 |
//! | 账号被锁定 / 被禁用 | 返回**可区分**的 [`ControlError::AccountLocked`] / [`ControlError::AccountDisabled`] | **残留用户枚举面**（见下） |
//! | `refresh` 遇到已撤销令牌 | 判为**令牌重放**并连坐撤销整个家族 | 极端时序下会误伤同族，代价是用户重新登录一次 |
//! | `authenticate` 遇到已撤销令牌 | 只报 [`ControlError::SessionRevoked`]，**不**连坐 | 漏掉一种重放信号；理由是登出后的在途请求不该把整族连坐掉 |
//!
//! ## 已知局限：账号状态带来的用户枚举
//!
//! 上表第 2 行是**明知有代价仍这么选**的。改成「也返回 CredentialsRejected」的话，
//! 一个输入了**正确**密码的用户会被告知「密码错误」——
//! 那直接违反铁律七（失败必须自诊断：用户在出错时唯一需要执行的是 `quill doctor`）。
//! 因此保留可区分报错，把局限记录在此与交付报告里，由主理人裁决是否接受。
//!
//! # 并发边界（必须知道）
//!
//! - SQLite 单写者（`quill-store` 建池 `max_connections=1`），
//!   所以「校验邀请码 → 建用户 → 记账」能放进一个事务而不丢原子性。
//! - 但这**不等于**多进程安全，详见 [`ControlPlane::create_first_owner`]。

use std::sync::Arc;

use sqlx::SqlitePool;

use quill_domain::{SessionId, UserId, UuidBytes};

use crate::clock::Clock;
use crate::error::ControlError;
use crate::password::{validate_password, PasswordHasher, Pbkdf2Params};
use crate::repo;
use crate::secret::{
    invite_code_digest, new_invite_code, new_session_token, session_token_digest, OsEntropySource,
    SecretSource,
};
use crate::user::{
    normalize_username, validate_display_name, validate_username, UserProfile, UserRole, UserStatus,
};

/// 默认语言标记（`users.locale` 的 schema 默认值；显式写出以免依赖隐式默认）。
pub const DEFAULT_LOCALE: &str = "zh-CN";

/// 邀请码允许的最大使用次数。
pub const MAX_INVITE_USES: i32 = 1_000;

/// 鉴权策略参数：失败锁定、会话时长、邀请码时长。
///
/// 抽成结构体而不是散落的 `const`：这些值会按部署场景不同
/// （家用 NAS 与公司内网对会话时长的期望完全不同），散落的 `const` 无法覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthPolicy {
    /// 连续失败多少次后锁定。
    pub max_failures: i32,
    /// 锁定时长（毫秒）。
    pub lock_millis: i64,
    /// 登录后签发的会话有效期（毫秒）。
    pub session_ttl_millis: i64,
    /// 轮换后新会话的有效期（毫秒）。
    pub refresh_ttl_millis: i64,
    /// 邀请码默认有效期（毫秒）。
    pub invite_ttl_millis: i64,
}

impl AuthPolicy {
    /// 生产默认值。
    pub const fn production() -> Self {
        Self {
            max_failures: 5,
            // 15 分钟：够挡住在线爆破，又不至于让手抖连错 5 次就锁一整晚
            lock_millis: 15 * 60 * 1000,
            // 7 天：家用场景的浏览器不会天天重新登录
            session_ttl_millis: 7 * 24 * 60 * 60 * 1000,
            // 轮换时给 1 天，覆盖「忘记点了重新登录」
            refresh_ttl_millis: 24 * 60 * 60 * 1000,
            invite_ttl_millis: 7 * 24 * 60 * 60 * 1000,
        }
    }

    /// 测试用：把时间窗压到秒级，让「过期 / 解锁」能被单测精确断言。
    pub const fn for_tests() -> Self {
        Self {
            max_failures: 3,
            lock_millis: 60_000,
            session_ttl_millis: 3_600_000,
            refresh_ttl_millis: 600_000,
            invite_ttl_millis: 120_000,
        }
    }
}

/// 建号请求。
///
/// ⚠️ 手写 `Debug` 且**屏蔽密码**：`quill-server` 一定会打请求日志（排障需要），
/// 而派生 `Debug` 会把明文密码抄一份进日志。
#[derive(Clone, PartialEq, Eq)]
pub struct RegistrationRequest {
    /// 用户名（原始写法；规范化后的形态入库）。
    pub username: String,
    /// 显示名。
    pub display_name: String,
    /// 明文密码（仅在调用瞬间存在，**永不**入库明文、**永不**进日志）。
    pub password: String,
    /// 目标角色。**仅** [`ControlPlane::create_user`] 使用；
    /// [`ControlPlane::redeem_invite`] 的角色由邀请码决定，本字段被忽略。
    pub role: UserRole,
}

impl std::fmt::Debug for RegistrationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistrationRequest")
            .field("username", &self.username)
            .field("display_name", &self.display_name)
            .field("password", &"<已隐藏>")
            .field("role", &self.role)
            .finish()
    }
}

impl RegistrationRequest {
    /// 便捷构造（角色默认 `member`）。
    pub fn new(
        username: impl Into<String>,
        display_name: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            display_name: display_name.into(),
            password: password.into(),
            role: UserRole::Member,
        }
    }

    /// 指定角色。
    pub fn with_role(mut self, role: UserRole) -> Self {
        self.role = role;
        self
    }
}

/// 一次成功登录（或轮换）的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSession {
    /// 会话标识。
    pub session_id: SessionId,
    /// 用户标识。
    pub user_id: UserId,
    /// 用户名（规范化）。
    pub username_norm: String,
    /// 角色。
    pub role: UserRole,
    /// **明文令牌**：只在签发那一刻有值，库里只有它的 SHA-256 摘要，取不回来。
    pub token: String,
    /// 签发时刻。
    pub issued_at_ms: i64,
    /// 过期时刻。
    pub expires_at_ms: i64,
}

/// 令牌校验通过后的调用者身份。
///
/// 用途：`quill-server` 的鉴权中间件拿它构造 `UserContext`
/// （契约 §2.4：身份 = 请求入口构造的上下文，**不是**全局单例）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authenticated {
    /// 会话标识。
    pub session_id: SessionId,
    /// 用户标识。
    pub user_id: UserId,
    /// 用户档案。
    pub profile: UserProfile,
    /// 会话过期时刻。
    pub expires_at_ms: i64,
}

/// 新签发的邀请码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedInvite {
    /// 邀请码标识（用于之后 `revoke_invite`）。
    pub id: UuidBytes,
    /// **明文邀请码**：只在签发那一刻返回；库里只有摘要，**无法再取回**。
    pub code: String,
    /// 被邀请人将获得的角色。
    pub role: UserRole,
    /// 过期时刻。
    pub expires_at_ms: i64,
    /// 允许使用次数。
    pub max_uses: i32,
}

/// 邀请码概览（**不含**明文码 —— 取不回来）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteSummary {
    /// 邀请码标识。
    pub id: UuidBytes,
    /// 签发者（= 调用方本人；`list_invites` 已按此过滤）。
    pub created_by: UserId,
    /// 被邀请人将获得的角色。
    pub role: UserRole,
    /// 允许使用次数。
    pub max_uses: i32,
    /// 已使用次数。
    pub used_count: i32,
    /// 过期时刻。
    pub expires_at_ms: i64,
    /// 创建时刻。
    pub created_at_ms: i64,
    /// 是否已撤销。
    pub revoked: bool,
}

/// 登出结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogoutOutcome {
    /// 本次真正撤销的会话数。
    ///
    /// ⚠️ **刻意不是 `()`**：登出对「本来就没登录」的令牌幂等成功，
    /// 若返回 `()`，调用方无法区分「撤销了一个会话」与「什么都没发生」——
    /// 那就是静默通过（铁律十六精神：0 与未检查必须可区分）。
    pub revoked: u64,
}

/// 多用户控制面。
///
/// ⚠️ 本类型**不是**全局单例（边界规则 7 面 B）：它持有连接池、时钟与随机源，
/// 由 `quill-server` 启动时构造并注入 axum 状态。
pub struct ControlPlane {
    pool: SqlitePool,
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn SecretSource>,
    hasher: PasswordHasher,
    policy: AuthPolicy,
}

impl std::fmt::Debug for ControlPlane {
    /// ⚠️ 只打「形状」不打内容：`Debug` 可能被 `quill-server` 打进启动日志，
    /// 而熵源与连接池都不该出现在那里（前者是凭据来源，后者带数据库路径）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlPlane")
            .field("hasher", &self.hasher)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl ControlPlane {
    /// 用生产策略 + **系统熵源**构造。
    ///
    /// 这是给生产用的入口：熵源固定为 [`OsEntropySource`]（真 CSPRNG），
    /// 调用方**无法**在这里误传一个弱熵源。
    /// 需要注入确定性熵源时（测试）请用 [`ControlPlane::with_policy`]。
    pub fn new_with_os_entropy(
        pool: SqlitePool,
        clock: Arc<dyn Clock>,
        params: Pbkdf2Params,
    ) -> Self {
        Self::new(pool, clock, Arc::new(OsEntropySource), params)
    }

    /// 用生产策略构造。
    pub fn new(
        pool: SqlitePool,
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn SecretSource>,
        params: Pbkdf2Params,
    ) -> Self {
        Self::with_policy(pool, clock, entropy, params, AuthPolicy::production())
    }

    /// 指定策略构造。
    pub fn with_policy(
        pool: SqlitePool,
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn SecretSource>,
        params: Pbkdf2Params,
        policy: AuthPolicy,
    ) -> Self {
        Self {
            pool,
            clock,
            entropy,
            hasher: PasswordHasher::new(params),
            policy,
        }
    }

    /// 连接池（供 `quill-server` 组装其它仓储时共用同一个池）。
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// 当前策略。
    pub fn policy(&self) -> AuthPolicy {
        self.policy
    }

    // ═══════════════════ 建号 ═══════════════════

    /// 创建**第一个** owner（首次启动引导）。
    ///
    /// # 并发局限（必须知道）
    ///
    /// 「库必须为空」与「插入 owner」之间**没有**跨进程锁。
    /// 本项目是单进程（契约 C-1 裁决），所以在单进程内成立；
    /// 若将来真有两个进程同时首启，可能产生两个 owner ——
    /// 唯一索引挡不住它（`username_norm` 不同即可）。
    /// 真正的修法是加一张「初始化标记」单行表（expand-only 迁移），须主理人裁决。
    /// 在那之前，这段注释是该局限的**唯一**记录。
    pub async fn create_first_owner(
        &self,
        req: &RegistrationRequest,
    ) -> Result<UserProfile, ControlError> {
        let now = self.clock.now_millis();
        let (username, norm, display) = self.validate_registration(req)?;
        // ⚠️ 角色强制 owner：调用方传什么都不生效。
        //    否则「建第一个 owner」就等于「任何人能建管理员」。
        let digest = self.hasher.hash(self.entropy.as_ref(), &req.password);
        if repo::count_users(&self.pool).await? > 0 {
            return Err(ControlError::FirstOwnerExists);
        }
        let id = UserId::from_bytes(self.new_uuid_bytes());
        repo::insert_user(
            &self.pool,
            repo::NewUser {
                id: &id,
                username: &username,
                username_norm: &norm,
                display_name: &display,
                digest: &digest,
                role: UserRole::Owner,
                locale: DEFAULT_LOCALE,
                now_ms: now,
            },
        )
        .await?;
        self.profile_of(&id).await
    }

    /// owner 建号。
    ///
    /// 允许 owner 建 owner：单点故障恢复（owner 账号被盗或误删）需要这条路。
    /// 该决定的代价是「owner 凭据泄露 = 完全沦陷」，因此**必须**配合审计（v1.1）。
    pub async fn create_user(
        &self,
        actor: &UserId,
        req: &RegistrationRequest,
    ) -> Result<UserProfile, ControlError> {
        self.require_owner(actor, "创建用户").await?;
        let now = self.clock.now_millis();
        let (username, norm, display) = self.validate_registration(req)?;
        let digest = self.hasher.hash(self.entropy.as_ref(), &req.password);
        let id = UserId::from_bytes(self.new_uuid_bytes());
        repo::insert_user(
            &self.pool,
            repo::NewUser {
                id: &id,
                username: &username,
                username_norm: &norm,
                display_name: &display,
                digest: &digest,
                role: req.role,
                locale: DEFAULT_LOCALE,
                now_ms: now,
            },
        )
        .await?;
        self.profile_of(&id).await
    }

    /// 邀请码兑换建号（角色由邀请码决定，请求里的 `role` 被忽略）。
    ///
    /// 整个「查邀请码 → 判有效性 → 建用户 → 记账」在**一个事务**里。
    /// 拆成两条语句时，「检查 `used_count < max_uses`」与「`used_count+1`」
    /// 之间的窗口会被并发的第二个请求插队 → 超出 `max_uses` 发放。
    pub async fn redeem_invite(
        &self,
        code: &str,
        req: &RegistrationRequest,
    ) -> Result<UserProfile, ControlError> {
        let now = self.clock.now_millis();
        let (username, norm, display) = self.validate_registration(req)?;
        let digest = self.hasher.hash(self.entropy.as_ref(), &req.password);
        let code_digest = invite_code_digest(code)?;
        let id = UserId::from_bytes(self.new_uuid_bytes());

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| repo::map_db_error(e, "开启邀请码兑换事务"))?;

        let invite = repo::find_invite_tx(&mut tx, &code_digest)
            .await?
            .ok_or(ControlError::InviteUnknown)?;
        if invite.revoked_at_ms.is_some() {
            return Err(ControlError::InviteRevoked);
        }
        if invite.expires_at_ms <= now {
            return Err(ControlError::InviteExpired {
                expired_at_ms: invite.expires_at_ms,
            });
        }
        if invite.used_count >= invite.max_uses {
            return Err(ControlError::InviteExhausted {
                max_uses: invite.max_uses,
            });
        }

        repo::insert_user_tx(
            &mut tx,
            repo::NewUser {
                id: &id,
                username: &username,
                username_norm: &norm,
                display_name: &display,
                digest: &digest,
                role: invite.role,
                locale: DEFAULT_LOCALE,
                now_ms: now,
            },
        )
        .await?;
        repo::consume_invite_tx(&mut tx, &invite.id, &id, now).await?;
        tx.commit()
            .await
            .map_err(|e| repo::map_db_error(e, "提交邀请码兑换事务"))?;

        self.profile_of(&id).await
    }

    // ═══════════════════ 登录 / 登出 / 轮换 ═══════════════════

    /// 登录。
    ///
    /// 判定顺序是**刻意**的：
    /// 1. 规范化 + 查用户；
    /// 2. 用户不存在 → 消耗等价 CPU 时间后返回 `CredentialsRejected`；
    /// 3. **先验密码**，再判状态 —— 否则「禁用账号」能被一个错误密码探测出来，
    ///    而且禁用状态本该只对持有正确密码的人才有意义；
    /// 4. 密码正确后才判锁定 / 禁用 / 签发会话。
    pub async fn login(
        &self,
        username: &str,
        password: &str,
        user_agent: Option<&str>,
        peer_addr: Option<&str>,
    ) -> Result<AuthSession, ControlError> {
        let now = self.clock.now_millis();
        let norm = normalize_username(username);
        // ⚠️ 用户名非法也走「消耗时间 + 统一错误」：
        //    否则非法用户名比合法用户名明显更快，等于免费的用户名格式探测器。
        let creds = if norm.is_empty() || validate_username(&norm).is_err() {
            self.hasher.burn_equivalent_time();
            None
        } else {
            repo::find_credentials(&self.pool, &norm).await?
        };

        let Some(creds) = creds else {
            return Err(ControlError::CredentialsRejected);
        };

        if !self.hasher.verify_stored(&creds.digest, password) {
            let fail = creds.login_fail_count.saturating_add(1);
            let locked_until = if fail >= self.policy.max_failures {
                Some(now.saturating_add(self.policy.lock_millis))
            } else {
                creds.locked_until_ms
            };
            repo::record_login_failure(&self.pool, &creds.id, fail, locked_until, now).await?;
            // ⚠️ 这里**不**告诉攻击者「你已被锁定」：那会加速用户名枚举。
            //    锁定信息只在**密码正确**时给出（见下）。
            return Err(ControlError::CredentialsRejected);
        }

        // ── 密码正确，从这里开始才允许返回「与账号状态有关」的错误 ──
        if let Some(until) = creds.locked_until_ms {
            if until > now {
                return Err(ControlError::AccountLocked {
                    username_norm: norm,
                    until_ms: until,
                });
            }
        }
        if creds.status != UserStatus::Active {
            return Err(ControlError::AccountDisabled {
                username_norm: norm,
            });
        }

        // ⚠️ 这里**不**再查一次 profile：`find_credentials` 的 SQL 已带
        //    `deleted_at IS NULL`，多查一次只会引入「两次读之间用户被删」的窗口。
        repo::record_login_success(&self.pool, &creds.id, now).await?;
        self.issue_session(NewSession {
            user_id: &creds.id,
            username_norm: &norm,
            // ⚠️ 角色取自**刚验过密码的那一行**（`creds.role`），
            //    而不是二次查询的 `profile.role`：
            //    两次读之间若发生角色变更，用哪个都是「某一瞬间的事实」，
            //    但只有前者与「被验证的凭据」同源，可解释。
            role: creds.role,
            now_ms: now,
            origin: SessionOrigin::Login,
            user_agent,
            peer_addr,
        })
        .await
    }

    /// 轮换令牌（旧令牌作废、签发新令牌）。
    ///
    /// 旧令牌被再次使用时判为**令牌重放**并连坐撤销整个家族 ——
    /// 这是刷新令牌轮换的核心价值：偷到旧令牌的人无法在被发现后继续用。
    pub async fn refresh(&self, token: &str) -> Result<AuthSession, ControlError> {
        let now = self.clock.now_millis();
        let digest = session_token_digest(token)?;
        let row = repo::find_session_by_digest(&self.pool, &digest)
            .await?
            .ok_or(ControlError::SessionUnknown)?;

        if row.revoked_at_ms.is_some() {
            repo::revoke_family(
                &self.pool,
                &row.family_id,
                now,
                "检出令牌重放：整个家族连坐撤销",
            )
            .await?;
            return Err(ControlError::SessionReuseDetected {
                family: row.family_id.to_string(),
            });
        }
        if row.expires_at_ms <= now {
            return Err(ControlError::SessionExpired {
                expired_at_ms: row.expires_at_ms,
            });
        }
        if row.user_status != UserStatus::Active {
            return Err(ControlError::AccountDisabled {
                username_norm: String::new(),
            });
        }
        if row.issued_at_ms < row.pwd_changed_at_ms {
            return Err(ControlError::SessionRevoked {
                reason: "改密后需重新登录".to_string(),
            });
        }

        let profile = self.profile_of(&row.user_id).await?;
        repo::revoke_session(&self.pool, &row.id, now, "轮换：被新令牌取代").await?;
        self.issue_session(NewSession {
            user_id: &row.user_id,
            username_norm: &profile.username_norm,
            role: profile.role,
            now_ms: now,
            origin: SessionOrigin::Rotation {
                family_id: row.family_id,
                parent_id: row.id,
            },
            // ⚠️ 轮换**不更新** user_agent / peer_addr：
            //    它们描述的是「这次登录用的浏览器」，而轮换发生在同一条会话链上，
            //    覆盖成 `None` 会把来源信息抹掉（审计价值归零）。
            user_agent: None,
            peer_addr: None,
        })
        .await
    }

    /// 登出。
    ///
    /// 幂等：令牌本来就不存在时返回 `revoked = 0` 而**不**报错 ——
    /// 用户的意图「让我处于未登录状态」已经满足。
    /// 格式非法的令牌仍然报错（那是调用方的 bug，不是用户的意图）。
    pub async fn logout(&self, token: &str) -> Result<LogoutOutcome, ControlError> {
        let now = self.clock.now_millis();
        let digest = session_token_digest(token)?;
        let Some(row) = repo::find_session_by_digest(&self.pool, &digest).await? else {
            return Ok(LogoutOutcome { revoked: 0 });
        };
        let revoked = repo::revoke_session(&self.pool, &row.id, now, "用户登出").await?;
        Ok(LogoutOutcome { revoked })
    }

    /// 校验令牌，返回调用者身份。
    ///
    /// 校验项（顺序即优先级）：格式 → 存在 → 未撤销 → 未过期 → 账号启用 → 会话不早于改密时刻。
    pub async fn authenticate(&self, token: &str) -> Result<Authenticated, ControlError> {
        let now = self.clock.now_millis();
        let digest = session_token_digest(token)?;
        let row = repo::find_session_by_digest(&self.pool, &digest)
            .await?
            .ok_or(ControlError::SessionUnknown)?;

        if row.revoked_at_ms.is_some() {
            return Err(ControlError::SessionRevoked {
                reason: row
                    .revoked_reason
                    .clone()
                    .unwrap_or_else(|| "已撤销（未记录原因）".to_string()),
            });
        }
        if row.expires_at_ms <= now {
            return Err(ControlError::SessionExpired {
                expired_at_ms: row.expires_at_ms,
            });
        }
        if row.user_status != UserStatus::Active {
            return Err(ControlError::AccountDisabled {
                username_norm: String::new(),
            });
        }
        if row.issued_at_ms < row.pwd_changed_at_ms {
            return Err(ControlError::SessionRevoked {
                reason: "改密后需重新登录".to_string(),
            });
        }
        let profile = self.profile_of(&row.user_id).await?;
        Ok(Authenticated {
            session_id: row.id,
            user_id: row.user_id,
            profile,
            expires_at_ms: row.expires_at_ms,
        })
    }

    // ═══════════════════ 自助与管理员操作 ═══════════════════

    /// 改自己的密码，返回被撤销的会话数。
    ///
    /// 副作用：撤销该用户**全部**会话（含当前这个）——
    /// 改密的理由通常就是「怀疑别人拿到了会话」，留着旧会话等于没改。
    /// 同时 `users.pwd_changed_at` 前进，使任何漏网的会话在
    /// [`ControlPlane::authenticate`] 里被判失效（双保险，不依赖撤销成功）。
    pub async fn change_password(
        &self,
        actor: &UserId,
        old_password: &str,
        new_password: &str,
    ) -> Result<u64, ControlError> {
        let now = self.clock.now_millis();
        let profile = self.profile_of(actor).await?;
        let creds = repo::find_credentials(&self.pool, &profile.username_norm)
            .await?
            .ok_or(ControlError::UserNotFound)?;
        if !self.hasher.verify_stored(&creds.digest, old_password) {
            return Err(ControlError::CredentialsRejected);
        }
        validate_password(new_password, &profile.username_norm)?;
        let digest = self.hasher.hash(self.entropy.as_ref(), new_password);
        repo::replace_password(&self.pool, actor, &digest, now).await?;
        repo::revoke_user_sessions(&self.pool, actor, now, "改密后强制重新登录").await
    }

    /// 撤销某用户的全部会话（自己或 owner）。
    pub async fn revoke_all_sessions(
        &self,
        actor: &UserId,
        target: &UserId,
    ) -> Result<u64, ControlError> {
        if actor != target {
            self.require_owner(actor, "撤销他人会话").await?;
        }
        repo::revoke_user_sessions(
            &self.pool,
            target,
            self.clock.now_millis(),
            "管理员撤销全部会话",
        )
        .await
    }

    /// 读一个用户的档案（自己或 owner）。
    ///
    /// **这是隔离性测试的主战场**：A 读不到 B。
    pub async fn get_user(
        &self,
        actor: &UserId,
        target: &UserId,
    ) -> Result<UserProfile, ControlError> {
        if actor != target {
            self.require_owner(actor, "查看他人资料").await?;
        }
        self.profile_of(target).await
    }

    /// 列出全部用户（owner only）。
    pub async fn list_users(&self, actor: &UserId) -> Result<Vec<UserProfile>, ControlError> {
        self.require_owner(actor, "列出全部用户").await?;
        repo::list_profiles(&self.pool).await
    }

    /// 启用 / 禁用某账号（owner only）。
    ///
    /// 禁用后该用户**已签发的会话立即失效**
    /// （[`ControlPlane::authenticate`] 每次都查用户状态），
    /// 不依赖批量撤销任务 —— 「禁用」是应急动作，应急动作不能等后台。
    pub async fn set_user_status(
        &self,
        actor: &UserId,
        target: &UserId,
        status: UserStatus,
    ) -> Result<(), ControlError> {
        self.require_owner(actor, "启用或禁用账号").await?;
        if actor == target && status == UserStatus::Disabled {
            return Err(ControlError::SelfDisableForbidden);
        }
        repo::set_status(&self.pool, target, status, self.clock.now_millis()).await
    }

    // ═══════════════════ 邀请码 ═══════════════════

    /// 签发邀请码（owner only）。
    ///
    /// 明文码**只在此刻返回一次**，库里只存 SHA-256 摘要 → 丢了就换一张。
    pub async fn create_invite(
        &self,
        actor: &UserId,
        role: UserRole,
        max_uses: i32,
    ) -> Result<IssuedInvite, ControlError> {
        self.require_owner(actor, "签发邀请码").await?;
        if !(1..=MAX_INVITE_USES).contains(&max_uses) {
            return Err(ControlError::InviteUsesOutOfRange {
                got: max_uses,
                max: MAX_INVITE_USES,
            });
        }
        let now = self.clock.now_millis();
        let code = new_invite_code(self.entropy.as_ref());
        let digest = invite_code_digest(&code)?;
        let id = UuidBytes::from_bytes(self.new_uuid_bytes());
        let expires_at = now.saturating_add(self.policy.invite_ttl_millis);
        repo::insert_invite(
            &self.pool,
            repo::NewInvite {
                id: &id,
                code_digest: &digest,
                created_by: actor,
                role,
                max_uses,
                expires_at_ms: expires_at,
                created_at_ms: now,
            },
        )
        .await?;
        Ok(IssuedInvite {
            id,
            code,
            role,
            expires_at_ms: expires_at,
            max_uses,
        })
    }

    /// 撤销邀请码（owner only，且只能撤销自己签发的）。
    pub async fn revoke_invite(
        &self,
        actor: &UserId,
        invite_id: &UuidBytes,
    ) -> Result<bool, ControlError> {
        self.require_owner(actor, "撤销邀请码").await?;
        let changed =
            repo::revoke_invite_owned(&self.pool, invite_id, actor, self.clock.now_millis())
                .await?;
        if changed {
            Ok(true)
        } else {
            Err(ControlError::InviteUnknown)
        }
    }

    /// 列出**自己签发的**邀请码（owner only）。
    ///
    /// ⚠️ 归属过滤在**应用层**做（而不是 SQL 的 `WHERE created_by=?`）：
    /// 这样「这张码是谁的」只有一个判定口径 —— `revoke_invite_owned` 的
    /// `WHERE created_by=?`。两处若用不同写法，迟早会漂移。
    pub async fn list_invites(&self, actor: &UserId) -> Result<Vec<InviteSummary>, ControlError> {
        self.require_owner(actor, "列出邀请码").await?;
        Ok(repo::list_invites(&self.pool)
            .await?
            .into_iter()
            .filter(|i| &i.created_by == actor)
            .map(|i| InviteSummary {
                id: i.id,
                created_by: i.created_by,
                role: i.role,
                max_uses: i.max_uses,
                used_count: i.used_count,
                expires_at_ms: i.expires_at_ms,
                created_at_ms: i.created_at_ms,
                revoked: i.revoked_at_ms.is_some(),
            })
            .collect())
    }

    // ═══════════════════ 内部工具 ═══════════════════

    /// 校验并归一化一份建号请求，返回 `(原始用户名, 规范化用户名, 显示名)`。
    fn validate_registration(
        &self,
        req: &RegistrationRequest,
    ) -> Result<(String, String, String), ControlError> {
        let norm = normalize_username(&req.username);
        validate_username(&norm)?;
        let display = req.display_name.trim().to_string();
        validate_display_name(&display)?;
        validate_password(&req.password, &norm)?;
        Ok((req.username.trim().to_string(), norm, display))
    }

    /// 要求 `actor` 是 owner。
    async fn require_owner(
        &self,
        actor: &UserId,
        operation: &'static str,
    ) -> Result<(), ControlError> {
        let p = self.profile_of(actor).await?;
        if p.role.is_owner() {
            Ok(())
        } else {
            Err(ControlError::NotAnOwner { operation })
        }
    }

    /// 读档案，不存在则 [`ControlError::UserNotFound`]。
    async fn profile_of(&self, id: &UserId) -> Result<UserProfile, ControlError> {
        repo::find_profile(&self.pool, id)
            .await?
            .ok_or(ControlError::UserNotFound)
    }

    /// 签发一个新会话（内部）。
    ///
    /// 入参打包成 [`NewSession`] 而非 8 个位置参数：
    /// 参数一多，调用点就迟早出现「两个相邻的 `None` 传反了」这类错误，
    /// 而 `None` 传反在类型上完全合法（这正是 clippy `too_many_arguments`
    /// 反复报的那类问题）。
    async fn issue_session(&self, spec: NewSession<'_>) -> Result<AuthSession, ControlError> {
        let NewSession {
            user_id,
            username_norm,
            role,
            now_ms,
            origin,
            user_agent,
            peer_addr,
        } = spec;
        let token = new_session_token(self.entropy.as_ref());
        let digest = session_token_digest(&token)?;
        let id = SessionId::from_bytes(self.new_uuid_bytes());
        let (family_id, parent_id, ttl) = match origin {
            SessionOrigin::Login => (id, None, self.policy.session_ttl_millis),
            SessionOrigin::Rotation {
                family_id,
                parent_id,
            } => (family_id, Some(parent_id), self.policy.refresh_ttl_millis),
        };
        let expires_at = now_ms.saturating_add(ttl);
        repo::insert_session(
            &self.pool,
            repo::NewSession {
                id: &id,
                user_id,
                token_digest: &digest,
                family_id: &family_id,
                parent_id: parent_id.as_ref(),
                issued_at_ms: now_ms,
                expires_at_ms: expires_at,
                user_agent,
                peer_addr,
            },
        )
        .await?;
        Ok(AuthSession {
            session_id: id,
            user_id: *user_id,
            username_norm: username_norm.to_string(),
            role,
            token,
            issued_at_ms: now_ms,
            expires_at_ms: expires_at,
        })
    }

    /// 造 16 字节标识（系统熵池，见 [`crate::secret::OsEntropySource`]）。
    fn new_uuid_bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        self.entropy.fill(&mut out);
        out
    }
}

/// 一次会话签发的全部输入（内部）。
///
/// 之所以打包成结构体而不是 8 个位置参数：相邻的 `None` 传反了在类型上完全合法，
/// 而 `user_agent` / `peer_addr` 恰好就是两个相邻的 `Option`。
#[derive(Debug, Clone, Copy)]
struct NewSession<'a> {
    user_id: &'a UserId,
    username_norm: &'a str,
    role: UserRole,
    now_ms: i64,
    origin: SessionOrigin,
    user_agent: Option<&'a str>,
    peer_addr: Option<&'a str>,
}

/// 新会话的来源（决定 `family_id` / `parent_id` / 有效期）。
#[derive(Debug, Clone, Copy)]
enum SessionOrigin {
    /// 首次登录：自成一个家族。
    Login,
    /// 轮换：沿用原家族并记父会话。
    Rotation {
        /// 原家族。
        family_id: SessionId,
        /// 被替换掉的会话。
        parent_id: SessionId,
    },
}
