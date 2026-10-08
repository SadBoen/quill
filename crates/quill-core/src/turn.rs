//! 对话循环：**一轮怎么推进**。
//!
//! 取历史 → 建请求 → 流式或一次性取回 → 有工具就执行并回灌 → 直到模型给出正文，
//! 或者工具预算用尽（再用一次不带工具的收尾调用逼它作答）。与 HTTP 无关：
//! 增量怎么出去由 [`TurnObserver`] 决定，SSE/JSON 的编码是壳的活。
//!
//! **出处（Q077 纪律：如实标注）**：这段循环是 quill 自研的
//! （`docs/KERNEL-ALIGNMENT.md §Q030` 已认定 `api_chat` 自研，不是抄 goose）。
//! goose 的对应物是 `vendor/goose/crates/goose/src/agents/agent.rs` 的
//! `reply` / `reply_impl`（一轮回复主循环）与 `agents/tool_execution.rs`
//! （工具往返执行）—— 移植时按那两处的推进顺序对照，但**没有逐行照抄**：
//! goose 的循环里混着会话存储、权限确认与状态机，在 quill 那些是别处的活。
//! 按 Q077 的口径，这里是「自创 + 对照上游」，**不是**「抄自上游」。
//!
//! 从壳搬进来的过程见 `docs/KERNEL-PORTS.md §4.1`（Q012）。

use std::time::Instant;

use quill_provider::{
    ChatRequest, ChatResponse, Message, ProviderError, SharedProvider, StreamDelta, TokenUsage,
    ToolCall, ToolSpec,
};
use serde_json::{json, Value};

use crate::llm::LlmConfig;
use crate::tools::{ToolRegistry, FINAL_ANSWER_MARKER, FINAL_ANSWER_PROMPT, MAX_TOOL_ROUNDS};

/// 一轮模型调用怎么发出去。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReplyMode {
    /// 等模型把整段回包给完再返回（老路由，语义不变）。
    Once,
    /// 增量一到就往外发；一个字都没吐出来就失败时退回 `Once`。
    Streamed,
}

/// 内核跑一轮需要的全部素材。
///
/// 除 `messages` 外都是**借用**：provider 句柄、工具表、配置都属于壳，
/// 内核只是用一趟。`messages` 由内核取走（循环要往里追加工具往返），
/// 壳在调用后不再需要它 —— 落库用的是 [`TurnOutcome`]，不是这份 msgs。
pub struct TurnInput<'a> {
    pub provider: &'a SharedProvider,
    pub llm_config: &'a LlmConfig,
    pub registry: &'a ToolRegistry,
    pub tools: &'a [ToolSpec],
    pub messages: Vec<Message>,
    pub mode: ReplyMode,
}

/// 一轮里「发生了什么」的出口（`docs/KERNEL-PORTS.md §4`）。
///
/// 一次性那条路不需要任何输出（壳给一个空实现），SSE 那条路把它换成往外发
/// 事件的实现。**循环本身只有一份。**
///
/// `Send` 是 supertrait 而非可选：SSE 那条路要把整个 future 交给后台任务，
/// 没有它编译不过。
pub trait TurnObserver: Send {
    /// 新一轮模型调用开始，`round` 从 0 起。
    fn round_start(&mut self, _round: usize) {}
    /// 正文增量。
    fn text(&mut self, _delta: &str) {}
    /// 思考增量。它不是答案，展示时必须与正文分开。
    fn reasoning(&mut self, _delta: &str) {}
    /// 模型请求调用工具。
    fn tool_call(&mut self, _call: &ToolCall) {}
    /// 工具执行完毕。**只带成败，不带结果正文**：全文在 `done` 事件的
    /// `tool_calls` 里，每个增量都抄一遍会把一帧撑到几千字符。
    fn tool_result(&mut self, _call: &ToolCall, _ok: bool) {}
    /// 这一轮的正文/思考会被后面的调用覆盖掉，现在丢弃。
    fn discard(&mut self, _round: usize) {}
}

/// 循环跑完、还没落库也还没拼响应的东西。
///
/// 字段 `pub`：壳要拿它拼 HTTP 响应体的每一格，并逐条落库。
pub struct TurnOutcome {
    pub reply: ChatResponse,
    pub usage: TokenUsage,
    pub tool_trace: Vec<Value>,
    pub rounds: usize,
    pub forced_final_answer: bool,
    pub turn_ms: i64,
}

