use std::fmt;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// 「模型一直调工具、不给正文」时的建议。
///
/// 措辞刻意**与 detail 里那句「换个更直接的问法」说同一件事** ——
/// ISSUE-027 的病根就是两句下一步打架：detail 说「换问法」，
/// 结构化 `next_step` 却在教用户去重启一个正在正常应答的服务。
/// 这里**绝不能**出现「确认端点活着 / 启动 llama-server」：
/// 轮次用尽的时候，模型每一轮都回过话，服务好得很。
pub const ADVICE_TOOL_LOOP_EXHAUSTED: &str =
    "模型每一轮都回话了，服务是好的 —— 只是它一直只调工具、一直没给出正文。\
     下一步：换个更直接的问法（把要什么一次说清楚），\
     或先停用这一轮里挂着的技能/工具：它们可能让模型觉得'还得再查一下'。\
     已执行的工具见上面那段。细节跑 `quill doctor`。";

#[derive(Debug, Clone)]
pub enum ApiError {
    Unauthorized {
        detail: &'static str,
    },

    Forbidden {
        detail: String,
    },

    /// 模型服务**回话了**但拒绝了这次请求。与 `ProviderUnavailable` 分开的原因
    /// 见 `provider_rejected` 的文档。
    ProviderRejected {
        detail: String,
        advice: &'static str,
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
        /// 「下一步」由调用方传，因为它随**是哪个资源**而变：专家名删除后
        /// 可以复用同名，备份目录则是**非空就拒写** —— 套用同一句会让用户
        /// 去 `DELETE` 一个备份目录，而那正是最不该被建议的动作。
        advice: &'static str,
    },

    /// 请求本身读得懂，内容却不成立 —— 例如备份的清单解析不了、摘要对不上。
    ///
    /// 单独一个 422 是为了和 `Conflict` 分开：界面按状态码给不同的「下一步」，
    /// 两件事混在一个 409 里，用户就会被告知一个与真实原因无关的做法。
    Unprocessable {
        detail: String,
        advice: &'static str,
    },

    MethodNotAllowed {
        method: String,
        path: String,
    },

    NotImplemented {
        method: &'static str,
        path: &'static str,
        /// 「为什么没做、接下来怎么办」由调用方给。
        ///
        /// 501 最容易变成一句没有信息的话：「该路由已登记，但能力尚未实现」。
        /// 用户读到它只能猜。可实际情况往往是**有意不做**——
        /// 例如 `POST /api/users` 不建账号，是 `bootstrap.rs` 里写死的部署口径
        /// （账号只来自环境变量配置），而不是没来得及写。把这句话按路由分开，
        /// 501 才真的在告诉人下一步。
        advice: &'static str,
    },

    StorageUnavailable {
        detail: String,
    },

    ProviderUnavailable {
        detail: String,
    },

    /// **外部服务**（不是模型）没给出可用结果。
    ///
    /// 现在只有技能市场（SkillHub）在用。它与 `ProviderUnavailable` 必须分开：
    /// 后者的「下一步」是「执行 `curl $QUILL_LLM_BASE_URL/models` 确认端点活着；
    /// 本地模型请先启动 llama-server」—— 把这句话挂在「技能市场连不上」上，
    /// 会把用户引到一台与这件事毫无关系的机器上去排查。见 ISSUE-054。
    UpstreamUnavailable {
        detail: String,
        /// 「下一步」由调用方写，因为它随**是哪个上游**而变
        /// （SkillHub 说检查 `QUILL_SKILLHUB_HOST`，别的上游说别的）。
        advice: &'static str,
    },

    /// 请求过于频繁。登录端点用它挡住「反复 POST 把 PBKDF2 的 CPU 打满」。
    TooManyRequests {
        detail: String,
        /// 建议客户端等待的秒数。会原样写进 `Retry-After` 响应头。
        retry_after_secs: u64,
    },

