use std::sync::{Arc, OnceLock};

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;

use crate::db::DbBridge;
use crate::error::ApiError;
use crate::state::AppState;
use quill_domain::UserId;

#[derive(Clone, PartialEq, Eq)]
pub struct AuthContext {
    pub user_id: quill_domain::UserId,

    pub is_admin: bool,
}

impl std::fmt::Debug for AuthContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthContext")
            .field("user_id", &self.user_id)
            .field("is_admin", &self.is_admin)
            .finish()
    }
}

pub trait TokenResolver: Send + Sync + 'static {
    fn resolve(&self, token: &str) -> Result<AuthContext, TokenRejected>;

    /// 向下拿到复合解析器本体（若装的确实是它）。
    ///
    /// 只给 `/api/auth/me` 这类「需要额外信息」的少数端点用；日常鉴权只调
    /// `resolve`，不碰这里。加这个方法而不是把具体类型塞进 `AppState`，
    /// 是为了让「令牌从哪来」这件事在类型上就分得清。
    fn as_any_composite(&self) -> Option<&CompositeTokenResolver> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRejected {
    Unknown,

    Malformed,
}

impl std::fmt::Display for TokenRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => f.write_str("令牌不存在"),
            Self::Malformed => f.write_str("令牌格式非法"),
        }
    }
}

#[derive(Debug, Default)]
pub struct EnvTokenResolver {
    entries: Vec<TokenEntry>,
}

#[derive(Debug, Clone)]
struct TokenEntry {
    token: String,
    ctx: AuthContext,
    login: String,
}

impl EnvTokenResolver {
    pub fn new(entries: Vec<(String, AuthContext)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(token, ctx)| {
                    let login = ctx.user_id.to_compact_hex();
                    TokenEntry { token, ctx, login }
                })
                .collect(),
        }
    }

    /// 令牌背后的人，含可用于建 `users` 行的登录名。
    pub fn subjects(&self) -> Vec<(UserId, String, bool)> {
        let mut out: Vec<(UserId, String, bool)> = self
            .entries
            .iter()
            .map(|e| (e.ctx.user_id, e.login.clone(), e.ctx.is_admin))
            .collect();
        out.sort_by_key(|e| e.0.to_compact_hex());
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }

    pub fn from_env() -> (Self, Vec<super::config::Warning>) {
        let mut warnings = Vec::new();
        let raw = match std::env::var("QUILL_TOKENS") {
            Ok(r) => r,
            Err(_) => {
                warnings.push(super::config::Warning {
                    source: "QUILL_TOKENS".to_string(),
                    message:
                        "未设置，当前**所有** /api 路由都会返回 401。\
                         这是刻意的：宁可全部拒绝，也不要无鉴权放行。\
                         调试用可设为 `dev-token:@alice:admin`（`@用户名` 会按用户名推导 ID，\
                         与 `quill --as alice` 指向同一个人）。"
                            .to_string(),
                });
                return (Self::default(), warnings);
            }
        };

        let mut entries = Vec::new();
        for item in raw.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let mut parts = item.split(':');
            let (Some(token), Some(user_raw)) = (parts.next(), parts.next()) else {
                warnings.push(super::config::Warning {
                    source: "QUILL_TOKENS".to_string(),
                    message: format!(
                        "条目 {item:?} 格式非法（应为 `令牌:身份[:admin]`，身份写 @用户名 或 32 位 ID），已跳过。"
                    ),
                });
                continue;
            };
            let is_admin = match parts.next() {
                None => false,
                Some("admin") => true,
                Some(other) => {
                    warnings.push(super::config::Warning {
                        source: "QUILL_TOKENS".to_string(),
                        message: format!("条目 {item:?} 的角色 {other:?} 不认识（只支持 `admin`），按普通用户处理。"),
                    });
                    false
                }
            };
            if parts.next().is_some() {
                warnings.push(super::config::Warning {
                    source: "QUILL_TOKENS".to_string(),
                    message: format!("条目 {item:?} 字段过多（应为 2~3 段），已跳过。"),
                });
                continue;
            }
            match quill_control::parse_token_subject(user_raw) {
                Ok(subject) => {
                    let declared_login = match &subject {
                        quill_control::TokenSubject::Named(n) => Some(n.clone()),
                        quill_control::TokenSubject::Id(_) => None,
                    };
                    match subject.resolve() {
                        Ok(user_id) => entries.push(TokenEntry {
                            token: token.to_string(),
                            ctx: AuthContext { user_id, is_admin },
                            login: declared_login
                                .unwrap_or_else(|| user_id.to_compact_hex()),
                        }),
                        Err(e) => warnings.push(super::config::Warning {
                            source: "QUILL_TOKENS".to_string(),
                            message: format!("条目 {item:?} 的身份无法解析，已跳过：{e}"),
                        }),
                    }
                }
                Err(e) => warnings.push(super::config::Warning {
                    source: "QUILL_TOKENS".to_string(),
                    message: format!("条目 {item:?} 的身份非法，已跳过：{e}"),
                }),
            }
        }
        (Self { entries }, warnings)
    }
}

