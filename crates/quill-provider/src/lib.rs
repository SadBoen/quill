//! LLM 供应商抽象。
//!
//! 这一层只做三件事：把请求/响应映射到 OpenAI 兼容线格式、把流式 SSE 解成增量、
//! 以及把失败分类成「下一步该做什么」。它不关心编排、存储或界面。

mod error;
mod local;
mod openai;
mod provider;
mod pump;
mod sse;
mod types;
mod wire;

pub use error::{excerpt, redact_url, ProviderError, DOCTOR_CMD, LOCAL_MODELS_PROBE, MAX_BODY_CHARS};
pub use local::{
    llama_cpp, BASE_ENV, LLAMA_CPP_BASE_URL, LLAMA_CPP_DEFAULT_MODEL, MODEL_ENV,
};
pub use openai::{
    classify_status, classify_transport, split_utf8, step_event, OpenAiCompatible, Step,
    DEFAULT_TIMEOUT,
};
pub use provider::{BoxFuture, Provider, ProviderStream, SharedProvider};
pub use pump::pump_stream;
pub use sse::{SseDecoder, SseEvent, DONE_SENTINEL};
pub use types::{
    ChatRequest, ChatResponse, FinishReason, Message, MessageContent, ModelInfo, Role, StreamDelta,
    StreamSummary, TokenUsage, ToolCall, ToolSpec,
};
pub use wire::{
    build_request_body, message_to_wire, parse_chat_response, parse_models_list, parse_stream_chunk,
    parse_tool_calls, tool_call_from_parts, tool_spec_to_wire, usage_from_value, ChoiceDelta,
    FunctionDelta, StreamChoice, StreamChunk, ToolCallAssembler, ToolCallDelta,
};