/// 一轮对话（可能含多次模型调用）的 token 用量累加器。
///
/// ## 为什么需要它
///
/// 工具往返那段循环每轮都 `reply = provider.chat(&follow_up)`，**只把最后一轮的
/// `reply.usage` 存进 messages**。于是「这一轮」在统计条上只剩最后一次调用的数：
/// 实测 50 条真实任务里有 34 条以工具轮收场，而工具轮恰恰是入参最大的一类 ——
/// 用户拿这个数字判断「技能是不是把上下文撑爆了」，少报一半正好报在要命的地方。
///
/// ## 累加规则
///
/// 1. **只加真值**（`session_metrics::sum_reported` 的同一条规矩：「token 可加，
///    有一条真值即可」）。一条都没报就是 `None` —— 不能因为求和就凭空造出 0，
///    那会让界面显示「这次聊天一点没花 token」。
/// 2. **缓存两项不加进入参**。goose 口径里 `cache_read` / `cache_write` 是
///    `input` 的**子集**（见 migration 0008），把它们并进 `input` 会算出 >100% 的
///    命中率。求和是**逐项**求和，语义不变。
/// 3. `cache_read` 求和后**不允许超过 `input`**。越界只可能来自「某几轮报了
///    `input=0`/`None`、另一轮报了 `cache_read`」这种半真值组合，夹到 `input`
///    上，命中率就永远落在 100% 以内。诚实的上游每轮都满足子集关系，求和后
///    必然也满足，所以这条夹取**只**动得了异常上报。
/// 4. **单轮逐字不变**（`rounds <= 1` 时不夹）。不带工具的那一轮仍然原样存上游
///    报来的数：那是上游的口径，不在这一层替它改写。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TurnUsage {
    total: TokenUsage,
    /// 这一轮里一共调了几次模型。某条夹取规则只对「真的求和过」的情形生效，
    /// 靠它把单轮和多轮区分开。
    rounds: u32,
}

impl TurnUsage {
    /// 记入一次模型调用报上来的 usage。
    pub fn push(&mut self, usage: TokenUsage) {
        add_reported(&mut self.total.input, usage.input);
        add_reported(&mut self.total.output, usage.output);
        add_reported(&mut self.total.cache_read, usage.cache_read);
        add_reported(&mut self.total.cache_write, usage.cache_write);
        self.rounds = self.rounds.saturating_add(1);
    }

    /// 这一轮的总量。存档与响应 JSON 都用它，保证两边是同一份数。
    pub fn finish(self) -> TokenUsage {
        let mut total = self.total;
        if self.rounds > 1 {
            if let (Some(input), Some(read)) = (total.input, total.cache_read) {
                total.cache_read = Some(read.min(input));
            }
        }
        total
    }
}

/// 只在有真值时累加；`None` 保持 `None`（「没报」不等于「报了个 0」）。
fn add_reported(acc: &mut Option<u32>, value: Option<u32>) {
    if let Some(v) = value {
        *acc = Some(acc.unwrap_or(0).saturating_add(v));
    }
}

fn build_request(llm_config: &LlmConfig, tools: &[ToolSpec], msgs: &[Message]) -> ChatRequest {
    let request = crate::llm::build_request(llm_config, msgs.to_vec());
    if tools.is_empty() {
        request
    } else {
        request.with_tools(tools.to_vec())
    }
}

async fn one_round(
    provider: &SharedProvider,
    mode: ReplyMode,
    request: &ChatRequest,
    observer: &mut dyn TurnObserver,
) -> Result<ChatResponse, ProviderError> {
    match mode {
        ReplyMode::Once => provider.chat(request).await,
        ReplyMode::Streamed => streamed_round(provider, request, observer).await,
    }
}

