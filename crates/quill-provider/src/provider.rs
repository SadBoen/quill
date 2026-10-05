use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures_core::Stream;

use crate::error::ProviderError;
use crate::types::{ChatRequest, ChatResponse, ModelInfo, StreamDelta};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'a>>;

pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<StreamDelta, ProviderError>> + Send>>;

/// Object-safe so the server can hold `Arc<dyn Provider>` regardless of which
/// backend is configured. Futures are boxed instead of using `#[async_trait]` so
/// the trait stays a plain trait with no macro in the public signature.
pub trait Provider: Send + Sync + fmt::Debug {
    fn name(&self) -> &str;

    fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse>;

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream>;

    fn models<'a>(&'a self) -> BoxFuture<'a, Vec<ModelInfo>>;
}

pub type SharedProvider = Arc<dyn Provider>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use std::fmt;

    #[derive(Debug)]
    struct Stub;

    impl Provider for Stub {
        fn name(&self) -> &str {
            "stub"
        }

        fn chat<'a>(&'a self, request: &'a ChatRequest) -> BoxFuture<'a, ChatResponse> {
            Box::pin(async move {
                Ok(ChatResponse {
                    id: None,
                    model: request.model.clone(),
                    text: String::new(),
                    reasoning: String::new(),
                    tool_calls: Vec::new(),
                    finish_reason: None,
                    usage: Default::default(),
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> BoxFuture<'a, ProviderStream> {
            let empty: ProviderStream = Box::pin(futures_util::stream::empty());
            Box::pin(async move { Ok(empty) })
        }

        fn models<'a>(&'a self) -> BoxFuture<'a, Vec<ModelInfo>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    #[tokio::test]
    async fn a_provider_is_usable_behind_a_trait_object() {
        let shared: SharedProvider = Arc::new(Stub);
        let req = ChatRequest::new("stub-model", vec![Message::user("hi")]);
        let resp = shared.chat(&req).await.expect("桩实现应当成功");
        assert_eq!(resp.model, "stub-model");
        assert_eq!(shared.name(), "stub");

        let models = shared.models().await.expect("桩实现应当成功");
        assert!(models.is_empty());
    }

    #[test]
    fn a_trait_object_still_meets_send_sync_debug() {
        fn assert_bounds<T: Send + Sync + fmt::Debug + ?Sized>() {}
        assert_bounds::<dyn Provider>();
    }
}