impl TokenResolver for EnvTokenResolver {
    fn resolve(&self, token: &str) -> Result<AuthContext, TokenRejected> {
        if token.is_empty() {
            return Err(TokenRejected::Malformed);
        }
        self.entries
            .iter()
            .find(|e| e.token == token)
            .map(|e| e.ctx.clone())
            .ok_or(TokenRejected::Unknown)
    }
}

/// 会话令牌解析：查 `sessions_auth` 表。
///
/// 与 `EnvTokenResolver` 分开的原因：前者是**启动时从环境变量读进内存**的静态表，
/// 进程内查表即可；会话令牌是 `POST /api/auth/login` 运行时签发的，必须落库，
/// 而且登出后要立刻失效。两种令牌的存储位置与失效方式根本不同，硬塞进一个结构
/// 只会让「这份令牌是不是查库」变成隐式约定。
///
/// 数据库句柄用 `OnceLock` 后装：解析器在 `build_state` 开头就构造，
/// 而 `DbBridge` 要等迁移跑完才有，顺序反过来会形成构造环。
#[derive(Debug, Default)]
pub struct SessionTokenResolver {
    db: OnceLock<Arc<DbBridge>>,
}

impl SessionTokenResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// 建库之后回填。重复调用无效且不算错误（测试里可能换实例）。
    pub fn attach(&self, db: Arc<DbBridge>) {
        let _ = self.db.set(db);
    }

    pub fn attached(&self) -> bool {
        self.db.get().is_some()
    }

    /// 解析会话令牌 → 用户上下文。查不到一律 `Unknown`，与格式错误区分开只是为了
    /// 服务端日志，对外两者都是同一个 401（见 `error.rs` 的枚举防测试）。
    fn resolve_via_db(&self, token: &str) -> Result<AuthContext, TokenRejected> {
        let Some(db) = self.db.get() else {
            return Err(TokenRejected::Unknown);
        };
        let token = token.to_string();
        // 与 api_auth::with_control 同一手法：结果带出，通道上只走 AgentError。
        let slot: Arc<std::sync::Mutex<Option<Result<AuthContext, TokenRejected>>>> =
            Arc::new(std::sync::Mutex::new(None));
        let writer = Arc::clone(&slot);
        let reader = Arc::clone(&slot);
        db.call(move |pool, _rt| {
            Box::pin(async move {
                let cp = quill_control::ControlPlane::new_with_os_entropy(
                    pool,
                    Arc::new(quill_control::SystemClock),
                    quill_control::Pbkdf2Params::production(),
                );
                let outcome = match cp.authenticate(&token).await {
                    Ok(a) => Ok(AuthContext {
                        user_id: a.user_id,
                        is_admin: a.profile.role == quill_control::UserRole::Owner,
                    }),
                    Err(e) => {
                        eprintln!("[auth] 会话令牌校验失败：{e}");
                        Err(TokenRejected::Unknown)
                    }
                };
                *writer.lock().expect("结果槽位不该被毒化") = Some(outcome);
                Ok(())
            })
        })
        .map_err(|e| {
            eprintln!("[auth] 校验会话令牌时 DbBridge 通道失败：{e}");
            TokenRejected::Unknown
        })?;
        let out = reader.lock().expect("结果槽位不该被毒化").take();
        out.unwrap_or(Err(TokenRejected::Unknown))
    }
}

impl TokenResolver for SessionTokenResolver {
    fn resolve(&self, token: &str) -> Result<AuthContext, TokenRejected> {
        if token.is_empty() {
            return Err(TokenRejected::Malformed);
        }
        self.resolve_via_db(token)
    }
}

/// 复合解析器：先查静态的环境变量令牌表，查不到再查会话表。
///
/// 顺序有意如此：`QUILL_TOKENS` 是部署者直给的凭据，应当在热路径上零查库命中，
/// 开发与应急场景（数据库出问题时）也仍然进得来。代价是每**一个**没配在环境变量
/// 里的令牌都会落到一次 DB 查询 —— 这是登录后每个请求的真实成本，记在这里以免
/// 以后有人把它当 bug 顺手改掉顺序。
#[derive(Debug)]
pub struct CompositeTokenResolver {
    env: EnvTokenResolver,
    session: Arc<SessionTokenResolver>,
}

impl CompositeTokenResolver {
    /// 返回解析器本体，以及会话解析器的句柄 —— `build_state` 在建库之后要拿这个
    /// 句柄 `attach()` 句柄，二者必须是**同一个实例**，否则回填的 DbBridge 落空。
    pub fn new(env: EnvTokenResolver) -> (Self, Arc<SessionTokenResolver>) {
        let session = Arc::new(SessionTokenResolver::new());
        (
            Self {
                env,
                session: Arc::clone(&session),
            },
            session,
        )
    }

