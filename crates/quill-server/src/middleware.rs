
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

pub const REQUEST_ID_HEADER: &str = "x-quill-request-id";

#[derive(Clone, Copy, Debug, Default)]
pub struct GuardLayer;

impl<S> Layer<S> for GuardLayer {
    type Service = Guard<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Guard { inner }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Guard<S> {
    inner: S,
}

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

            std::future::poll_fn(|cx| inner.poll_ready(cx)).await?;
            let request_id = new_request_id();
            let tagged = attach_request_id(req, &request_id);
            match CatchUnwind::new(inner.call(tagged)).await {
                Ok(Ok(resp)) => Ok(with_request_id(resp, &request_id)),
                Ok(Err(_)) => Ok(with_request_id(internal_response(&request_id), &request_id)),
                Err(payload) => {

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

fn detail_of(payload: &PanicPayload) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<非字符串 panic payload>".to_string()
    }
}

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

        let this = self.get_mut();
        if let Some(out) = this.output.take() {
            return Poll::Ready(*out);
        }
        let Some(mut inner) = this.inner.take() else {

            return Poll::Ready(Err(Box::new("future 已被重复 poll")));
        };
        let polled = std::panic::catch_unwind(AssertUnwindSafe(|| inner.as_mut().poll(cx)));
        match polled {
            Ok(Poll::Ready(v)) => Poll::Ready(Ok(v)),
            Ok(Poll::Pending) => {

                this.inner = Some(inner);
                Poll::Pending
            }
            Err(payload) => Poll::Ready(Err(payload)),
        }
    }
}

pub const REQUEST_ID_HEADER_NAME: &str = REQUEST_ID_HEADER;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_header_name_is_a_legal_http_header_name() {

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
