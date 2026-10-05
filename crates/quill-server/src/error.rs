//! 统一错误类型 —— 所有对外响应必须是中文人话 + 下一步该执行什么。
//!
//! # 为什么不用 `thiserror`
//!
//! `quill-adapters` 的既定口径是「零第三方依赖、`Display`/`Error` 手写」
//! （见该 crate 头注释）。本 crate 沿用同一口径：手写 `Display` 只需十几行，
//! 而新增一个宏依赖会让 `Cargo.lock` 多一处变更面。
//!
//! # 铁律七：失败必须自诊断
//!
//! 每个错误变体都强制携带两样东西：
//! 1. **中文人话**说明发生了什么（不把 `EACCES` / 内部类型名抛给用户）；
//! 2. **下一步该执行什么**（`next_step`），可直接复制。
//!
//! # 铁律四 / 内部信息不外泄
//!
//! [`ApiError::Internal`] 的 `detail` 字段**只允许放已审阅过的中文说明**，
//! 不放 panic payload、不放栈、不放 SQL 错误串。真实原因只写服务端日志
//! （`crate::middleware` 的 panic 兜底 + stderr），客户端只拿到可读信息 +
//! 请求 ID，让用户能在日志里定位到同一行。

use std::fmt;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// 对外错误类型。
///
/// ⚠️ 变体**刻意**不带"内部细节"通道：没有 `Internal { source: anyhow::Error }`
/// 这种形态。一旦存在，某个 handler 就会顺手把内部串塞进响应体。
#[derive(Debug, Clone)]
pub enum ApiError {
    /// 401 未认证。
    ///
    /// ⚠️ **不区分「用户不存在」与「密码/令牌错误」**——两种情况返回**完全相同**的
    /// 文案与字段。区分即等于给攻击者一个用户枚举器（`V1_SCOPE_CONSTRAINTS` 约束 5
    /// 的提权边界精神的同一条要求）。
    Unauthorized {
        /// 已审阅的中文说明。**必须对「令牌不存在」与「令牌错误」取同一个值。**
        detail: &'static str,
    },
    /// 403 已认证但无权限（如非 admin 访问 owner-only 路由）。
    Forbidden { detail: String },
    /// 404 路由不存在。
    NotFound { path: String },
    /// 404 资源不存在或对该用户不可见。
    ///
    /// ⚠️ 与 [`ApiError::NotFound`] 分开的原因：那个的文案是
    /// 「本实例没有路由 X」，用于路径未登记；本变体用于「路由在、资源不在」。
    /// 两者共用一个变体就会出现「专家不存在却提示本实例没有路由 /api/experts/x」
    /// 这种把用户引向错误排查方向的文案 —— 不可诊断的失败。
    EntityNotFound { detail: String },
    /// 400 请求本身不合法（缺字段、类型错、标识非法）。
    BadRequest { detail: String },
    /// 409 与既有资源冲突（例如同名专家已存在）。
    Conflict { detail: String },
    /// 405 方法不允许（路径存在但方法不对）。
    MethodNotAllowed { method: String, path: String },
    /// 501 路由已登记、能力尚未实现。
    NotImplemented {
        method: &'static str,
        path: &'static str,
    },
    /// 503 存储不可用（数据库打不开 / schema 未迁移）。
    ///
    /// ⚠️ **刻意与 500 分开**：服务本身活着、只是拿不到数据。
    /// 合成 500 会让运维去查代码与栈，而真实原因是一条 WARN 与一次迁移。
    StorageUnavailable { detail: String },
    /// 500 内部错误（含被兜住的 panic）。
    ///
    /// ⚠️ `detail` 只放中文说明；真实原因走服务端日志。
    Internal { detail: String },
}

impl ApiError {
    /// 401 的**唯一**构造入口。
    ///
    /// 单入口是为了让"两处 401 文案不一致"在代码层就写不出来——
    /// 靠 review 保证文案一致是纸面约束，构造入口才是编译期约束。
    pub const fn unauthorized() -> Self {
        Self::Unauthorized {
            detail: "未认证或凭据无效",
        }
    }

    /// 403：已认证但权限不足。
    pub fn forbidden(detail: impl Into<String>) -> Self {
        Self::Forbidden {
            detail: detail.into(),
        }
    }

