//! 把一条 `ProviderStream` 边读边攒成一条 `ChatResponse`。
//!
//! 为什么不写在 `openai.rs` 里：`sse_response_to_stream` 只负责「解 wire 格式」，
//! 而「把增量累积成一条完整回复」是**协议无关**的 —— 任何 `Provider` 实现
//! （包括本地的 llama.cpp）返回的流都要走同一段。放进协议实现里，
//! 下一个 provider 就得复制一份，累积口径迟早分叉。
//!
//! `on_delta` 让上层在收到每个增量的**当场**就能往外发（HTTP SSE 就是这么用的）。
//! 它是同步回调：增量必须立刻让出去，攒完再发就退化成一次性响应了。

use futures_util::StreamExt;

use crate::error::ProviderError;
use crate::provider::ProviderStream;
use crate::types::{ChatResponse, StreamDelta, StreamSummary, ToolCall};

/// 读干一条流，同时把每个增量交给 `on_delta`，最后交回一条完整回复。
///
/// `model_fallback` 只在上游没在流里报 `model` 时用 —— `ChatResponse.model`
/// 是非可选的字符串，而流式分片**不一定**带模型名（多数实现只有 usage 帧带）。
/// 宁可退回请求时用的模型名，也不要交一条空模型名的回复给界面。
///
/// 回调类型带 `Send`：调用方（quill-server）要把这个 future 交给后台任务，
/// 而 `&mut dyn FnMut` 不是 `Send`，这个收窄会一路传染到 Axum 的 handler 上，
/// 最后只报一句没头没脑的「Handler 没实现」。
pub async fn pump_stream(
    mut stream: ProviderStream,
    model_fallback: &str,
    on_delta: &mut (dyn FnMut(StreamDelta) + Send),
) -> Result<ChatResponse, ProviderError> {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut summary = StreamSummary::default();

    while let Some(item) = stream.next().await {
        let delta = item?;
        match &delta {
            StreamDelta::Text(t) => text.push_str(t),
            StreamDelta::Reasoning(r) => reasoning.push_str(r),
            StreamDelta::ToolCall(c) => tool_calls.push(c.clone()),
            StreamDelta::Done(s) => summary = s.clone(),
        }
        on_delta(delta);
    }

    Ok(ChatResponse {
        id: summary.id,
        model: summary.model.unwrap_or_else(|| model_fallback.to_string()),
        text,
        reasoning,
        tool_calls,
        finish_reason: summary.finish_reason,
        usage: summary.usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FinishReason, TokenUsage};
    use futures_util::stream;

    fn call(name: &str) -> ToolCall {
        ToolCall {
            id: "call_1".into(),
            name: name.into(),
            arguments: serde_json::json!({"q": name}),
        }
    }

    fn stream_of(items: Vec<Result<StreamDelta, ProviderError>>) -> ProviderStream {
        Box::pin(stream::iter(items))
    }

    #[tokio::test]
    async fn deltas_are_accumulated_into_one_reply_and_forwarded_in_order() {
        let mut seen: Vec<StreamDelta> = Vec::new();
        let mut sink = |d: StreamDelta| seen.push(d);
        let s = stream_of(vec![
            Ok(StreamDelta::Reasoning("先想".into())),
            Ok(StreamDelta::Text("你好".into())),
            Ok(StreamDelta::Text("，世界".into())),
            Ok(StreamDelta::Done(StreamSummary {
                finish_reason: Some(FinishReason::Stop),
                usage: TokenUsage::new(Some(11), Some(7)),
                ..Default::default()
            })),
        ]);

        let reply = pump_stream(s, "fallback-model", &mut sink)
            .await
            .expect("流应当读完");

        assert_eq!(reply.text, "你好，世界");
        assert_eq!(reply.reasoning, "先想");
        assert_eq!(reply.finish_reason, Some(FinishReason::Stop));
        assert_eq!(reply.usage.input, Some(11));
        assert_eq!(reply.usage.output, Some(7));
        assert_eq!(
            seen,
            vec![
                StreamDelta::Reasoning("先想".into()),
                StreamDelta::Text("你好".into()),
                StreamDelta::Text("，世界".into()),
                StreamDelta::Done(StreamSummary {
                    finish_reason: Some(FinishReason::Stop),
                    usage: TokenUsage::new(Some(11), Some(7)),
                    ..Default::default()
                }),
            ],
            "上层必须按到达顺序拿到每一个增量"
        );
    }

    #[tokio::test]
    async fn tool_calls_are_collected_and_the_reply_reports_them() {
        let s = stream_of(vec![
            Ok(StreamDelta::ToolCall(call("afrexai-qa-test-plan"))),
            Ok(StreamDelta::ToolCall(call("list_experts"))),
            Ok(StreamDelta::Done(StreamSummary {
                finish_reason: Some(FinishReason::ToolCalls),
                ..Default::default()
            })),
        ]);

        let reply = pump_stream(s, "m", &mut |_| {}).await.expect("流应当读完");

        assert_eq!(
            reply
                .tool_calls
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["afrexai-qa-test-plan", "list_experts"],
            "工具调用不能因为流式就漏掉 —— 它们要回灌给模型继续下一轮"
        );
        assert_eq!(reply.finish_reason, Some(FinishReason::ToolCalls));
    }

    #[tokio::test]
    async fn a_stream_with_no_done_frame_still_yields_a_reply_without_invented_numbers() {
        let s = stream_of(vec![Ok(StreamDelta::Text("半句".into()))]);
        let reply = pump_stream(s, "m", &mut |_| {}).await.expect("流应当读完");

        assert_eq!(reply.text, "半句");
        assert_eq!(
            reply.usage,
            TokenUsage::default(),
            "上游没给 usage 就必须是「没报」，不能补 0 冒充"
        );
    }

    #[tokio::test]
    async fn the_model_falls_back_to_the_requested_one_when_the_stream_omits_it() {
        let s = stream_of(vec![Ok(StreamDelta::Done(StreamSummary::default()))]);
        let reply = pump_stream(s, "qwen3-4b", &mut |_| {})
            .await
            .expect("流应当读完");
        assert_eq!(reply.model, "qwen3-4b");
    }

    #[tokio::test]
    async fn a_mid_stream_failure_is_reported_rather_than_silently_truncated() {
        let s = stream_of(vec![
            Ok(StreamDelta::Text("前半".into())),
            Err(ProviderError::Timeout {
                detail: "断了".into(),
            }),
        ]);

        let err = pump_stream(s, "m", &mut |_| {})
            .await
            .expect_err("流中断必须当成失败返回");
        assert!(
            matches!(err, ProviderError::Timeout { .. }),
            "原样透出分类后的错误：{err:?}"
        );
    }
}