/// 一次流式调用。**吐不出任何增量时的失败一律退回一次性调用。**
///
/// 退路是必需的：不少 OpenAI 兼容端点对 `stream: true` 的支持并不完整
/// （老版本 llama.cpp、部分网关直接回 400 或干脆回一整段 JSON）。
/// 流式是锦上添花，不能让它把「聊天」本身变成不可用。
async fn streamed_round(
    provider: &SharedProvider,
    request: &ChatRequest,
    observer: &mut dyn TurnObserver,
) -> Result<ChatResponse, ProviderError> {
    let mut emitted = 0usize;
    let result = match provider.stream(request).await {
        Ok(stream) => {
            let mut forward = |delta: StreamDelta| {
                emitted += 1;
                match &delta {
                    StreamDelta::Text(t) => observer.text(t),
                    StreamDelta::Reasoning(r) => observer.reasoning(r),
                    // 工具调用**不在这里让出去**：循环拿到整条回复之后自己发
                    // 一次。在这儿也发一遍就会每个工具调用出现两帧。
                    StreamDelta::ToolCall(_) => {}
                    // `Done` 只是收尾摘要，没有新内容可显示。
                    StreamDelta::Done(_) => {}
                }
            };
            quill_provider::pump_stream(stream, &request.model, &mut forward).await
        }
        Err(e) => Err(e),
    };
    match result {
        Ok(reply) => Ok(reply),
        Err(e) if emitted == 0 => {
            eprintln!("[chat] 流式一个字都没吐就失败（{e}），退回一次性调用");
            provider.chat(request).await
        }
        // 已经吐过字了：再补一次一次性调用会让同一段话在界面上出现两遍，
        // 那比报错更糟。老实报错。
        Err(e) => Err(e),
    }
}

/// 这一轮的可见内容会被后面的调用覆盖掉 —— 现在就告诉上层丢弃。
///
/// 不发这个信号，用户会看着一段已经显示出来的文字中途消失，以为是界面坏了。
fn discard_if_visible(reply: &ChatResponse, observer: &mut dyn TurnObserver, round: usize) {
    if !reply.answer().is_empty() || !reply.reasoning.trim().is_empty() {
        observer.discard(round);
    }
}