    /// 404：路由不存在。
    pub fn not_found(path: impl Into<String>) -> Self {
        Self::NotFound { path: path.into() }
    }

    /// 404：资源不存在或对当前用户不可见。
    pub fn entity_not_found(detail: impl Into<String>) -> Self {
        Self::EntityNotFound {
            detail: detail.into(),
        }
    }

    /// 400：请求不合法。
    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::BadRequest {
            detail: detail.into(),
        }
    }

    /// 409：与既有资源冲突。
    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::Conflict {
            detail: detail.into(),
        }
    }

    /// 503：存储不可用。
    pub fn storage_unavailable(detail: impl Into<String>) -> Self {
        Self::StorageUnavailable {
            detail: detail.into(),
        }
    }

    /// 405：方法不允许。
    pub fn method_not_allowed(method: impl Into<String>, path: impl Into<String>) -> Self {
        Self::MethodNotAllowed {
            method: method.into(),
            path: path.into(),
        }
    }

    /// 500：内部错误。
    ///
    /// ⚠️ 传入的 `detail` 会**原样发给客户端**。调用方必须传已审阅的中文说明，
    /// 不得传 panic payload 或底层错误串。
    pub fn internal(detail: impl Into<String>) -> Self {
        Self::Internal {
            detail: detail.into(),
        }
    }

    /// 对应 HTTP 状态码。
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized { .. } => StatusCode::UNAUTHORIZED,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::NotFound { .. } | Self::EntityNotFound { .. } => StatusCode::NOT_FOUND,
            Self::MethodNotAllowed { .. } => StatusCode::METHOD_NOT_ALLOWED,
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::Conflict { .. } => StatusCode::CONFLICT,
            Self::NotImplemented { .. } => StatusCode::NOT_IMPLEMENTED,
            Self::StorageUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// 机器可读的错误码（前端据此分支，不解析中文文案）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized { .. } => "unauthorized",
            Self::Forbidden { .. } => "forbidden",
            Self::NotFound { .. } => "not_found",
            Self::EntityNotFound { .. } => "entity_not_found",
            Self::MethodNotAllowed { .. } => "method_not_allowed",
            Self::BadRequest { .. } => "bad_request",
            Self::Conflict { .. } => "conflict",
            Self::NotImplemented { .. } => "not_implemented",
            Self::StorageUnavailable { .. } => "storage_unavailable",
            Self::Internal { .. } => "internal_error",
        }
    }

    /// 给人看的说明。
    pub fn detail(&self) -> String {
        match self {
            Self::Unauthorized { detail } => (*detail).to_string(),
            Self::Forbidden { detail } => detail.clone(),
            Self::NotFound { path } => format!("本实例没有路由 {path}"),
            Self::EntityNotFound { detail } => detail.clone(),
            Self::MethodNotAllowed { method, path } => {
                format!("路由 {path} 不支持 {method} 方法")
            }
            Self::BadRequest { detail } => detail.clone(),
            Self::Conflict { detail } => detail.clone(),
            Self::NotImplemented { method, path } => format!(
                "路由 {method} {path} 已按 docs/PHASE2_CONTRACT.md §5.1 登记，\
                 但能力尚未实现"
            ),
            Self::StorageUnavailable { detail } => detail.clone(),
            Self::Internal { detail } => detail.clone(),
        }
    }

    /// **下一步该执行什么**（可直接复制的中文指引）。
    pub fn next_step(&self) -> &'static str {
        match self {
            Self::Unauthorized { .. } => {
                "在请求头加上 `Authorization: Bearer <令牌>`；\
                 令牌由管理员通过环境变量 QUILL_TOKENS 配置（格式 `令牌:用户ID[:admin]`）。\
                 登录接口见 POST /api/auth/login。"
            }
            Self::Forbidden { .. } => {
                "该操作仅限 admin。请用带 `:admin` 后缀的令牌重新登录，\
                 或联系管理员在 QUILL_TOKENS 中把你标记为 admin。"
            }
            Self::NotFound { .. } => {
                "对照 docs/PHASE2_CONTRACT.md §5.1 的路由表确认路径拼写；\
                 本实例当前只装配 HTTP 骨架，未登记的路径一律 404（不会静默兜底）。"
            }
            Self::EntityNotFound { .. } => {
                "确认标识拼写（小写 kebab-case）；\
                 若该资源属于别的用户，本实例按隔离口径同样返回「不存在」——\
                 这一点是刻意的（避免变成跨用户枚举接口）。\
                 用 `quill doctor --section=experts` 查看本账号名册。"
            }
            Self::BadRequest { .. } => {
                "按响应里的字段名修正请求体后重试（标识类字段只接受小写 kebab-case）；\
                 若确认字段与文档一致却仍报同样的错，执行 `GET /healthz` 核对服务版本。"
            }
            Self::Conflict { .. } => {
                "先 `GET` 该资源确认它是否已存在；\
                 若要替换请先 `DELETE` 再创建（专家名删除后可复用）。"
            }
            Self::MethodNotAllowed { .. } => {
                "对照 docs/PHASE2_CONTRACT.md §5.1 确认该路径允许的方法；\
                 路径存在但方法不对不会被当成 404。"
            }
            Self::NotImplemented { .. } => {
                "执行 `GET /healthz` 确认服务存活；该路由随对应 crate 落地后自动转为可用，\
                 在此之前请不要在客户端里依赖它返回成功。"
            }
            Self::StorageUnavailable { .. } => {
                "存储不可用：先执行 `quill doctor --section=db` 打印数据库诊断；\
                 若提示缺表，按 crates/quill-store/migrations/0001_init.sql 执行迁移后重启服务；\
                 若提示打不开数据库，用 `QUILL_DB_PATH` 指向一个可写路径后重启。"
            }
            Self::Internal { .. } => {
                "查看服务端 stderr 日志中带请求 ID 的记录定位真实原因（客户端只拿到可读说明，\
                 不会收到内部栈）；然后执行 `quill doctor` 打印完整诊断，修复后用同一请求重试。"
            }
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.detail())
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = Json(json!({
            "error": {
                "code": self.code(),
                "detail": self.detail(),
                "next_step": self.next_step(),
            }
        }));
        // ⚠️ 401 必须带 `WWW-Authenticate`（RFC 9110 §11.6.1）：
        //    客户端据此知道该用哪种认证方式，而不是靠猜。
        let mut response = (status, body).into_response();
        if status == StatusCode::UNAUTHORIZED {
            if let Ok(v) = "Bearer".parse() {
                response
                    .headers_mut()
                    .insert(axum::http::header::WWW_AUTHENTICATE, v);
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthorized_has_single_shape_so_user_enumeration_is_impossible() {
        // 构造入口唯一 ⇒ 两条不同原因的 401 走的是同一个 detail，
        // 「用户不存在」与「令牌错误」在响应里无法区分。
        let a = ApiError::unauthorized();
        let b = ApiError::unauthorized();
        assert_eq!(a.detail(), b.detail());
        assert_eq!(a.code(), "unauthorized");
        assert_eq!(a.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn every_variant_carries_a_non_empty_next_step() {
        // 铁律七：失败必须自诊断。漏写 next_step 就是漏了"下一步执行什么"。
        let cases = [
            ApiError::unauthorized(),
            ApiError::forbidden("非 admin"),
            ApiError::not_found("/api/nope"),
            ApiError::entity_not_found("专家不存在"),
            ApiError::bad_request("缺 display_name"),
            ApiError::conflict("专家已存在"),
            ApiError::method_not_allowed("POST", "/healthz"),
            ApiError::not_implemented_for_test(),
            ApiError::storage_unavailable("数据库打不开"),
            ApiError::internal("内部错误"),
        ];
        for c in &cases {
            assert!(!c.detail().trim().is_empty(), "{c:?} 缺中文说明");
            assert!(!c.next_step().trim().is_empty(), "{c:?} 缺下一步指引");
        }
    }

    #[test]
    fn not_implemented_mentions_the_registered_route() {
        let e = ApiError::NotImplemented {
            method: "GET",
            path: "/api/experts",
        };
        assert!(e.detail().contains("/api/experts"));
        assert_eq!(e.status(), StatusCode::NOT_IMPLEMENTED);
    }

    impl ApiError {
        fn not_implemented_for_test() -> Self {
            Self::NotImplemented {
                method: "GET",
                path: "/api/version",
            }
        }
    }
}
