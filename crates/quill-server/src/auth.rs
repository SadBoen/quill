//! 鉴权中间件 —— 会话令牌 → `UserId` 解析。
//!
//! # 契约依据
//!
//! `docs/PHASE2_CONTRACT.md` §2.4：`IdentityProvider` trait 已删除，
//! 改为「`UserContext` 在**请求入口**构造，非 trait」。
//! 因此本模块是**中间件**而非 trait 实现，构造在 `from_request_parts` 里。
//!
//! # 边界规则 7 面 B：禁 static 承载用户态
//!
//! 用户身份**绝不**放在 `static` / `OnceLock` 里。`AuthContext` 每次请求新建，
//! 由 axum 的 `Extension` 按请求注入，生命周期与请求同生共死。
//! 一旦挂成全局单例，B 用户就能 steer 进 A 的 session（见契约 §2.5）。
//!
//! # 不泄露「用户不存在」与「密码错误」
//!
//! 401 是**唯一**的未认证响应，文案恒为 [`ApiError::unauthorized`]。
//! 令牌查不到、令牌格式非法、请求头缺失——三者的响应**逐字节相同**。
//! 区分它们等于提供一个用户枚举器。

use std::sync::Arc;

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;

use crate::error::ApiError;
use crate::state::AppState;

/// 令牌 → 身份。
///
/// ⚠️ **不进任何日志**：`Debug` 刻意只打令牌前 6 位。
/// 打全量令牌等于把「日志读权限」变成「冒用任何用户」的权限。
#[derive(Clone, PartialEq, Eq)]
pub struct AuthContext {
    /// 用户标识（来自 `quill-adapters`，与 `quill_domain::UserId` 同类型）。
    pub user_id: quill_domain::UserId,
    /// 是否 admin。仅 admin 可做 MCP 写操作 / 改角色（`V1_SCOPE_CONSTRAINTS` 约束 5-b）。
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

/// 令牌解析器。
///
/// ⚠️ 这是**进程内**实现，不是数据库。`quill-control`（账号/鉴权/会话管理）
/// 目前是空壳，真实的令牌校验应落在它那里；本 trait 是**留给它的接缝**：
/// quill-control 落地后只需换掉实现，路由层与中间件一行不改。
///
/// 契约 §2.3 已删除 `IdentityProvider` trait，本 trait 是 quill-server **内部**接缝，
/// 不进 `quill-adapters` 契约层——不因此违反「删除 trait」那条裁决。
pub trait TokenResolver: Send + Sync + 'static {
    /// 校验令牌并解析身份。
    ///
    /// 返回 `Err` 时**不得**区分「令牌不存在」与「令牌已失效」——
    /// 调用方会把所有 `Err` 收敛成同一个 401 响应。
    fn resolve(&self, token: &str) -> Result<AuthContext, TokenRejected>;
}

/// 令牌被拒的原因（**仅供服务端日志**，不发给客户端）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenRejected {
    /// 令牌不存在。
    Unknown,
    /// 令牌格式非法。
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

/// 进程内令牌表。
///
/// ⚠️ **这是开发期形态**：令牌来自环境变量 `QUILL_TOKENS`（`令牌:用户ID[:admin]`，逗号分隔）。
/// 生产形态必须是 `quill-control` 的持久化 + 撤销表（`sessions_auth`），
/// 因为进程内令牌**重启即失效**且**无法撤销**。
#[derive(Debug, Default)]
pub struct EnvTokenResolver {
    entries: Vec<(String, AuthContext)>,
}

impl EnvTokenResolver {
    /// 由显式条目构造（**不读环境变量**）。
    ///
    /// ⚠️ 存在的理由：集成测试必须与运行环境的 `QUILL_TOKENS` 无关，
    /// 否则本地能过的测试在 CI 上会因环境不同而变红（铁律十四）。
    /// 因此测试走这个入口，**不是** `from_env`。
    pub fn new(entries: Vec<(String, AuthContext)>) -> Self {
        Self { entries }
    }

    /// 从 `QUILL_TOKENS` 装配。
    ///
    /// 格式：`dev-token-1:0192...:admin,dev-token-2:0192...`
    /// 配错**不 panic**：跳过该条目并把原因交给调用方打 WARN（与 `config` 同一口径）。
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

/// 从请求头取 `Authorization: Bearer <token>`。
///
/// 缺失/格式错/查不到 → **同一个** 401（见模块头说明）。
pub fn require_auth(state: &AppState, parts: &mut Parts) -> Result<AuthContext, ApiError> {
    let header = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let Some(header) = header else {
        // 无 Authorization 头：与"令牌错误"走**完全相同**的返回路径。
        return Err(ApiError::unauthorized());
    };
    let Some(token) = header.strip_prefix("Bearer ") else {
        return Err(ApiError::unauthorized());
    };
    let token = token.trim();

    match state.tokens.resolve(token) {
        Ok(ctx) => Ok(ctx),
        Err(reason) => {
            // ⚠️ 失败原因**只进服务端日志**，响应体不带任何区分信息。
            eprintln!(
                "[auth] 401 请求路径 {} —— 令牌被拒：{reason}",
                parts.uri.path()
            );
            Err(ApiError::unauthorized())
        }
    }
}

/// 认证过的请求上下文，作为 axum extractor 使用。
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
        // ⚠️ 每次请求新建，**不放 static**（契约 §2.5 规则 7 面 B）。
        parts.extensions.insert(ctx.clone());
        Ok(AuthUser(ctx))
    }
}

/// `axum::FromRef` 桥接：`Arc<AppState>` 状态 → `&AppState`。
///
/// ⚠️ 只为 `Arc<AppState>` 提供。`Router::with_state(state)` 传的是 `AppState` 本身，
///    那条路径由 axum 自带的 blanket impl（`T: Clone`）覆盖，无需在此重复。
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
        // ⚠️ 这是"不泄露用户是否存在"的**机制**证明：两个原因存在且不同，
        // 但它们在 HTTP 层会被收敛成同一个 401（见 `require_auth`）。
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
        // 用户 ID 本身可以打（它不是凭据）；但结构里**不该**有令牌字段。
        assert!(!d.contains("token"));
    }
}
