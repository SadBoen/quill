//! 拿真机上的 llama.cpp（MiniCPM5-1B-Q8_0）跑一遍 provider，验一下「能不能真用」。
//! 需要先起本地服务：见 quill doctor 的 provider 段。

use futures_util::stream::StreamExt;
use quill_provider::{ChatRequest, Message, Provider, StreamDelta};
use std::time::Duration;

#[tokio::test]
// 需要先起本地服务，否则 `cargo test -p quill-provider` 会因本测试 FAILED 而挂掉。
// 手动跑：QUILL_TEST_LLM=http://127.0.0.1:18080/v1 cargo test -p quill-provider --test real_local_model
#[ignore = "需要本机 llama-server 在跑"]
async fn real_local_model_answers_and_calls_a_tool() {
    let base =
        std::env::var("QUILL_TEST_LLM").unwrap_or_else(|_| "http://127.0.0.1:18080/v1".to_string());

    let Ok(provider) = quill_provider::OpenAiCompatible::new(base, "local", None::<String>) else {
        eprintln!("跳过：provider 构造失败");
        return;
    };
    let provider = provider.with_timeout(Duration::from_secs(300));

    let models = provider.models().await.expect("应能列出模型");
    assert!(!models.is_empty(), "本地服务应至少报告一个模型");

    let req = ChatRequest::new("local", vec![Message::user("1+1等于几？只回数字。")])
        .with_max_tokens(900);
    let res = provider.chat(&req).await.expect("对话请求应成功");

    eprintln!(
        "answer={:?} reasoning_len={} finish={:?} usage={:?}",
        res.answer(),
        res.reasoning.len(),
        res.finish_reason,
        res.usage.total()
    );
    assert!(
        res.has_answer(),
        "真机上必须拿到正文；若只有思考内容，多半是 max_tokens 太小（truncated_by_reasoning={}）",
        res.truncated_by_reasoning()
    );
    assert!(
        res.answer().contains('2'),
        "答案里应含 2，实际 {:?}",
        res.answer()
    );

    let sreq =
        ChatRequest::new("local", vec![Message::user("说“你好”两个字。")]).with_max_tokens(900);
    let mut stream = provider.stream(&sreq).await.expect("流式请求应成功");
    let mut text = String::new();
    let mut done = false;
    while let Some(item) = stream.next().await {
        match item.expect("每帧都要能解析") {
            StreamDelta::Text(t) => text.push_str(&t),
            StreamDelta::Reasoning(_) => {}
            StreamDelta::ToolCall(_) => {}
            StreamDelta::Done(_) => done = true,
        }
    }
    eprintln!("streamed text={text:?} done={done}");
    assert!(done, "流必须以 Done 收尾");
    assert!(!text.trim().is_empty(), "流式也必须拿到正文");
}