    pub fn env(&self) -> &EnvTokenResolver {
        &self.env
    }

    /// 这个 `user_id` 对应的环境变量登录名（`QUILL_TOKENS` 里写的 `@alice` 或 id）。
    /// 登录签发的会话令牌的人不在这里，必须回库查。
    pub fn env_login_of(&self, user_id: UserId) -> Option<String> {
        self.env
            .subjects()
            .into_iter()
            .find(|(id, _, _)| *id == user_id)
            .map(|(_, login, _)| login)
    }
}

impl TokenResolver for CompositeTokenResolver {
    fn resolve(&self, token: &str) -> Result<AuthContext, TokenRejected> {
        if token.is_empty() {
            return Err(TokenRejected::Malformed);
        }
        match self.env.resolve(token) {
            Ok(ctx) => Ok(ctx),
            Err(TokenRejected::Malformed) => Err(TokenRejected::Malformed),
            Err(TokenRejected::Unknown) => self.session.resolve(token),
        }
    }

    fn as_any_composite(&self) -> Option<&CompositeTokenResolver> {
        Some(self)
    }
}

pub fn require_auth(state: &AppState, parts: &mut Parts) -> Result<AuthContext, ApiError> {
    let header = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let Some(header) = header else {
        return Err(ApiError::unauthorized());
    };
    let Some(token) = header.strip_prefix("Bearer ") else {
        return Err(ApiError::unauthorized());
    };
    let token = token.trim();

    match state.tokens.resolve(token) {
        Ok(ctx) => Ok(ctx),
        Err(reason) => {
            eprintln!(
                "[auth] 401 请求路径 {} —— 令牌被拒：{reason}",
                parts.uri.path()
            );
            Err(ApiError::unauthorized())
        }
    }
}

pub struct AuthUser(pub AuthContext);

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let ctx = require_auth(&app, parts)?;

        parts.extensions.insert(ctx.clone());
        Ok(AuthUser(ctx))
    }
}

/// 只在 `is_admin == true` 时才放行，否则返回 403 + 中文 next_step。
/// 不能靠 `AuthUser` 内层再判一次来替代：那样会让 unauth 走 401、auth-but-not-admin
/// 走 403，跟路由是否「需要 admin」的语义在响应里看不出来；写成独立 extractor
/// 把这条意图钉死。
pub struct RequireAdmin(pub AuthContext);

impl<S> FromRequestParts<S> for RequireAdmin
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let ctx = require_auth(&app, parts)?;
        if !ctx.is_admin {
            return Err(ApiError::forbidden(
                "实例级配置（/api/admin/*）只允许 admin 令牌访问。下一步：\
                 在 QUILL_TOKENS 中把令牌标记为 admin（条目末尾加 `:admin`），\
                 或换用一个 admin 令牌重新发起请求。",
            ));
        }
        parts.extensions.insert(ctx.clone());
        Ok(RequireAdmin(ctx))
    }
}

impl FromRef<Arc<AppState>> for AppState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        state.as_ref().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UID: &str = "0192b7c8-0000-7000-8000-000000000001";

    #[tokio::test]
    async fn unknown_token_and_malformed_token_yield_identical_rejection() {
        let r = EnvTokenResolver::new(vec![(
            "good".to_string(),
            AuthContext {
                user_id: quill_domain::UserId::parse(UID).expect("测试用 UID 必须合法"),
                is_admin: false,
            },
        )]);
        let unknown = r.resolve("nope").unwrap_err();
        let malformed = r.resolve("").unwrap_err();

        assert_ne!(unknown, malformed, "服务端内部应能区分原因");
        let e1 = ApiError::unauthorized();
        let e2 = ApiError::unauthorized();
        assert_eq!(e1.detail(), e2.detail(), "对外文案必须逐字相同");
        assert_eq!(e1.status(), e2.status());
    }

    #[tokio::test]
    async fn valid_token_resolves_to_user_id() {
        let r = EnvTokenResolver::new(vec![(
            "good".to_string(),
            AuthContext {
                user_id: quill_domain::UserId::parse(UID).expect("合法"),
                is_admin: true,
            },
        )]);
        let ctx = r.resolve("good").expect("合法令牌应解析成功");
        assert!(ctx.is_admin);
        assert_eq!(ctx.user_id.to_compact_hex().len(), 32);
    }

    #[test]
    fn auth_context_debug_does_not_leak_token() {
        let ctx = AuthContext {
            user_id: quill_domain::UserId::parse(UID).expect("合法"),
            is_admin: false,
        };
        let d = format!("{ctx:?}");
        assert!(d.contains("user_id"));

        assert!(!d.contains("token"));
    }
}
