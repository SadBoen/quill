
use std::fmt;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, Clone)]
pub enum ApiError {

    Unauthorized {

        detail: &'static str,
    },

    Forbidden { detail: String },

    NotFound { path: String },

    EntityNotFound { detail: String },

    BadRequest { detail: String },

    Conflict { detail: String },

    MethodNotAllowed { method: String, path: String },

    NotImplemented {
        method: &'static str,
        path: &'static str,
    },

    StorageUnavailable { detail: String },

    Internal { detail: String },
}

impl ApiError {

    pub const fn unauthorized() -> Self {
        Self::Unauthorized {
            detail: "未认证或凭据无效",
        }
    }

    pub fn forbidden(detail: impl Into<String>) -> Self {
        Self::Forbidden {
            detail: detail.into(),
        }
    }

    pub fn not_found(path: impl Into<String>) -> Self {
        Self::NotFound { path: path.into() }
    }

    pub fn entity_not_found(detail: impl Into<String>) -> Self {
        Self::EntityNotFound {
            detail: detail.into(),
        }
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::BadRequest {
            detail: detail.into(),
        }
    }

    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::Conflict {
            detail: detail.into(),
        }
    }

    pub fn storage_unavailable(detail: impl Into<String>) -> Self {
        Self::StorageUnavailable {
            detail: detail.into(),
        }
    }

    pub fn method_not_allowed(method: impl Into<String>, path: impl Into<String>) -> Self {
        Self::MethodNotAllowed {
            method: method.into(),
            path: path.into(),
        }
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::Internal {
            detail: detail.into(),
        }
    }

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

        let a = ApiError::unauthorized();
        let b = ApiError::unauthorized();
        assert_eq!(a.detail(), b.detail());
        assert_eq!(a.code(), "unauthorized");
        assert_eq!(a.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn every_variant_carries_a_non_empty_next_step() {

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
