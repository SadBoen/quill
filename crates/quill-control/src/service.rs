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

pub const DEFAULT_LOCALE: &str = "zh-CN";

pub const MAX_INVITE_USES: i32 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthPolicy {
    pub max_failures: i32,

    pub lock_millis: i64,

    pub session_ttl_millis: i64,

    pub refresh_ttl_millis: i64,

    pub invite_ttl_millis: i64,
}

impl AuthPolicy {
    pub const fn production() -> Self {
        Self {
            max_failures: 5,

            lock_millis: 15 * 60 * 1000,

            session_ttl_millis: 7 * 24 * 60 * 60 * 1000,

            refresh_ttl_millis: 24 * 60 * 60 * 1000,
            invite_ttl_millis: 7 * 24 * 60 * 60 * 1000,
        }
    }

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

#[derive(Clone, PartialEq, Eq)]
pub struct RegistrationRequest {
    pub username: String,

    pub display_name: String,

    pub password: String,

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

    pub fn with_role(mut self, role: UserRole) -> Self {
        self.role = role;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSession {
    pub session_id: SessionId,

    pub user_id: UserId,

    pub username_norm: String,

    pub role: UserRole,

    pub token: String,

    pub issued_at_ms: i64,

    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authenticated {
    pub session_id: SessionId,

    pub user_id: UserId,

    pub profile: UserProfile,

    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedInvite {
    pub id: UuidBytes,

    pub code: String,

    pub role: UserRole,

    pub expires_at_ms: i64,

    pub max_uses: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteSummary {
    pub id: UuidBytes,

    pub created_by: UserId,

    pub role: UserRole,

    pub max_uses: i32,

    pub used_count: i32,

    pub expires_at_ms: i64,

    pub created_at_ms: i64,

    pub revoked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogoutOutcome {
    pub revoked: u64,
}

pub struct ControlPlane {
    pool: SqlitePool,
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn SecretSource>,
    hasher: PasswordHasher,
    policy: AuthPolicy,
}

impl std::fmt::Debug for ControlPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlPlane")
            .field("hasher", &self.hasher)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl ControlPlane {
    pub fn new_with_os_entropy(
        pool: SqlitePool,
        clock: Arc<dyn Clock>,
        params: Pbkdf2Params,
    ) -> Self {
        Self::new(pool, clock, Arc::new(OsEntropySource), params)
    }

    pub fn new(
        pool: SqlitePool,
        clock: Arc<dyn Clock>,
        entropy: Arc<dyn SecretSource>,
        params: Pbkdf2Params,
    ) -> Self {
        Self::with_policy(pool, clock, entropy, params, AuthPolicy::production())
    }

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

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn policy(&self) -> AuthPolicy {
        self.policy
    }

    pub async fn create_first_owner(
        &self,
        req: &RegistrationRequest,
    ) -> Result<UserProfile, ControlError> {
        let now = self.clock.now_millis();
        let (username, norm, display) = self.validate_registration(req)?;

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

    pub async fn login(
        &self,
        username: &str,
        password: &str,
        user_agent: Option<&str>,
        peer_addr: Option<&str>,
    ) -> Result<AuthSession, ControlError> {
        let now = self.clock.now_millis();
        let norm = normalize_username(username);

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

            return Err(ControlError::CredentialsRejected);
        }

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

        repo::record_login_success(&self.pool, &creds.id, now).await?;
        self.issue_session(NewSession {
            user_id: &creds.id,
            username_norm: &norm,

            role: creds.role,
            now_ms: now,
            origin: SessionOrigin::Login,
            user_agent,
            peer_addr,
        })
        .await
    }

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

            user_agent: None,
            peer_addr: None,
        })
        .await
    }

    pub async fn logout(&self, token: &str) -> Result<LogoutOutcome, ControlError> {
        let now = self.clock.now_millis();
        let digest = session_token_digest(token)?;
        let Some(row) = repo::find_session_by_digest(&self.pool, &digest).await? else {
            return Ok(LogoutOutcome { revoked: 0 });
        };
        let revoked = repo::revoke_session(&self.pool, &row.id, now, "用户登出").await?;
        Ok(LogoutOutcome { revoked })
    }

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

    pub async fn list_users(&self, actor: &UserId) -> Result<Vec<UserProfile>, ControlError> {
        self.require_owner(actor, "列出全部用户").await?;
        repo::list_profiles(&self.pool).await
    }

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

    async fn profile_of(&self, id: &UserId) -> Result<UserProfile, ControlError> {
        repo::find_profile(&self.pool, id)
            .await?
            .ok_or(ControlError::UserNotFound)
    }

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

    fn new_uuid_bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        self.entropy.fill(&mut out);
        out
    }
}

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

#[derive(Debug, Clone, Copy)]
enum SessionOrigin {
    Login,

    Rotation {
        family_id: SessionId,

        parent_id: SessionId,
    },
}
