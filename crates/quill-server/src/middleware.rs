//! 统一兜底层：请求 ID + panic 捕获。
//!
//! # 契约 §5.1「通用中间件」
//!
//! 原文：「认证（除 `/api/auth/login`）、**请求 ID**、审计日志、**panic 捕获**」。
//! 本模块实现请求 ID 与 panic 捕获两项（审计日志属 V1.1，本文件只留接缝）。
//!
//! # panic 为什么必须兜住
//!
//! 契约 §一规则 8：**禁 `panic = "abort"`**（上游 8 处 `catch_unwind` 依赖 unwind）。
//! 但"依赖 unwind"只保证**能**捕获，不保证**被**捕获 ——
//! handler 里一个 `unwrap()` 会顺着 tower 服务栈冒到连接任务，
//! 干掉同一 worker 上**其它**在途请求。
//!
//! 因此这里把每个请求的 future 包进 [`CatchUnwind`]：panic → 500 + 中文可读信息，
//! **且不把 panic payload 泄漏给客户端**。
//!
//! # 为什么自己写 `CatchUnwind` 而不用 `futures_util::FutureExt::catch_unwind`
//!
//! `futures-util` **不在** `Cargo.lock` 的直接依赖里，新增依赖须主理人裁决
//! （`docs/DECISIONS.md` 的既定口径）。`CatchUnwind` 只有 20 行，
//! 为它引入一个宏依赖不值得 —— 自己写也让"我们在哪一层兜住 panic"完全显式。
//!
//! ⚠️ `catch_unwind` 要求被捕获的闭包 `UnwindSafe`；请求 future 不是，
//!    故用 `AssertUnwindSafe` 包裹。这是有意识的取舍：兜底的唯一目的是
//!    「不让一个请求的 panic 杀掉整个连接」，不试图证明 future 内部
//!    无跨 await 的不变式（那是 handler 自己的责任）。

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Response};
use tower::{Layer, Service};

use crate::error::ApiError;

/// 请求 ID 响应头名。
pub const REQUEST_ID_HEADER: &str = "x-quill-request-id";

/// 兜底层：**任何**路由（包括未来新增的）都经过它。
///
/// ⚠️ 刻意做成 `tower::Layer` 而不是 `axum::middleware::from_fn`：
/// `from_fn` 拿不到 handler future 的 poll 层，panic 发生时已在它的栈外，
/// 捕获不到。
#[derive(Clone, Copy, Debug, Default)]
pub struct GuardLayer;

impl<S> Layer<S> for GuardLayer {
    type Service = Guard<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Guard { inner }
    }
}

/// [`GuardLayer`] 的服务体。
#[derive(Clone, Copy, Debug)]
pub struct Guard<S> {
    inner: S,
}

/// 被捕获的 panic 载荷。
type PanicPayload = Box<dyn Any + Send>;

impl<S> Service<Request<Body>> for Guard<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        Box::pin(async move {
            // ⚠️ tower 契约：调用 `call` 之前必须先 `poll_ready` 到 Ready。
            //    用 `poll_fn` 而不是凭空造 Context —— 造一个 noop waker 会让
            //    内层服务的 waker 登记到那个空转 waker 上，真实唤醒就丢了。
            std::future::poll_fn(|cx| inner.poll_ready(cx)).await?;
            let request_id = new_request_id();
            let tagged = attach_request_id(req, &request_id);
            match CatchUnwind::new(inner.call(tagged)).await {
                Ok(Ok(resp)) => Ok(with_request_id(resp, &request_id)),
                Ok(Err(_)) => Ok(with_request_id(internal_response(&request_id), &request_id)),
                Err(payload) => {
                    // ⚠️ payload 只进 stderr，客户端拿到的是固定中文说明 + 请求 ID。
                    eprintln!(
                        "[panic] 请求 {request_id} 内部 panic：{}",
                        detail_of(&payload)
                    );
                    Ok(with_request_id(internal_response(&request_id), &request_id))
                }
            }
        })
    }
}

/// 500 响应（中文说明 + 请求 ID，绝不含内部细节）。
fn internal_response(request_id: &str) -> Response<Body> {
    ApiError::internal(format!(
        "服务端内部错误（请求 ID {request_id}）。\
         真实原因已写入服务端日志，未泄漏到响应里。"
    ))
    .into_response()
}

fn attach_request_id(mut req: Request<Body>, request_id: &str) -> Request<Body> {
    if let Ok(v) = HeaderValue::from_str(request_id) {
        req.headers_mut().insert(REQUEST_ID_HEADER, v);
    }
    req
}

fn with_request_id(mut resp: Response<Body>, request_id: &str) -> Response<Body> {
    if let Ok(v) = HeaderValue::from_str(request_id) {
        resp.headers_mut().insert(REQUEST_ID_HEADER, v);
    }
    resp
}

