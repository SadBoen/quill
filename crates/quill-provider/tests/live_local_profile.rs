//! 验证 `llama_cpp()` 这个本地 profile 入口真能连上真机，不重复 `real_local_model.rs`
//! 已经覆盖的 chat/stream 行为。
//!
//! 需要本机 `llama-server` 在跑：
//! `bash .wsl-llama.sh`（绑 127.0.0.1，WSL 侧还要设 `QUILL_LLAMA_BASE` 指向 Windows 地址）

use quill_provider::{llama_cpp, ChatRequest, Message, Provider, BASE_ENV};

#[tokio::test]
#[ignore = "需要本机 llama-server 在跑"]
async fn the_local_profile_answers_a_real_question() {
    let provider = llama_cpp(None).expect("默认本地 profile 必须能构造");
    eprintln!(
        "base={} model={} (env {BASE_ENV}={:?})",
        provider.base_url(),
        provider.model(),
        std::env::var(BASE_ENV).ok()
    );

    let models = provider.models().await.expect("应当能列出模型");
    eprintln!("models={models:?}");
    assert!(!models.is_empty(), "本地服务应当至少报告一个模型");

    let got = provider
        .chat(&ChatRequest::new("", vec![Message::user("用一句话回答：1+1等于几？")])
            .with_temperature(0.1)
            .with_max_tokens(900))
        .await
        .expect("真实推理必须成功");

    eprintln!("answer={:?} usage={:?}", got.answer(), got.usage.total());
    assert!(got.has_answer(), "真机上必须拿到正文");
    assert!(got.answer().contains('2'), "答案里应含 2，实际 {:?}", got.answer());
}