    /// **模型连上了、每轮都回话了，但一直不给出正文**（工具往返用尽）。
    ///
    /// 第三种处境：既不是 `ProviderUnavailable`（连不上），
    /// 也不是 `ProviderRejected`（回了一个错误）。这里**什么都没报错**，
    /// 模型只是一直在调工具、一直不收敛。
    ///
    /// 为什么要单列：这两个已有的建议在**这里**都不对——
    /// 「确认端点活着 / 启动 llama-server」是在教用户去查一台当场回过话的服务；
    /// 「回了一个错误状态码」更是凭空捏造了一个不存在的错误。
    /// 而 detail 里那句「换个更直接的问法」是对的，却被结构化的 `next_step`
    /// 盖住了，界面上于是出现两句互相打架的下一步。见 ISSUE-027。
    ///
    /// 实测（2026-10-06，`sb-3d-scan-calc` / `sb-citation-check`）：
    /// 模型连续 4 轮都带 tool_calls、没有正文，报成
    /// 「模型服务不可用」—— 而那 4 轮里它每次都回话了。
    ToolLoopExhausted {
        detail: String,
        advice: &'static str,
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

    /// 「资源已存在」。「下一步」由调用方按**是哪个资源**传进来 ——
    /// 不同资源的替换方式不一样（专家名删了能复用，备份目录不能删），
    /// 在这里给一句通用的话就是把用户引到错误的分支上。
    pub fn conflict(detail: impl Into<String>, advice: &'static str) -> Self {
        Self::Conflict {
            detail: detail.into(),
            advice,
        }
    }

    /// 「请求读得懂，内容不成立」（422）。同样由调用方给「下一步」。
    pub fn unprocessable(detail: impl Into<String>, advice: &'static str) -> Self {
        Self::Unprocessable {
            detail: detail.into(),
            advice,
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

    /// **外部上游**（如技能市场）没给出可用结果。
    ///
    /// 不要拿 [`Self::service_unavailable`] 顶替：那条的「下一步」谈的是
    /// 模型服务与 llama-server，挂在技能市场上会把用户带偏。
    pub fn upstream_unavailable(detail: impl Into<String>, advice: &'static str) -> Self {
        Self::UpstreamUnavailable {
            detail: detail.into(),
            advice,
        }
    }

    /// **模型服务活着、但它回了一个错误**（HTTP 4xx/5xx、响应不合法、上游拒绝）。
    ///
    /// 为什么要与 `ProviderUnavailable` 分开：`ProviderUnavailable` 的「下一步」是
    /// 「先 curl 一下确认端点活着 / 把 llama-server 起起来」—— 那句话在**连不上**
    /// 时是对的，在**连上了但被拒**时是错的：让用户去检查一个当场回过话的服务，
    /// 比不给建议更糟（他会以为服务没起，反复重启，最后仍然不通）。
    ///
    /// 证据（2026-10-06 实测）：模型上下文是 8192，装了 5 个 SKILL 之后请求
    /// 变成 8525 token，上游回 `exceed_context_size_error`。那时候界面给的
    /// 「下一步」是「先确认端点活着；本地模型请先启动 llama-server」——
    /// 而端点明明活着。见 ISSUE-018。
    ///
    /// `advice` 由调用方按 `ProviderError` 的种类挑好传进来，而不是在这里猜：
    /// `quill-provider` 已经分得清 `Unreachable`（没连上）与 `Status`（回了错）。
    pub fn provider_rejected(detail: impl Into<String>, advice: &'static str) -> Self {
        Self::ProviderRejected {
            detail: detail.into(),
            advice,
        }
    }

    /// 工具往返用尽、始终没有正文。见 `ToolLoopExhausted` 的说明。
    pub fn tool_loop_exhausted(detail: impl Into<String>) -> Self {
        Self::ToolLoopExhausted {
            detail: detail.into(),
            advice: ADVICE_TOOL_LOOP_EXHAUSTED,
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
            Self::Unprocessable { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::NotImplemented { .. } => StatusCode::NOT_IMPLEMENTED,
            Self::StorageUnavailable { .. } | Self::ProviderUnavailable { .. }
            | Self::UpstreamUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            // 与 `ProviderUnavailable` 同一个状态码：**请求确实没拿到结果**。
            // 分开的是「下一步」说什么，不是「算不算失败」。
            Self::ProviderRejected { .. } => StatusCode::SERVICE_UNAVAILABLE,
            // 同样是 503：请求确实没拿到正文。分开的仍然是「下一步」说什么。
            // 用 502 反而更贴切，但会改动对外状态码，**这一轮不做** ——
            // 先把最容易误导用户的那句改对，状态码另议。
            Self::ToolLoopExhausted { .. } => StatusCode::SERVICE_UNAVAILABLE,
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
            Self::Unprocessable { .. } => "unprocessable",
            Self::NotImplemented { .. } => "not_implemented",
            Self::StorageUnavailable { .. } => "storage_unavailable",
            Self::ProviderUnavailable { .. } => "provider_unavailable",
            Self::UpstreamUnavailable { .. } => "upstream_unavailable",
            Self::ProviderRejected { .. } => "provider_rejected",
            // 与上面两个 provider 码**必须不同**：连不上 / 被拒 / 不收敛
            // 是三件不同的事，界面要能分开说。见 ISSUE-027。
            Self::ToolLoopExhausted { .. } => "tool_loop_exhausted",
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
            Self::Conflict { detail, .. } => detail.clone(),
            Self::Unprocessable { detail, .. } => detail.clone(),
            Self::NotImplemented { method, path, .. } => {
                format!("路由 {method} {path} 已登记，但能力尚未实现")
            }
            Self::StorageUnavailable { detail } => detail.clone(),
            Self::ProviderUnavailable { detail } => detail.clone(),
            Self::UpstreamUnavailable { detail, .. } => detail.clone(),
            Self::ProviderRejected { detail, .. } => detail.clone(),
            Self::ToolLoopExhausted { detail, .. } => detail.clone(),
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
            Self::Conflict { advice, .. } => advice,
            Self::Unprocessable { advice, .. } => advice,
            Self::MethodNotAllowed { .. } => {
                "确认该路径允许的方法；\
                 路径存在但方法不对不会被当成 404。"
            }
            Self::NotImplemented { advice, .. } => advice,
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
            // 外部上游（技能市场等）：**不许**复用上面那句 ——
            // 让用户去 curl 模型端点、启动 llama-server，跟技能市场没有一点关系。
            Self::UpstreamUnavailable { advice, .. } => advice,
            // 「下一步」由调用方按 ProviderError 的种类挑好传进来。
            // 绝不能在这里给一句通用的「去确认端点活着」——模型明明回过话，
            // 让用户去检查一个活着的服务只会把他引到错误的分支上。
            Self::ProviderRejected { advice, .. } => advice,
            // 轮次用尽：不能说「服务不可用」，也不能说「回了一个错误」——
            // 模型每一轮都回过话，只是没收敛。见 ISSUE-027。
            Self::ToolLoopExhausted { advice, .. } => advice,
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

    /// ISSUE-018 的核心防线：**模型明明回过话，就不许让用户去重启它。**
    ///
    /// 两种错误的「下一步」必须不同，而且 `ProviderRejected` 那句里
    /// **不许**出现「确认端点活着」「启动 llama-server」——
    /// 那两句在「回过话但被拒」时是错的，会把用户引到「反复重启一个活着的服务」
    /// 这条死路上。
    #[test]
    fn a_live_model_that_refuses_must_not_be_reported_as_unreachable() {
        let dead = ApiError::service_unavailable("模型调用失败：连不上 LLM 服务");
        let alive = ApiError::provider_rejected(
            "模型调用失败：exceeds the available context size",
            crate::api_chat::ADVICE_REJECTED_BY_STATUS,
        );

        // 状态码可以一样（都没拿到结果），但错误码与「下一步」必须分得开。
        assert_eq!(dead.code(), "provider_unavailable");
        assert_eq!(alive.code(), "provider_rejected");
        assert_eq!(dead.status(), alive.status());

        for wrong in ["确认端点活着", "启动 llama-server", "llama-server"] {
            assert!(
                !alive.next_step().contains(wrong),
                "模型回过话了，「下一步」里不该出现「{wrong}」：{}",
                alive.next_step()
            );
        }
        // 真的没连上时，那句建议**必须**还在 —— 别把一个 bug 修成另一个 bug。
        assert!(dead.next_step().contains("确认端点活着"));
    }

    /// 两条分支都必须给出**可执行的**下一步，而且不能是同一句。
    ///
    /// **这里不断言字面量「下一步」三个字**：`next_step` 是结构化字段，
    /// 前端把它渲染成独立的一段（`Page.tsx` 的 `.form-error-next`），
    /// 标签由字段名承担。仓库里既有的 `next_step`（unauthorized、
    /// too_many_requests、internal）也都不含这三个字 ——
    /// 要断言它，就得先改掉那三条，属于无谓的措辞 churn。
    /// 真正要盯的是：**有内容、且能照着做**。
    #[test]
    fn both_provider_outcomes_hand_the_user_a_distinct_actionable_next_step() {
        let dead = ApiError::service_unavailable("模型调用失败：连不上 LLM 服务");
        let alive = ApiError::provider_rejected(
            "模型调用失败：exceeds the available context size",
            crate::api_chat::ADVICE_REJECTED_BY_STATUS,
        );

        assert_ne!(
            dead.next_step(),
            alive.next_step(),
            "连不上与回过话但被拒，下一步必须不同"
        );
        for e in [dead, alive] {
            let n = e.next_step();
            assert!(n.chars().count() > 20, "{:?} 的下一步太短，说不清怎么做：{n}", e.code());
            // 「能照着做」的最低要求：给得出一个可执行的东西（命令 / 键名 / 具体动作）。
            let actionable = n.contains('`') || n.contains("下一步") || n.contains("；");
            assert!(actionable, "{:?} 的下一步里没有可执行的动作：{n}", e.code());
        }
    }

    #[test]
    fn a_model_that_never_stops_calling_tools_is_not_an_unreachable_endpoint() {
        // ISSUE-027：模型连续 N 轮都回话了、只是没收敛。
        // 修之前走的是 `service_unavailable`，附上的建议是
        // 「确认端点活着 / 启动 llama-server」—— 让用户去查一台
        // 当场回过话的服务。
        let e = ApiError::tool_loop_exhausted("模型连续 4 轮都在请求调用工具，没有给出正文。");

        assert_eq!(e.code(), "tool_loop_exhausted");
        assert_ne!(
            e.code(),
            "provider_unavailable",
            "不收敛与连不上是两件事，错误码必须分开"
        );
        assert_ne!(e.code(), "provider_rejected", "这里什么都没报错，别编一个错误出来");

        for wrong in ["确认端点活着", "启动 llama-server", "llama-server", "回了一个错误状态码"] {
            assert!(
                !e.next_step().contains(wrong),
                "「下一步」里不该出现「{wrong}」：{}",
                e.next_step()
            );
        }
        // 建议必须与 detail 里那句「换个更直接的问法」说同一件事。
        assert!(
            e.next_step().contains("更直接"),
            "下一步要与 detail 里的处置方向一致：{}",
            e.next_step()
        );
        assert!(e.detail().contains("4 轮"), "detail 要如实说清是第几轮用尽的");
    }

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
            ApiError::conflict("专家已存在", "换个名字，或者先删掉原来那个。"),
            ApiError::unprocessable("备份清单解析不了", "这份备份不可用于还原。"),
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
            advice: "先看这里是什么能力。",
        };
        assert!(e.detail().contains("/api/experts"));
        assert_eq!(e.status(), StatusCode::NOT_IMPLEMENTED);
        // 「下一步」按路由给，不是一句通用安慰。
        assert_eq!(e.next_step(), "先看这里是什么能力。");
    }

    impl ApiError {
        fn not_implemented_for_test() -> Self {
            Self::NotImplemented {
                method: "GET",
                path: "/api/version",
                advice: "执行 `GET /healthz` 确认服务存活；该路由随对应 crate 落地后自动转为可用，\
                 在此之前请不要在客户端里依赖它返回成功。",
            }
        }
    }
}