/// 请求 ID 生成器。
///
/// ⚠️ **不是** UUID：`uuid` crate 不在 `Cargo.lock`（新增依赖须主理人裁决）。
/// 这里用「纳秒时间戳 + 进程内单调计数」拼一个**仅用于日志关联**的字符串。
/// 它不需要密码学强度 —— 用途是让用户能把一条 500 报障和一行日志对上。
fn new_request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("r{nanos:x}-{n:x}")
}

/// 提取 panic 信息（**仅**服务端日志用）。
fn detail_of(payload: &PanicPayload) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<非字符串 panic payload>".to_string()
    }
}

/// 在 poll 层捕获 panic 的 future。
///
/// 状态机：`inner` 为 `None` 表示 future 已完成（`output` 已有值），
/// 因此重复 poll 一个已完成的 future 也不会 panic。
///
/// ⚠️ `output` 存 `Box<...>` 而不是直接存 `F::Output`：
/// `Box<T>` 恒为 `Unpin`，而 `F::Output`（如 `Response<Body>`）不是。
/// 若直接存，本结构体的 `Unpin` 就得依赖 `F::Output: Unpin`，
/// 于是 `self.get_mut()` 在泛型场景下编译不过（实测 E0277）。
struct CatchUnwind<F: Future> {
    inner: Option<Pin<Box<F>>>,
    output: Option<Box<Result<F::Output, PanicPayload>>>,
}

impl<F: Future> CatchUnwind<F> {
    fn new(inner: F) -> Self {
        Self {
            inner: Some(Box::pin(inner)),
            output: None,
        }
    }
}

impl<F: Future> Future for CatchUnwind<F> {
    type Output = Result<F::Output, PanicPayload>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `inner` 是 `Pin<Box<..>>`、`output` 是 `Box<Option<..>>` → 本结构体 Unpin。
        let this = self.get_mut();
        if let Some(out) = this.output.take() {
            return Poll::Ready(*out);
        }
        let Some(mut inner) = this.inner.take() else {
            // 已完成但 output 已被取走（违反 Future 契约的重复 poll）。
            // 不 panic：返回 Ready 让调用方自行处理。
            return Poll::Ready(Err(Box::new("future 已被重复 poll")));
        };
        let polled = std::panic::catch_unwind(AssertUnwindSafe(|| inner.as_mut().poll(cx)));
        match polled {
            Ok(Poll::Ready(v)) => Poll::Ready(Ok(v)),
            Ok(Poll::Pending) => {
                // ⚠️ 必须放回去：pending 不代表 future 结束，waker 已由 inner 登记。
                this.inner = Some(inner);
                Poll::Pending
            }
            Err(payload) => Poll::Ready(Err(payload)),
        }
    }
}

/// 请求 ID 头名的强类型别名。
///
/// ⚠️ 刻意**不在 const 上下文里** `HeaderName::from_static` 做校验：
/// `HeaderName` 有析构函数，const 里的临时值无法 drop（实测 E0493）。
/// 改为在测试里断言头名合法 —— 编译期报错比运行期更好，但这里要能通过。
pub const REQUEST_ID_HEADER_NAME: &str = REQUEST_ID_HEADER;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_header_name_is_a_legal_http_header_name() {
        // 头名非法会让 `HeaderValue` 插入失败 → 静默丢失请求 ID（不可诊断的失败）。
        let n = axum::http::HeaderName::from_static(REQUEST_ID_HEADER_NAME);
        assert_eq!(n.as_str(), "x-quill-request-id");
    }

    #[test]
    fn request_ids_are_unique_and_non_empty() {
        let a = new_request_id();
        let b = new_request_id();
        assert_ne!(a, b, "请求 ID 必须互不相同，否则日志无法关联");
        assert!(a.starts_with('r'));
    }

    #[test]
    fn panic_detail_handles_both_payload_shapes_and_the_unknown_one() {
        // ⚠️ 必须显式标注为 `PanicPayload`（= `Box<dyn Any + Send>`）：
        //    `Box::new(s)` 推的是 `Box<&str>`，**不会**自动协变到 trait object，
        //    真实运行时的 panic 载荷才是 trait object（实测 E0308）。
        let s: PanicPayload = Box::new("静态消息");
        assert_eq!(detail_of(&s), "静态消息");
        let owned: PanicPayload = Box::new(String::from("拥有所有权的消息"));
        assert_eq!(detail_of(&owned), "拥有所有权的消息");
        let opaque: PanicPayload = Box::new(42u8);
        assert!(detail_of(&opaque).contains("非字符串"));
    }

    #[tokio::test]
    async fn catch_unwind_converts_panic_into_err_payload() {
        let fut = CatchUnwind::new(async { panic!("内部细节不该出去") });
        let err = fut.await.expect_err("panic 必须变成 Err");
        assert!(detail_of(&err).contains("内部细节"));
    }

    #[tokio::test]
    async fn catch_unwind_passes_through_normal_results() {
        let v = CatchUnwind::new(async { 41u8 + 1 })
            .await
            .expect("正常结果不应被捕获");
        assert_eq!(v, 42);
    }
}
