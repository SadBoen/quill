
use std::sync::Arc;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;

use crate::error::ApiError;
use crate::state::AppState;

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
    entries: Vec<(String, AuthContext)>,
}

impl EnvTokenResolver {

    pub fn new(entries: Vec<(String, AuthContext)>) -> Self {
        Self { entries }
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
                              调试用可设为 `dev-token:0192b7c8-0000-7000-8000-000000000001:admin`。"
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
                        "条目 {item:?} 格式非法（应为 `令牌:用户ID[:admin]`），已跳过。"
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
            match quill_domain::UserId::parse(user_raw.trim()) {
                Ok(user_id) => entries.push((token.to_string(), AuthContext { user_id, is_admin })),
                Err(e) => warnings.push(super::config::Warning {
                    source: "QUILL_TOKENS".to_string(),
                    message: format!("条目 {item:?} 的用户 ID 非法（{e}），已跳过。"),
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
            .find(|(t, _)| t == token)
            .map(|(_, c)| c.clone())
            .ok_or(TokenRejected::Unknown)
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