/// 工具往返循环。一次性与 SSE 两条路由跑的都是这一段。
pub async fn run_turn(
    input: TurnInput<'_>,
    observer: &mut dyn TurnObserver,
) -> Result<TurnOutcome, ProviderError> {
    let TurnInput {
        provider,
        llm_config,
        registry,
        tools,
        messages,
        mode,
    } = input;
    let started = Instant::now();
    // 这一轮**所有**模型调用的 usage 都记在这里，而不是只留最后一轮 ——
    // 工具往返的每一轮都真花了入参，漏掉它们统计条就只会报最后那次。
    let mut turn_usage = TurnUsage::default();
    let mut msgs = messages;

    observer.round_start(0);
    let request = build_request(llm_config, tools, &msgs);
    let mut reply = one_round(provider, mode, &request, observer).await?;
    turn_usage.push(reply.usage);

    let mut tool_trace: Vec<Value> = Vec::new();
    let mut rounds = 0usize;
    let mut round_no = 0usize;
    while !reply.tool_calls.is_empty() {
        if rounds >= MAX_TOOL_ROUNDS {
            eprintln!(
                "[chat] 工具往返达到上限 {} 轮，停止；未执行的调用：{:?}",
                MAX_TOOL_ROUNDS,
                reply.tool_calls.iter().map(|c| &c.name).collect::<Vec<_>>()
            );
            break;
        }
        rounds += 1;
        discard_if_visible(&reply, observer, round_no);

        let calls = reply.tool_calls.clone();
        msgs.push(Message::assistant_tool_calls(calls.clone()));
        for call in &calls {
            observer.tool_call(call);
            let result = registry.call(call);
            let ok = result.is_ok();
            match &result {
                Ok(text) => eprintln!("[chat] 工具 {} 执行成功（{} 字符）", call.name, text.len()),
                Err(detail) => eprintln!("[chat] 工具 {} 失败：{detail}", call.name),
            }
            // 渲染一次、存两处：回灌给模型的文本与写进响应的轨迹必须是同一份，
            // 否则界面上显示的与模型实际看到的会不一致。
            let rendered = registry.render_result(call, result);
            tool_trace.push(json!({
                "id": call.id,
                "name": call.name,
                "arguments": call.arguments,
                "ok": ok,
                "result": rendered,
            }));
            msgs.push(Message::tool_result(&call.id, &call.name, rendered));
            observer.tool_result(call, ok);
        }

        let follow_up = build_request(llm_config, tools, &msgs);
        round_no += 1;
        observer.round_start(round_no);
        reply = one_round(provider, mode, &follow_up, observer).await?;
        turn_usage.push(reply.usage);
    }

    // 轮次用尽、模型仍然只给 tool_calls 没有正文时，**再做一次收尾调用**：
    // 把工具**摘掉**，并明确要求「用手上的信息作答」。
    //
    // 为什么非要这一步：模型其实完全有能力说清「我查到了什么、缺什么」——
    // 同一批 4B 在第 1、2、3、6、9、12 条任务里都主动这么做了。
    // 不做这一步，用户拿到的只是一条 `tool_loop_exhausted` 错误，
    // **一句有用的正文都没有**，而那些信息本来就在上下文里。
    let mut forced_final_answer = false;
    if !reply.tool_calls.is_empty() && reply.answer().trim().is_empty() {
        eprintln!("[chat] 工具往返用尽仍无正文，去掉工具再问一次，强制它作答");
        discard_if_visible(&reply, observer, round_no);
        msgs.push(Message::assistant_tool_calls(reply.tool_calls.clone()));
        msgs.push(Message::user(FINAL_ANSWER_PROMPT.to_string()));
        // **不带 tools**：模型此刻已经证明会一直要工具，再给一次只是再要一轮。
        let final_request = crate::llm::build_request(llm_config, msgs.clone());
        round_no += 1;
        observer.round_start(round_no);
        match one_round(provider, mode, &final_request, observer).await {
            Ok(last) => {
                // 收尾这一次也真花了 token：不管它最后有没有被采纳，都记上。
                turn_usage.push(last.usage);
                tool_trace.push(json!({
                    "id": "final",
                    "name": FINAL_ANSWER_MARKER,
                    "arguments": json!({ "rounds_exhausted": MAX_TOOL_ROUNDS }),
                    "ok": true,
                    "result": last.answer().to_string(),
                }));
                if !last.answer().trim().is_empty() {
                    reply = last;
                    forced_final_answer = true;
                } else {
                    discard_if_visible(&last, observer, round_no);
                }
            }
            Err(e) => {
                // 失败不改变结论：下面照旧报 `tool_loop_exhausted`。流式那边
                // 由调用方把这个 Err 变成 `error` 事件，不会无声断连接。
                eprintln!("[chat] 收尾调用失败，保留原来的 tool_loop_exhausted 结论：{e}");
            }
        }
    }
    let turn_ms = started.elapsed().as_millis() as i64;
    // 存档、汇总列、响应 JSON 三处用**同一份**总量。
    let usage = turn_usage.finish();

    Ok(TurnOutcome {
        reply,
        usage,
        tool_trace,
        rounds,
        forced_final_answer,
        turn_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_provider::{ModelInfo, Provider, ProviderStream, SharedProvider};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// 按脚本回话的假 provider：每次调用取一条预设回复。
    /// 脚本用尽后回同一条尾句（测试里一般只用到脚本长度）。
    #[derive(Debug)]
    struct Scripted {
        replies: Mutex<Vec<ChatResponse>>,
        calls: AtomicUsize,
        /// 流式那条路：真把脚本回复拆成增量吐出去。
        stream_enabled: bool,
    }

    impl Scripted {
        fn new(replies: Vec<ChatResponse>, stream_enabled: bool) -> Self {
            Self {
                replies: Mutex::new(replies),
                calls: AtomicUsize::new(0),
                stream_enabled,
            }
        }

        fn next_reply(&self) -> ChatResponse {
            let guard = self.replies.lock().unwrap();
            let idx = self.calls.fetch_add(1, Ordering::SeqCst);
            if guard.is_empty() {
                return text_reply("");
            }
            if idx < guard.len() {
                guard[idx].clone()
            } else {
                guard[guard.len() - 1].clone()
            }
        }
    }

    impl Provider for Scripted {
        fn name(&self) -> &str {
            "scripted"
        }
        fn chat<'a>(
            &'a self,
            _request: &'a ChatRequest,
        ) -> quill_provider::BoxFuture<'a, ChatResponse> {
            Box::pin(async move { Ok(self.next_reply()) })
        }
        fn stream<'a>(
            &'a self,
            _request: &'a ChatRequest,
        ) -> quill_provider::BoxFuture<'a, ProviderStream> {
            Box::pin(async move {
                if !self.stream_enabled {
                    return Err(ProviderError::NotConfigured {
                        detail: "该脚本没开流式".to_string(),
                    });
                }
                let reply = self.next_reply();
                let text = reply.answer().to_string();
                let mut deltas: Vec<Result<StreamDelta, ProviderError>> = Vec::new();
                for ch in text.chars() {
                    deltas.push(Ok(StreamDelta::Text(ch.to_string())));
                }
                let summary = quill_provider::StreamSummary {
                    usage: reply.usage,
                    ..Default::default()
                };
                deltas.push(Ok(StreamDelta::Done(summary)));
                Ok(Box::pin(futures_util::stream::iter(deltas)) as ProviderStream)
            })
        }
        fn models<'a>(&'a self) -> quill_provider::BoxFuture<'a, Vec<ModelInfo>> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    fn text_reply(text: &str) -> ChatResponse {
        ChatResponse {
            id: None,
            model: "stub".into(),
            text: text.to_string(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            finish_reason: None,
            usage: TokenUsage::default(),
        }
    }

    fn tool_reply(name: &str) -> ChatResponse {
        ChatResponse {
            tool_calls: vec![ToolCall::new("c1", name, json!({}))],
            ..text_reply("")
        }
    }

    struct Recorder {
        texts: String,
        discs: Vec<usize>,
        tool_results: Vec<(String, bool)>,
    }
    impl Recorder {
        fn new() -> Self {
            Self {
                texts: String::new(),
                discs: Vec::new(),
                tool_results: Vec::new(),
            }
        }
    }
    impl TurnObserver for Recorder {
        fn text(&mut self, delta: &str) {
            self.texts.push_str(delta);
        }
        fn discard(&mut self, round: usize) {
            self.discs.push(round);
        }
        fn tool_result(&mut self, call: &ToolCall, ok: bool) {
            self.tool_results.push((call.name.clone(), ok));
        }
    }

    fn cfg() -> LlmConfig {
        LlmConfig {
            base_url: "http://localhost".into(),
            api_key: None,
            model: "stub".into(),
            max_tokens: 128,
            ..LlmConfig::default()
        }
    }

    fn registry_with(name: &str, ok: bool) -> ToolRegistry {
        let mut reg = ToolRegistry::default();
        let n = name.to_string();
        reg.register(
            ToolSpec::new(&n, "测试工具"),
            Arc::new(move |_args: &Value| {
                if ok {
                    Ok("工具结果".to_string())
                } else {
                    Err("工具坏了".to_string())
                }
            }),
        );
        reg
    }

    fn shared(replies: Vec<ChatResponse>, stream_enabled: bool) -> SharedProvider {
        Arc::new(Scripted::new(replies, stream_enabled))
    }

    fn input<'a>(
        provider: &'a SharedProvider,
        llm_config: &'a LlmConfig,
        registry: &'a ToolRegistry,
        tools: &'a [ToolSpec],
        messages: Vec<Message>,
        mode: ReplyMode,
    ) -> TurnInput<'a> {
        TurnInput {
            provider,
            llm_config,
            registry,
            tools,
            messages,
            mode,
        }
    }

    #[tokio::test]
    async fn a_plain_reply_needs_no_tool_round() {
        let provider = shared(vec![text_reply("你好")], false);
        let cfg = cfg();
        let reg = ToolRegistry::default();
        let mut obs = Recorder::new();
        let outcome = run_turn(
            input(
                &provider,
                &cfg,
                &reg,
                &[],
                vec![Message::user("嗨")],
                ReplyMode::Once,
            ),
            &mut obs,
        )
        .await
        .unwrap();
        assert_eq!(outcome.reply.answer(), "你好");
        assert_eq!(outcome.rounds, 0);
        assert!(outcome.tool_trace.is_empty());
        assert!(!outcome.forced_final_answer);
    }

    #[tokio::test]
    async fn a_tool_round_feeds_back_and_then_answers() {
        let provider = shared(vec![tool_reply("ping"), text_reply("查到了")], false);
        let cfg = cfg();
        let reg = registry_with("ping", true);
        let tools = reg.specs();
        let mut obs = Recorder::new();
        let outcome = run_turn(
            input(
                &provider,
                &cfg,
                &reg,
                &tools,
                vec![Message::user("查一下")],
                ReplyMode::Once,
            ),
            &mut obs,
        )
        .await
        .unwrap();
        assert_eq!(outcome.rounds, 1);
        assert_eq!(outcome.reply.answer(), "查到了");
        assert_eq!(obs.tool_results, vec![("ping".to_string(), true)]);
        let trace = &outcome.tool_trace[0];
        assert_eq!(trace["name"], "ping");
        assert_eq!(trace["ok"], true);
    }

    #[tokio::test]
    async fn a_failed_tool_still_feeds_back() {
        let provider = shared(
            vec![tool_reply("boom"), text_reply("工具坏了但我还能答")],
            false,
        );
        let cfg = cfg();
        let reg = registry_with("boom", false);
        let tools = reg.specs();
        let mut obs = Recorder::new();
        let outcome = run_turn(
            input(
                &provider,
                &cfg,
                &reg,
                &tools,
                vec![Message::user("试")],
                ReplyMode::Once,
            ),
            &mut obs,
        )
        .await
        .unwrap();
        assert_eq!(obs.tool_results, vec![("boom".to_string(), false)]);
        assert_eq!(outcome.tool_trace[0]["ok"], false);
    }

    #[tokio::test]
    async fn the_budget_is_enforced_and_a_final_answer_is_forced() {
        // 永远只回 tool_calls：预算用尽后应触发收尾调用，收尾回了正文就当答案。
        let mut forever: Vec<ChatResponse> = Vec::new();
        // 首轮 + 每一轮往返各一条，正好用满预算；再多一条会被上限拦住。
        for _ in 0..MAX_TOOL_ROUNDS + 1 {
            forever.push(tool_reply("ping"));
        }
        forever.push(text_reply("勉强答了"));
        let provider = shared(forever, false);
        let cfg = cfg();
        let reg = registry_with("ping", true);
        let tools = reg.specs();
        let mut obs = Recorder::new();
        let outcome = run_turn(
            input(
                &provider,
                &cfg,
                &reg,
                &tools,
                vec![Message::user("找")],
                ReplyMode::Once,
            ),
            &mut obs,
        )
        .await
        .unwrap();
        assert_eq!(outcome.rounds, MAX_TOOL_ROUNDS);
        assert!(outcome.forced_final_answer);
        assert_eq!(outcome.reply.answer(), "勉强答了");
        assert_eq!(
            outcome.tool_trace.last().unwrap()["name"],
            FINAL_ANSWER_MARKER
        );
    }

    #[tokio::test]
    async fn a_streamed_round_reaches_the_observer() {
        let provider = shared(vec![text_reply("流式三字")], true);
        let cfg = cfg();
        let reg = ToolRegistry::default();
        let mut obs = Recorder::new();
        let outcome = run_turn(
            input(
                &provider,
                &cfg,
                &reg,
                &[],
                vec![Message::user("嗨")],
                ReplyMode::Streamed,
            ),
            &mut obs,
        )
        .await
        .unwrap();
        assert_eq!(obs.texts, "流式三字");
        assert_eq!(outcome.reply.answer(), "流式三字");
    }

    #[test]
    fn usage_sums_every_round_and_only_trims_cache_on_multi_round() {
        let mut one = TurnUsage::default();
        one.push(TokenUsage {
            input: Some(10),
            output: Some(2),
            cache_read: Some(30),
            cache_write: None,
        });
        // 单轮：不夹取（cache_read 比 input 大也照记，上游这么报就照抄）。
        assert_eq!(one.finish().cache_read, Some(30));

        let mut many = TurnUsage::default();
        for _ in 0..2 {
            many.push(TokenUsage {
                input: Some(10),
                output: Some(1),
                cache_read: Some(30),
                cache_write: Some(5),
            });
        }
        let total = many.finish();
        assert_eq!(total.input, Some(20));
        assert_eq!(total.output, Some(2));
        // 多轮：cache_read 被夹到不超过 input。
        assert_eq!(total.cache_read, Some(20));
        assert_eq!(total.cache_write, Some(10));
    }

    #[test]
    fn a_missing_usage_stays_missing() {
        let mut acc = TurnUsage::default();
        acc.push(TokenUsage::default());
        let total = acc.finish();
        assert_eq!(total.input, None);
        assert_eq!(total.cache_read, None);
    }
}
