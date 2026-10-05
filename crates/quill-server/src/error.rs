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

    Forbidden {
        detail: String,
    },

    NotFound {
        path: String,
    },

    EntityNotFound {
        detail: String,
    },

    BadRequest {
        detail: String,
    },

    Conflict {
        detail: String,
    },

    MethodNotAllowed {
        method: String,
        path: String,
    },

    NotImplemented {
        method: &'static str,
        path: &'static str,
    },

    StorageUnavailable {
        detail: String,
    },

    ProviderUnavailable {
        detail: String,
    },

    /// 请求过于频繁。登录端点用它挡住「反复 POST 把 PBKDF2 的 CPU 打满」。
    TooManyRequests {
        detail: String,
        /// 建议客户端等待的秒数。会原样写进 `Retry-After` 响应头。
        retry_after_secs: u64,
    },

    Internal {
        detail: String,
    },
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

    pub fn service_unavailable(detail: impl Into<String>) -> Self {
        Self::ProviderUnavailable {
            detail: detail.into(),
        }
    }

    pub fn storage_unavailable_detail(detail: impl Into<String>) -> Self {
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

    pub fn too_many_requests(detail: impl Into<String>, retry_after_secs: u64) -> Self {
        Self::TooManyRequests {
            detail: detail.into(),
            // 0 秒会让客户端立刻重试，等于没限流；至少给 1 秒。
            retry_after_secs: retry_after_secs.max(1),
        }
    }

    /// 客户端该等多久再重试。非 429 一律为 `None`。
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::TooManyRequests { retry_after_secs, .. } => Some(*retry_after_secs),
            _ => None,
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
            Self::StorageUnavailable { .. } | Self::ProviderUnavailable { .. } => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
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
            Self::ProviderUnavailable { .. } => "provider_unavailable",
            Self::TooManyRequests { .. } => "too_many_requests",
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
            Self::NotImplemented { method, path } => {
                format!("路由 {method} {path} 已登记，但能力尚未实现")
            }
            Self::StorageUnavailable { detail } => detail.clone(),
            Self::ProviderUnavailable { detail } => detail.clone(),
            Self::TooManyRequests { detail, .. } => detail.clone(),
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
                "确认路径拼写是否与调用方约定一致；\
                 本实例未登记的路径一律 404（不会静默兜底成 200）。"
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
                "确认该路径允许的方法；\
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
            Self::ProviderUnavailable { .. } => {
                "模型服务不可用：先执行 `curl $QUILL_LLM_BASE_URL/models` 确认端点活着；\
                 本地模型请先启动 llama-server，再用 `QUILL_LLM_BASE_URL` / `QUILL_LLM_MODEL` \
                 指向正确的地址与模型名后重启 quill-server。"
            }
            Self::Internal { .. } => {
                "查看服务端 stderr 日志中带请求 ID 的记录定位真实原因（客户端只拿到可读说明，\
                 不会收到内部栈）；然后执行 `quill doctor` 打印完整诊断，修复后用同一请求重试。"
            }
            Self::TooManyRequests { .. } => {
                "登录尝试过于密集，已被临时挡下。等响应头 `Retry-After` 指定的秒数过后再重试；\
                 若是脚本在轮询登录，请把间隔放宽到 1 秒以上，不要靠并发硬撞。\
                 本实例的策略是「同一用户名 + 同一来源 IP 在滑动窗口内累计失败 N 次即锁定」，\
                 成功登录会清空该窗口，所以正常用户不会被误伤。"
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
        if let Some(secs) = self.retry_after_secs() {
            // Retry-After 的合法取值是秒数或 HTTP 日期，这里固定用秒数。
            if let Ok(v) = secs.to_string().parse() {
                response
                    .headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, v);
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
    fn a_dead_model_endpoint_does_not_send_the_operator_to_the_database() {
        let llm = ApiError::service_unavailable("本实例没有可用的模型服务");
        let db = ApiError::storage_unavailable("数据库打不开");

        assert_eq!(llm.code(), "provider_unavailable");
        assert_eq!(db.code(), "storage_unavailable");
        assert_ne!(llm.code(), db.code(), "两类故障必须是不同的错误码");

        assert!(
            llm.next_step().contains("QUILL_LLM_BASE_URL"),
            "模型故障的修复建议要指向模型环境变量：{}",
            llm.next_step()
        );
        assert!(
            llm.next_step() != db.next_step(),
            "模型故障与存储故障的修复建议必须不同，否则会把人引去查数据库"
        );
        assert!(
            db.next_step().contains("--section=db"),
            "存储故障仍要指向 db 段：{}",
            db.next_step()
        );
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
            ApiError::too_many_requests("登录太频繁", 30),
            ApiError::internal("内部错误"),
        ];
        for c in &cases {
            assert!(!c.detail().trim().is_empty(), "{c:?} 缺中文说明");
            assert!(!c.next_step().trim().is_empty(), "{c:?} 缺下一步指引");
        }
    }

    #[test]
    fn too_many_requests_is_429_and_says_how_long_to_wait() {
        let e = ApiError::too_many_requests("登录尝试过于频繁", 42);
        assert_eq!(e.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(e.code(), "too_many_requests");
        assert_eq!(e.retry_after_secs(), Some(42));
    }

    #[test]
    fn a_zero_second_retry_hint_is_clamped_so_clients_do_not_hot_loop() {
        let e = ApiError::too_many_requests("登录尝试过于频繁", 0);
        assert_eq!(
            e.retry_after_secs(),
            Some(1),
            "Retry-After: 0 等于没限流，客户端会立刻再撞一次"
        );
    }

    #[test]
    fn retry_after_is_only_set_for_429() {
        assert_eq!(ApiError::unauthorized().retry_after_secs(), None);
        assert_eq!(ApiError::internal("x").retry_after_secs(), None);
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
