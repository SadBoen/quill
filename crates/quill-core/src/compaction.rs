//! 上下文压缩（queue Q017）。
//!
//! 参考源：`vendor/goose/crates/goose-context-management/`（goose v1.53.0，`.rs` 共
//! 1156 行；`model.rs` 阈值与策略、`structured.rs` 结构化摘要、`summarize.rs` 摘要生成、
//! `provider.rs` 与 provider 的接口、`templates.rs`/`prompts/` 提示词）。
//! 「什么时候该压」还有第二处出处：`vendor/goose/crates/goose/src/context_mgmt/mod.rs:224-274`
//! （`check_if_compaction_needed`）—— `goose-context-management` 只写「怎么压」，
//! 阈值判定在它的调用方里，这里照它抄。
//!
//! **现状（2026-10-09）**：已接线（Q018）。本模块是纯逻辑的唯一归属
//! （最高指示第 3 条：照 goose 抄，不自创算法）。
//!
//! 分工：纯逻辑（何时该压、压成什么形状、压缩前后用量账怎么连续）在这里、可单测；
//! 真正调模型生成摘要与数 token 由壳侧注入 —— 见
//! `crates/quill-server/src/chat_compaction.rs`（provider 与估算器的实现），
//! 接线点是 `api_chat::prepare_turn`。壳读的就是配置字段
//! `compaction_threshold_tokens`，本模块**不读配置**，只收传进来的阈值。
//!
//! # 移植对照表（每一项都能回 `vendor/goose` 核对）
//!
//! - 阈值常量 [`DEFAULT_COMPACTION_THRESHOLD`] = 0.8 ← `goose-context-management/src/lib.rs:32`
//! - 阈值判定 [`should_compact`] ← `goose/src/context_mgmt/mod.rs:224-274`（比例式，严格大于）
//! - 消息文本化 [`format_message_for_compacting`] ← `goose-context-management/src/format.rs:4-83`
//! - 模型调用注入 [`CompactionModel`] ← `goose-context-management/src/model.rs:12-19`
//! - 分词注入 [`TokenEstimator`] ← `goose-context-management/src/model.rs:21-26`
//! - 摘要请求文案 `SUMMARIZE_REQUEST_TEXT` ← `goose-context-management/src/summarize.rs:16-17`
//! - 摘要阶梯（逐级删工具响应）〔`REMOVAL_PERCENTAGES` / `filter_tool_responses`〕
//!   ← 同文件 `:14` / `:36-75`；阶梯循环 `:124-182`
//! - 结构化摘要解析/渲染 [`StructuredSummary`] ← `goose-context-management/src/structured.rs:13-290`
//! - 渲染用的围栏 `code_fence` ← `goose-context-management/src/templates.rs:34-47`
//! - 摘要提示词 `COMPACTION_PROMPT` ← `goose-context-management/src/prompts/compaction.md`
//! - 摘要渲染形态（章节与顺序）← `goose-context-management/src/prompts/compaction_summary.md:10-75`
//! - 用量补齐 `ensure_usage_tokens` ← `goose-context-management/src/summarize.rs:95-119`
//!   （注意：必须在结构化改写**之前**估，见 `:149-156`）
//! - 压缩产物形状 [`CompactionResult`] ← `goose/src/context_mgmt/mod.rs:48-57`、`:70-207`
//!   （原消息全部归档、摘要 + 续跑提示进 agent 可见集、保留最近一条纯文本 user 消息）
//! - 续跑提示文案 ← `goose/src/context_mgmt/mod.rs:33-46`
//!
//! # 与 goose 的差异（不假装逐行等同）
//!
//! 1. **无 `async-trait` 依赖**：goose 的 [`CompactionModel::complete`] 用 `#[async_trait]`
//!    （`model.rs:12`）把 `async fn` 装箱；`quill-core` 没有这个依赖，本轮也不改
//!    `Cargo.toml`，所以手写 `Pin<Box<dyn Future + Send + '_>>` —— 这只是同一个装箱，
//!    不改变调用方的写法。理由：摘要生成是网络调用，壳侧（Q018）在 async 路径上调用，
//!    做成同步会逼调用方阻塞 tokio worker。
//! 2. **`TokenEstimator` 由 async 改 sync，且必填**：goose 的计数要异步加载分词器
//!    （`context_mgmt/mod.rs:305-313`），且 `estimator` 是 `Option`（`summarize.rs:126`）。
//!    quill 这里把分词器当**注入的纯函数**（本模块不做分词、不依赖 HTTP/DB），并且
//!    必填：没有分词器，压缩前后两本用量账对不上（判据要求「用量前后连续」）。
//! 3. **消息模型是最小集**：`{role, text, has_tool_response}`。goose 的 `MessageContent`
//!    有 image/document/tool_request/tool_response/confirmation/action/thinking/
//!    system_notification/error（`format.rs:8-70`）以及可见性元数据（agent/user 可见位、
//!    turn-context）。本轮只移植纯逻辑需要的那部分；turn-context 的继承
//!    （`context_mgmt/mod.rs:176-191`）与 audience 投影没有对应概念，未移植，
//!    影响面记在本节第 6 条。
//! 4. **阈值口径**：goose 是「占上下文窗口的比例」（0.8）。quill 的配置字段
//!    `compaction_threshold_tokens` 是**绝对 token 数**（`crates/quill-server/src/llm.rs:52`），
//!    所以这里两个口径都给：[`should_compact`]（goose 原样）与
//!    [`should_compact_over_tokens`]（quill 口径，比较规则照抄 goose 的严格大于）。
//! 5. **无 minijinja**：goose 用模板引擎渲染提示词与摘要（`templates.rs:18-59`），
//!    提示词可被用户覆盖。quill 本轮不引模板引擎：提示词用字面量替换 `{{ messages }}`，
//!    摘要渲染用代码复刻 `prompts/compaction_summary.md` 的章节形态。因此
//!    `templates.rs:18-32` 的「用户自定义模板」能力本轮没有。
//! 6. **`is_most_recent` 按简化定义**：goose 判「保留的 user 消息后面只剩 turn-context 事件」
//!    （`context_mgmt/mod.rs:122`）；quill 没有 turn-context 概念，故定义为「它就在历史末尾」。
//! 7. **结构化摘要的解析机制**：goose 用 serde derive + 自定义反序列化器
//!    （`structured.rs:15-31`）；quill 直接遍历 `serde_json::Value` 复刻同样的宽松语义。
//!    另外 goose 保留未知字段到 `extra`（`structured.rs:33-37`），供用户自定义渲染模板取用；
//!    quill 的渲染是固定的，故丢弃未知字段（对「是否算空摘要」的影响与 goose 相同）。
//!    JSON 解析用 `serde_json::from_str`，并照 `goose-provider-types/src/json.rs`
//!    补了「字符串里的裸控制字符」转义兜底；goose 还有截断修复，但 `structured.rs:209-214`
//!    明确不对未闭合对象做修复，而候选文档都是括号配平的，故不移植截断修复。
//!    还有一处实测差异：goose 给 `serde_json` 开了 `preserve_order`（该 crate
//!    `Cargo.toml:27`），对象值 stringify 时键按插入序；`quill-core` 没开（字典序），
//!    只影响 `key: value` 的拼接顺序，不影响内容取舍（见 `stringify_lenient` 文档）。
//! 8. **错误类型**：goose 用 `anyhow` + `ProviderError`（`summarize.rs:163-176`）；
//!    `quill-core` 不引 `anyhow`，本模块定义本地枚举，三条文案逐字照抄 goose。
//! 9. **未移植**：`CompactingProvider`（`provider.rs`，那是 provider 抽象层的包装，
//!    属于壳侧接线）、工具对摘要（`context_mgmt/mod.rs:376-590`，另一条独立策略，
//!    按 tool call 批量摘要，不在 Q017 判据内）。
//!
//! # 本轮不做什么（别把这里读成「已经生效」）
//!
//! - **未接线**：本模块没有任何调用方。配置字段 `compaction_threshold_tokens`
//!   （定义在 `crates/quill-server/src/llm.rs:52`）**在本模块里不会被读到** —— 阈值由
//!   参数传入，Q018 决定从哪读、接进哪条对话路径，并在界面上如实反映是否生效。
//! - **未验证**：真实 provider 下的压缩行为、真实分词器的计数、界面显示 —— 本轮都
//!   没跑过（只跑了本文件内的单测，证据见最终交付说明）。
//! - 本模块只依赖 `serde_json` 与标准库，不依赖 HTTP / 数据库 / `quill-server`。

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use serde_json::{Map, Value};

// ===========================================================================
// 一、阈值判定（什么时候该压缩）
// ===========================================================================

/// goose 的自动压缩阈值（占上下文窗口的比例）。
/// 逐字取自 `vendor/goose/crates/goose-context-management/src/lib.rs:32`。
pub const DEFAULT_COMPACTION_THRESHOLD: f64 = 0.8;

/// goose 口径的「该不该压」：`check_if_compaction_needed`
/// （`vendor/goose/crates/goose/src/context_mgmt/mod.rs:224-274`）的纯逻辑部分。
///
/// 语义逐条对齐：
/// - 占用比例 = `current_tokens / context_limit`（同文件 `:266`）；
/// - 阈值 `<= 0.0` 或 `>= 1.0` 视为**关闭**自动压缩（`:268-269`）；
/// - 比较是**严格大于**（`:271`）：占用**恰好等于**阈值时不压。
///
/// 未移植的那部分（需要 provider/会话，属于壳侧）：
/// - `provider.manages_own_context()` 为真时直接不压（`:230-232`）—— 调用方先判；
/// - `current_tokens` 优先取会话元数据、缺失才现算（`:249-263`）—— 数字由调用方给。
///
/// `context_limit == 0` 按 IEEE754 原样（正数/0 = inf → 压；0/0 = NaN → 不压），
/// 与 goose 的除法行为一致，没有额外发明。
pub fn should_compact(current_tokens: usize, context_limit: usize, threshold: f64) -> bool {
    let usage_ratio = current_tokens as f64 / context_limit as f64;
    if threshold <= 0.0 || threshold >= 1.0 {
        false // Auto-compact is disabled.（goose 原注释，context_mgmt/mod.rs:269）
    } else {
        usage_ratio > threshold
    }
}

/// quill 配置口径的「该不该压」：`compaction_threshold_tokens` 是**绝对 token 数**
/// （`crates/quill-server/src/llm.rs:52`），不是比例。
///
/// **差异与原因**：goose 没有这个口径（它只按比例压，`context_mgmt/mod.rs:236-240`）。
/// quill 的配置字段既然以 token 计，就照着 goose 的**比较规则**（严格大于，`:271`）
/// 定义同一条边界：`current == threshold` 不压。goose 的「0/≥1 关闭」是比例口径的
/// 哨兵（`:268-269`），绝对口径没有对应哨兵；quill 的配置校验保证阈值 ≥ 4001 且
/// 不超过上下文窗口（`crates/quill-server/src/llm.rs:236-247`），Q018 若要「关闭」
/// 语义，应在调用处判断，别在这里发明哨兵值。
pub fn should_compact_over_tokens(current_tokens: usize, threshold_tokens: usize) -> bool {
    current_tokens > threshold_tokens
}

// ===========================================================================
// 二、消息与分词（注入的纯逻辑）
// ===========================================================================

/// 消息角色。goose 用 `rmcp::model::Role` 的 `User` / `Assistant`
/// （`goose-context-management/src/format.rs:73-76`）；本模块只保留这两者。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

/// 参与压缩的消息。
///
/// **差异**：goose 的 `Message` 带完整 `MessageContent` 列表与可见性元数据；
/// quill 这里是最小集——纯逻辑只需要「角色 / 文本 / 是否含工具响应」。
/// `has_tool_response` 对应 goose 的 `has_tool_response`
/// （`goose-context-management/src/summarize.rs:30-34`）：只看有没有
/// `MessageContent::ToolResponse`，用于阶梯式删除。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionMessage {
    pub role: MessageRole,
    pub text: String,
    /// 是否含工具响应内容（goose: `MessageContent::ToolResponse`）。
    pub has_tool_response: bool,
}

impl CompactionMessage {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            text: text.into(),
            has_tool_response: false,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            text: text.into(),
            has_tool_response: false,
        }
    }

    /// 标记这条消息含工具响应（goose 里工具响应挂在 user 角色的消息上）。
    pub fn with_tool_response(mut self) -> Self {
        self.has_tool_response = true;
        self
    }
}

/// 一条消息在摘要器输入里的文本形态。
///
/// 照 `goose-context-management/src/format.rs:4-83` 抄：`[role]: 内容`，
/// 无内容时 `[role]: <empty message>`（`:78-82`）。
///
/// **差异**：goose 还处理 image/document/tool_request/tool_response/confirmation/
/// action/thinking/system_notification/error 各自的行格式（`:8-70`）；本模块的消息
/// 只有文本 + `has_tool_response` 标记，因此只移植文本与非空判定两条分支。
pub fn format_message_for_compacting(message: &CompactionMessage) -> String {
    let role_str = match message.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
    };
    if message.text.is_empty() {
        format!("[{role_str}]: <empty message>")
    } else {
        format!("[{role_str}]: {}", message.text)
    }
}

/// 分词器。goose 对应 `TokenEstimator`（`goose-context-management/src/model.rs:21-26`），
/// 用它的 `count_chat_tokens` / `count_text_tokens`。
///
/// **差异与原因**：goose 的方法返回 `usize` 但是 async（要异步加载 tiktoken 实现，
/// `goose/src/context_mgmt/mod.rs:305-313`）；quill 在这里把分词当成**注入的纯函数**，
/// 同步、无 IO —— 本模块不做分词实现，壳侧（Q018）注入真实现，单测注入确定性的假实现。
/// `Send + Sync` 是**接线要求**（Q018）：壳在 async 路径上调用 [`compact`]，
/// 这些 trait 对象的引用要跨 await 点，没有它整条 future 就不是 `Send`，
/// tokio::spawn 编译不过。goose 的实现天然满足（它用 async-trait，默认要求 Send）。
pub trait TokenEstimator: Send + Sync {
    /// 一段文本的 token 数。goose: `count_text_tokens`（`model.rs:25`）。
    fn count_text_tokens(&self, text: &str) -> usize;

    /// 一条消息的 token 数。goose 用 `count_chat_tokens("", [msg], &[])` 现算
    /// （`goose/src/context_mgmt/mod.rs:218`、`:259`）。
    fn count_message_tokens(&self, message: &CompactionMessage) -> usize {
        self.count_text_tokens(&message.text)
    }

    /// 一段「系统提示 + 消息序列」的 token 数。goose: `count_chat_tokens(system, messages)`
    /// （`model.rs:24`）。
    ///
    /// **差异**：goose 多一个 `tools` 参数（工具 schema 也计费），本模块的消息模型
    /// 不带工具 schema，故只有两个参数。
    fn count_chat_tokens(&self, system: &str, messages: &[CompactionMessage]) -> usize {
        self.count_text_tokens(system)
            + messages
                .iter()
                .map(|message| self.count_message_tokens(message))
                .sum::<usize>()
    }
}

// ===========================================================================
// 三、用量（token 账）
// ===========================================================================

/// 一次模型调用的 token 用量。字段形状对齐 goose 的 `Usage`
/// （`vendor/goose/crates/goose-provider-types/src/conversation/token_usage.rs:93-101`）：
/// 三项都是 `Option`，`None` = provider 没报。
///
/// 注意 goose 的 `ensure_usage_tokens`（`goose-context-management/src/summarize.rs:116-118`）
/// 在 input/output 都有值时会**重算 total = input + output**（覆盖 provider 报的 total）。
/// 本模块照抄这个行为（见 `ensure_usage_tokens`），因为它保证「摘要自身的用量」
/// 始终自洽，压缩前后的账才能对上。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub total_tokens: Option<usize>,
}

impl TokenUsage {
    /// 本次调用的计费用量：优先用 provider 报的 total，缺失时退回 `input + output`。
    pub fn total(&self) -> usize {
        match self.total_tokens {
            Some(total) => total,
            None => self.input_tokens.unwrap_or(0) + self.output_tokens.unwrap_or(0),
        }
    }
}

/// 计费口径的连续性（判据「用量前后连续」的账）：
/// **压缩前的会话累计用量 + 本次摘要调用自身的用量 = 压缩后的会话累计用量**。
///
/// **这是 quill 的补充**（goose 里由调用方把这次调用的用量并进会话指标：
/// `goose/src/agents/agent.rs:2423`、`:3204` 的
/// `update_session_metrics(..., &compaction.usage, Some(compaction.retained_context_tokens))`；
/// `goose-context-management` 只把 `CompactionResult.usage` 交出去，
/// `context_mgmt/mod.rs:48-57`）。
/// 单独写成函数是为了让「不能凭空丢、也不能翻倍」这条账**可被单测钉住**。
pub fn billable_total_after(billed_before: usize, usage: &TokenUsage) -> usize {
    billed_before + usage.total()
}

// ===========================================================================
// 四、模型调用注入
// ===========================================================================

/// 摘要模型的原始输出 + 它自己报的用量。
///
/// goose 对应 `(Message, ProviderUsage)` 二元组（`model.rs:18`）：`raw_output` 是
/// 消息正文（可能含 `<analysis>` 草稿与 ```json 块），`usage` 是这次调用的计费用量。
/// 本结构不带角色——摘要进历史时一律改成 user 角色（`summarize.rs:146`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSummary {
    pub raw_output: String,
    pub usage: TokenUsage,
}

/// 压缩过程的错误。goose 用 `anyhow` + `ProviderError`（`summarize.rs:163-176`）；
/// `quill-core` 不引 `anyhow`，这里定义本地枚举，**文案逐字照抄** goose 的三条消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionError {
    /// 对应 `ProviderError::ContextLengthExceeded`（goose-provider-types）：
    /// 摘要器自身也装不下这次输入。
    ContextLengthExceeded(String),
    /// 溢出且历史里没有工具响应可删（`summarize.rs:163-167`）。
    NoRemovableContext,
    /// 工具响应删光了仍溢出（`summarize.rs:170-174`）。
    ContextExceededAfterRemoval,
    /// 其余模型错误（`summarize.rs:175`：goose 直接 `?` 上抛）。
    Model(String),
}

impl fmt::Display for CompactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContextLengthExceeded(message) => write!(f, "{message}"),
            Self::NoRemovableContext => write!(
                f,
                "Failed to compact: the base prompt (system prompt, tool schemas, and conversation) \
                 exceeds the model's effective context window, and there are no tool responses to \
                 remove. Use a model or configuration with a larger usable context, disable some \
                 extensions to reduce the tool-schema payload, or start a new session."
            ),
            Self::ContextExceededAfterRemoval => write!(
                f,
                "Failed to compact: context limit exceeded even after removing all tool responses"
            ),
            Self::Model(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CompactionError {}

/// 压缩需要的那**一次**模型调用（生成摘要）。
///
/// 移植自 `goose-context-management/src/model.rs:12-19` 的 `CompactionModel::complete`：
/// 「Implementations decide model selection, fallbacks and session plumbing.」
///
/// **差异与原因**（详见模块头第 1 条）：goose 用 `#[async_trait]`；quill 手写装箱
/// `Pin<Box<dyn Future + Send + '_>>`，因为 `quill-core` 没有 `async-trait` 依赖，
/// 本轮也不改 `Cargo.toml`。实现方写一样的 `async` 块，例如：
///
/// ```ignore
/// fn complete<'a>(&'a self, system: &'a str, request: &'a [CompactionMessage])
///     -> Pin<Box<dyn Future<Output = Result<GeneratedSummary, CompactionError>> + Send + 'a>>
/// {
///     Box::pin(async move { self.provider.complete(system, request).await })
/// }
/// ```
pub trait CompactionModel: Send + Sync {
    fn complete<'a>(
        &'a self,
        system: &'a str,
        request: &'a [CompactionMessage],
    ) -> Pin<Box<dyn Future<Output = Result<GeneratedSummary, CompactionError>> + Send + 'a>>;
}

// ===========================================================================
// 五、结构化摘要（压缩成什么形状）
// ===========================================================================

/// 文件活动条目。字段与 JSON 键名照 `goose-context-management/src/structured.rs:40-48`
/// 的 `FileActivity`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileActivity {
    pub path: String,
    pub summary: String,
    pub key_code: Option<String>,
}

/// 结构化摘要。字段与 JSON 键名照 `goose-context-management/src/structured.rs:13-38`
/// 的 `StructuredSummary`：每个列表**最重要在前**（消费者可以从尾部截断）。
///
/// **差异**：goose 有 `extra`（未知字段，`structured.rs:33-37`），是为了让用户自定义的
/// 渲染模板还能取到它们；quill 的渲染是固定的（见 [`StructuredSummary::render`]），
/// 所以未知字段在解析时就丢弃 —— 对 `is_empty` 的影响与 goose 相同（未知字段不算内容）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructuredSummary {
    pub user_intent: Vec<String>,
    pub technical_concepts: Vec<String>,
    pub files: Vec<FileActivity>,
    pub errors_and_fixes: Vec<String>,
    pub problem_solving: Vec<String>,
    pub user_messages: Vec<String>,
    pub pending_tasks: Vec<String>,
    pub current_work: Option<String>,
    pub next_step: Option<String>,
}

impl StructuredSummary {
    /// 从模型响应里找出一份可用的结构化摘要。
    ///
    /// 照 `goose-context-management/src/structured.rs:128-138`：候选文档逐个尝试，
    /// 解析、归一（`Self::normalize`）、非空才算数；全都失败返回 `None`，
    /// 让调用方保留**原始文本**（无损回退，`:131-137` 的注释）。
    pub fn parse(response_text: &str) -> Option<Self> {
        json_candidates(response_text)
            .into_iter()
            .find_map(|candidate| {
                let value = parse_json_lenient(candidate)?;
                let mut summary = Self::from_value(&value);
                summary.normalize();
                (!summary.is_empty()).then_some(summary)
            })
    }

    /// 宽松地把一个 JSON 值读成摘要。
    ///
    /// goose 用 serde 的宽松反序列化器（`structured.rs:50-126`）达到同样效果；
    /// quill 手动遍历 `Value`，语义逐条对应：
    /// - 列表字段：数组逐项 stringify；`null` → 空；其它值 → 单项（`:68-78`）；
    /// - 字符串字段：非字符串也被 stringify 而不是报错（`:80-86`）；
    /// - 可选字符串：`null` → `None`，其它 → stringify（`:88-97`）；
    /// - `files`：对象按对象读，字符串按「只有 path」读，空 path 丢弃（`:99-126`）。
    fn from_value(value: &Value) -> Self {
        let Value::Object(object) = value else {
            return Self::default();
        };
        Self {
            user_intent: string_list(object.get("user_intent")),
            technical_concepts: string_list(object.get("technical_concepts")),
            files: file_list(object.get("files")),
            errors_and_fixes: string_list(object.get("errors_and_fixes")),
            problem_solving: string_list(object.get("problem_solving")),
            user_messages: string_list(object.get("user_messages")),
            pending_tasks: string_list(object.get("pending_tasks")),
            current_work: string_opt(object.get("current_work")),
            next_step: string_opt(object.get("next_step")),
        }
    }

    /// 丢掉空白条目，让「整篇都是空字符串」的响应算空（回退原始文本），
    /// 而不是渲染出一份什么都没有的摘要。照 `structured.rs:150-180`。
    fn normalize(&mut self) {
        fn blank(text: &str) -> bool {
            text.trim().is_empty()
        }
        for list in [
            &mut self.user_intent,
            &mut self.technical_concepts,
            &mut self.errors_and_fixes,
            &mut self.problem_solving,
            &mut self.user_messages,
            &mut self.pending_tasks,
        ] {
            list.retain(|entry| !blank(entry));
        }
        for file in &mut self.files {
            if file.key_code.as_deref().is_some_and(blank) {
                file.key_code = None;
            }
        }
        self.files
            .retain(|file| !blank(&file.path) || !blank(&file.summary) || file.key_code.is_some());
        if self.current_work.as_deref().is_some_and(blank) {
            self.current_work = None;
        }
        if self.next_step.as_deref().is_some_and(blank) {
            self.next_step = None;
        }
    }

    /// 照 `structured.rs:182-192`：未知字段不算内容，故只看这些已知字段。
    fn is_empty(&self) -> bool {
        self.user_intent.is_empty()
            && self.technical_concepts.is_empty()
            && self.files.is_empty()
            && self.errors_and_fixes.is_empty()
            && self.problem_solving.is_empty()
            && self.user_messages.is_empty()
            && self.pending_tasks.is_empty()
            && self.current_work.is_none()
            && self.next_step.is_none()
    }

    /// 渲染成进入历史的 Markdown 摘要。
    ///
    /// **差异与原因**：goose 用可覆盖的 minijinja 模板
    /// （`prompts/compaction_summary.md` + `templates.rs:49-59`）；quill 不引模板引擎，
    /// 这里用代码复刻**同一个章节序列**（`prompts/compaction_summary.md:10-75`）：
    /// `# Conversation Summary`，随后按序 User Intent / Technical Concepts /
    /// Files + Code / Errors + Fixes / Problem Solving / User Messages /
    /// Pending Tasks / Current Work / Next Step，空章节整节省略，
    /// 末尾 `trim()`（对应 `templates.rs:58`）。
    pub fn render(&self) -> String {
        let mut out = String::from("# Conversation Summary\n\n");
        push_list(&mut out, "## User Intent", &self.user_intent);
        push_list(&mut out, "## Technical Concepts", &self.technical_concepts);
        push_files(&mut out, &self.files);
        push_list(&mut out, "## Errors + Fixes", &self.errors_and_fixes);
        push_list(&mut out, "## Problem Solving", &self.problem_solving);
        push_list(&mut out, "## User Messages", &self.user_messages);
        push_list(&mut out, "## Pending Tasks", &self.pending_tasks);
        if let Some(current_work) = &self.current_work {
            out.push_str(&format!("## Current Work\n{current_work}\n\n"));
        }
        if let Some(next_step) = &self.next_step {
            out.push_str(&format!("## Next Step\n{next_step}\n"));
        }
        out.trim().to_string()
    }
}

/// 摘要正文的最终形态：模型返回结构化 JSON 就渲染成 Markdown，否则**原样保留**
/// （无损回退）。照 `goose-context-management/src/summarize.rs:77-93` 的
/// `apply_structured_summary`。
///
/// **差异**：goose 在渲染失败/渲染为空时打 `warn!` 日志；本模块无日志设施，
/// 这两种情况同样回退原始输出（行为一致，只是没有日志）。
pub fn apply_structured_summary(response_text: &str) -> String {
    let Some(summary) = StructuredSummary::parse(response_text) else {
        return response_text.to_string();
    };
    let rendered = summary.render();
    if rendered.trim().is_empty() {
        response_text.to_string()
    } else {
        rendered
    }
}

fn push_list(out: &mut String, heading: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(heading);
    out.push('\n');
    for item in items {
        out.push_str("- ");
        out.push_str(item);
        out.push('\n');
    }
    out.push('\n');
}

fn push_files(out: &mut String, files: &[FileActivity]) {
    if files.is_empty() {
        return;
    }
    out.push_str("## Files + Code\n\n");
    for file in files {
        if !file.path.is_empty() {
            out.push_str("### ");
            out.push_str(&file.path);
            out.push('\n');
        }
        out.push_str(&file.summary);
        out.push('\n');
        if let Some(key_code) = &file.key_code {
            out.push_str(&code_fence(key_code));
            out.push('\n');
        }
        out.push('\n');
    }
}

/// 用足够长的反引号围栏包住代码，嵌入的围栏无法越狱。
/// 照 `goose-context-management/src/templates.rs:34-47` 的 `code_fence` 过滤器。
fn code_fence(code: &str) -> String {
    let longest_run = code
        .chars()
        .fold((0usize, 0usize), |(max, run), c| {
            if c == '`' {
                (max.max(run + 1), run + 1)
            } else {
                (max, 0)
            }
        })
        .0;
    let fence = "`".repeat((longest_run + 1).max(3));
    format!("{fence}\n{}\n{fence}", code.trim_end_matches('\n'))
}

/// `null` → 空串；对象/数组按 `key: value` / `;` 拼接；其它值 `to_string()`。
/// 照 `structured.rs:50-66` 的 `stringify_lenient`。
///
/// **差异**：goose 的 `goose-context-management` 给 `serde_json` 开了
/// `preserve_order`（该 crate 的 `Cargo.toml:27`），对象键按**插入序**拼接；
/// `quill-core` 的 `serde_json` 没开这个 feature（键是 BTreeMap 的**字典序**），
/// 所以同一个对象值拼出来的键值顺序可能不同。只影响「对象被 stringify 成一行」时的
/// 拼接顺序，不影响字段的取舍与摘要内容——本轮不许改 `Cargo.toml`，故如实记差异。
fn stringify_lenient(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| format!("{key}: {}", stringify_lenient(value)))
            .collect::<Vec<_>>()
            .join("; "),
        Value::Array(items) => items
            .iter()
            .map(stringify_lenient)
            .collect::<Vec<_>>()
            .join("; "),
        other => other.to_string(),
    }
}

/// 照 `structured.rs:68-78` 的 `lenient_string_list`。
fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().map(stringify_lenient).collect(),
        Some(other) => vec![stringify_lenient(other)],
    }
}

/// 照 `structured.rs:88-97` 的 `lenient_string_opt`。
fn string_opt(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(other) => Some(stringify_lenient(other)),
    }
}

/// 照 `structured.rs:99-126` 的 `lenient_file_list`：条目应为对象，但模型可能
/// 直接给字符串，那就当成「只有 path」的活动，而不是丢掉整份摘要。
fn file_list(value: Option<&Value>) -> Vec<FileActivity> {
    let items: Vec<&Value> = match value {
        None | Some(Value::Null) => return Vec::new(),
        Some(Value::Array(items)) => items.iter().collect(),
        Some(other) => vec![other],
    };
    items
        .into_iter()
        .filter_map(|item| match item {
            Value::Object(object) => Some(file_activity(object)),
            other => {
                let path = stringify_lenient(other);
                if path.trim().is_empty() {
                    None
                } else {
                    Some(FileActivity {
                        path,
                        summary: String::new(),
                        key_code: None,
                    })
                }
            }
        })
        .collect()
}

/// 照 `structured.rs:40-48` 的字段语义宽松读取一个文件活动对象。
fn file_activity(object: &Map<String, Value>) -> FileActivity {
    FileActivity {
        path: string_opt(object.get("path")).unwrap_or_default(),
        summary: string_opt(object.get("summary")).unwrap_or_default(),
        key_code: string_opt(object.get("key_code")),
    }
}

/// 解析一个候选 JSON 文档：先 `serde_json::from_str`，失败再按
/// `goose-provider-types/src/json.rs` 的 `json_escape_control_chars_in_string`
/// 转义裸控制字符重试。
///
/// **差异**：goose 的 `safely_parse_json`（`json.rs:10-24`）还包含截断修复；
/// 但候选文档由 `leading_object` 保证括号配平，截断修复用不上，故不移植。
fn parse_json_lenient(candidate: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str(candidate) {
        return Some(value);
    }
    serde_json::from_str(&escape_control_chars(candidate)).ok()
}

/// 照 `goose-provider-types/src/json.rs` 的 `json_escape_control_chars_in_string`：
/// 把 U+0000..=U+001F 的裸控制字符换成 JSON 转义（常见于模型输出里没转义的换行）。
fn escape_control_chars(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{0000}'..='\u{001F}' => match c {
                '\u{0008}' => escaped.push_str("\\b"),
                '\u{000C}' => escaped.push_str("\\f"),
                '\n' => escaped.push_str("\\n"),
                '\r' => escaped.push_str("\\r"),
                '\t' => escaped.push_str("\\t"),
                other => escaped.push_str(&format!("\\u{:04x}", other as u32)),
            },
            other => escaped.push(other),
        }
    }
    escaped
}

/// 响应文本里的候选 JSON 文档，按顺序尝试。
///
/// 照 `goose-context-management/src/structured.rs:195-240` 的 `json_candidates`：
/// 每个 `</analysis>` 终止符之后（最后一个优先）先试它之后的 ```json 围栏（最后一个
/// 优先）再试开头的对象，最后试整段文本开头的对象。终止符之前的候选要重试，是因为
/// 摘要 JSON 自己可能引用 `</analysis>`（例如正在改压缩提示词的会话），这样简单的
/// `rfind` 会被骗；候选只有在「它包含了后面所有终止符出现」时才被接受——证明那些
/// 终止符是它内部引用的——草稿区里被围栏包住的示例因此不可能漏出来。
///
/// 每个候选都必须紧贴标记、且括号配平地闭合到 `}`（`:209-214`：用括号配平而不是
/// 围栏界定，因为字符串值里合法地可以是 ```；也不修复未闭合对象——修复会丢掉
/// 原始文本回退才能保住的后半段内容）。
fn json_candidates(text: &str) -> Vec<&str> {
    const TERMINATOR: &str = "</analysis>";

    let mut cuts: Vec<usize> = text
        .match_indices(TERMINATOR)
        .map(|(idx, _)| idx + TERMINATOR.len())
        .collect();
    if cuts.is_empty() {
        cuts.push(0);
    }

    let mut candidates: Vec<&str> = Vec::new();
    for &cut in cuts.iter().rev() {
        let tail = &text[cut..];
        let later_terminators = tail.matches(TERMINATOR).count();
        candidates.extend(
            fenced_json_blocks(tail)
                .chain(leading_object(tail))
                .filter(|candidate| candidate.matches(TERMINATOR).count() == later_terminators),
        );
    }
    candidates.extend(leading_object(text));
    candidates.dedup();
    candidates
}

/// 每个围栏都试，最后一个优先：字符串值里可能又引用了一段围栏 JSON，
/// 这种内嵌围栏不能把真正的候选挡住。照 `structured.rs:242-252`。
fn fenced_json_blocks(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices("```json")
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter_map(|(idx, marker)| leading_object(&text[idx + marker.len()..]))
}

/// 取出文本开头那个括号配平的 JSON 对象（跳过前导空白）。
/// 照 `structured.rs:254-290` 的 `leading_object`。
fn leading_object(text: &str) -> Option<&str> {
    let text = text.trim_start();
    if !text.starts_with('{') {
        return None;
    }

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (idx, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[..=idx]);
                }
            }
            _ => {}
        }
    }

    None
}

// ===========================================================================
// 六、摘要生成（阶梯式重试）
// ===========================================================================

/// 摘要调用里那条固定的用户消息。
/// 逐字取自 `goose-context-management/src/summarize.rs:16-17`。
const SUMMARIZE_REQUEST_TEXT: &str =
    "Please summarize the conversation history provided in the system prompt.";

/// 摘要器自己溢出时的删除梯度（百分比）。逐字取自
/// `goose-context-management/src/summarize.rs:14`。
const REMOVAL_PERCENTAGES: [u32; 5] = [0, 10, 20, 50, 100];

/// 压缩提示词。逐字取自
/// `vendor/goose/crates/goose-context-management/src/prompts/compaction.md`
/// （goose 用 minijinja 渲染 `{{ messages }}`，`templates.rs:49-59`；
/// quill 用字面量替换，差异见模块头第 5 条）。
const COMPACTION_PROMPT: &str = r##"## Task Context
- An llm context limit was reached when a user was in a working session with an agent (you)
- Distill the conversation below into a structured summary with only the most verbose parts removed
- Include user requests, your responses, all technical content, and as much of the original context as possible
- This will be used to let the user continue the working session
- The summary will be read by an agent (you) on a next exchange to allow for continuation of the session

**Conversation History:**
{{ messages }}

Wrap reasoning in `<analysis>` tags:
- Review conversation chronologically: user goals, your methods, key decisions, files, errors, fixes
- Keep this brief - the analysis is discarded, so it is a checklist of what to include, not the place for detail

After the closing `</analysis>` tag, output exactly one ```json code block and nothing else, matching this schema:

```json
{
  "user_intent": ["every user goal and request, most important first"],
  "technical_concepts": ["all discussed tools, methods, and concepts"],
  "files": [
    {
      "path": "path of a file that was viewed or edited",
      "summary": "what was done to it and why",
      "key_code": "important code, signatures, or diffs from this file (omit if none)"
    }
  ],
  "errors_and_fixes": ["bugs hit, their resolutions, and user-driven changes"],
  "problem_solving": ["issues solved or in progress, and key decisions: what was chosen, what was rejected, and why"],
  "user_messages": ["all user messages, truncating long tool call arguments or results"],
  "pending_tasks": ["all unresolved user requests, most important first"],
  "current_work": "active work at summary request time: filenames, code, alignment to latest instruction",
  "next_step": "include only if it directly continues a user instruction, otherwise omit"
}
```

Rules for the JSON:
- The `<analysis>` block is a discarded scratchpad: only the JSON survives, so it must be self-contained and repeat every detail from the analysis that matters for continuing
- Order every list from most to least important
- Every list entry must be a plain string, not a nested object - except `files`, whose entries are objects shaped as shown above
- Quote error messages, panic text, and failing test output verbatim in `errors_and_fixes` - exact strings including numbers, identifiers, and paths, not paraphrases
- This summary will only be read by you, so it is ok to make it much longer than a normal summary you would show to a human: spend your entire length budget on the JSON fields, and quote liberally - full output blocks, complete code snippets, exact user wording
- Do not exclude any information that might be important to continuing a session working with you
- Omit a field rather than inventing content for it
- No new ideas unless user confirmed"##;

/// 用格式化后的历史替换 `{{ messages }}`。
///
/// 对应 goose 的 `render(&templates.compaction, &SummarizeContext { messages })`
/// （`summarize.rs:135-142` + `templates.rs:49-59`），并同样 `trim()`。
pub fn compaction_prompt(formatted_history: &str) -> String {
    COMPACTION_PROMPT
        .replace("{{ messages }}", formatted_history)
        .trim()
        .to_string()
}

/// 摘要结果：进历史的那条消息文本 + 这次调用的计费用量。
///
/// 对应 goose 的 `Summary { message, usage }`（`summarize.rs:24-28`）：
/// `text` 是**已经过结构化渲染**的正文，`usage` 计的是**原始输出**（见下）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub text: String,
    pub usage: TokenUsage,
}

fn has_tool_response(message: &CompactionMessage) -> bool {
    message.has_tool_response
}

/// 按百分比从中点向外删工具响应：那里离当前工作最远，删了最不容易伤到上下文。
/// 逐行照 `goose-context-management/src/summarize.rs:36-75` 的 `filter_tool_responses`
/// 移植（含「至少删一条」与中点向外交替取左右两侧的行为）。
fn filter_tool_responses(
    messages: &[CompactionMessage],
    remove_percent: u32,
) -> Vec<&CompactionMessage> {
    if remove_percent == 0 {
        return messages.iter().collect();
    }

    let tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| has_tool_response(message))
        .map(|(index, _)| index)
        .collect();

    if tool_indices.is_empty() {
        return messages.iter().collect();
    }

    let num_to_remove = ((tool_indices.len() * remove_percent as usize) / 100).max(1);
    let middle = tool_indices.len() / 2;
    let mut indices_to_remove = Vec::new();

    for i in 0..num_to_remove {
        let offset = i / 2;
        if i % 2 == 0 {
            if middle > offset {
                indices_to_remove.push(tool_indices[middle - offset - 1]);
            }
        } else if middle + offset < tool_indices.len() {
            indices_to_remove.push(tool_indices[middle + offset]);
        }
    }

    messages
        .iter()
        .enumerate()
        .filter(|(index, _)| !indices_to_remove.contains(index))
        .map(|(_, message)| message)
        .collect()
}

/// provider 没报用量时用分词器补齐，并保证 `total = input + output`。
/// 照 `goose-context-management/src/summarize.rs:95-119`。
///
/// **差异**：goose 的 output 估计是对响应里的内容块逐个 `format!("{}", c)` 再 join(" ")
/// （`:106-113`）；本模块的消息只有一段文本，直接估这段文本。
fn ensure_usage_tokens(
    usage: &mut TokenUsage,
    estimator: &dyn TokenEstimator,
    system_prompt: &str,
    request: &[CompactionMessage],
    raw_output: &str,
) {
    if usage.input_tokens.is_none() {
        let count = estimator.count_chat_tokens(system_prompt, request);
        usage.input_tokens = Some(count);
    }
    if usage.output_tokens.is_none() {
        let count = estimator.count_text_tokens(raw_output);
        usage.output_tokens = Some(count);
    }
    if let (Some(input), Some(output)) = (usage.input_tokens, usage.output_tokens) {
        usage.total_tokens = Some(input + output);
    }
}

/// 把历史摘要成一条消息，摘要器自己溢出时逐级删工具响应重试。
/// 照 `goose-context-management/src/summarize.rs:121-182` 的 `summarize`。
///
/// 关键顺序（`:149-156`）：用量必须按**原始模型输出**估/记（那是计费口径），
/// 之后才把响应改写成更小的渲染摘要——否则压缩后的账会凭空变小。
pub async fn summarize(
    model: &dyn CompactionModel,
    estimator: &dyn TokenEstimator,
    messages: &[CompactionMessage],
) -> Result<Summary, CompactionError> {
    let request = vec![CompactionMessage::user(SUMMARIZE_REQUEST_TEXT)];
    let has_tool_responses = messages.iter().any(has_tool_response);

    for (attempt, &remove_percent) in REMOVAL_PERCENTAGES.iter().enumerate() {
        let filtered = filter_tool_responses(messages, remove_percent);
        let formatted = filtered
            .iter()
            .map(|message| format_message_for_compacting(message))
            .collect::<Vec<_>>()
            .join("\n");
        let system_prompt = compaction_prompt(&formatted);

        match model.complete(&system_prompt, &request).await {
            Ok(mut generated) => {
                // 用量按原始输出估（goose `summarize.rs:149-156` 的注释：
                // 「Usage must reflect the raw model output (billable tokens), so
                // estimate before the response is rewritten to the smaller rendered
                // summary.」）。
                ensure_usage_tokens(
                    &mut generated.usage,
                    estimator,
                    &system_prompt,
                    &request,
                    &generated.raw_output,
                );
                // 摘要进历史时是 user 角色（goose `summarize.rs:146`：`response.role = Role::User`）。
                let text = apply_structured_summary(&generated.raw_output);
                return Ok(Summary {
                    text,
                    usage: generated.usage,
                });
            }
            Err(CompactionError::ContextLengthExceeded(_)) if !has_tool_responses => {
                // 文案照抄 `summarize.rs:164-166`。
                return Err(CompactionError::NoRemovableContext);
            }
            Err(CompactionError::ContextLengthExceeded(_))
                if attempt < REMOVAL_PERCENTAGES.len() - 1 => {}
            Err(CompactionError::ContextLengthExceeded(_)) => {
                // 文案照抄 `summarize.rs:171-173`。
                return Err(CompactionError::ContextExceededAfterRemoval);
            }
            Err(error) => return Err(error),
        }
    }

    // goose 在这里是 unreachable 的兜底（`summarize.rs:179-181`）；循环必然在上面返回，
    // 这里保留同样的兜底语义（用同一个「删光仍溢出」变体，不发明新状态）。
    Err(CompactionError::ContextExceededAfterRemoval)
}

// ===========================================================================
// 七、压缩决策：形状与账
// ===========================================================================

/// 压缩方式。goose: `compact_messages(..., manual_compact: bool)`
/// （`goose/src/context_mgmt/mod.rs:70-76`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionMode {
    /// 自动压（超阈值那条路）：保留最近一条纯文本 user 消息，续跑提示按场景选。
    Auto,
    /// 用户手动压（`:69`）：不保留 user 消息，续跑提示用 manual 文案。
    Manual,
}

/// 压缩后的 agent 可见历史。对应 goose 的 `CompactionResult.conversation`
/// （`goose/src/context_mgmt/mod.rs:48-57`、`:136-192`）：
/// 原消息**全部保留但归档**（agent 不可见，`:142-146`），agent 可见集是
/// 「摘要 + 续跑提示（+ 原样保留的最近一条纯文本 user 消息）」（`:148-172`）。
///
/// 本模块不持有会话，所以把归档那一段原样交回调用方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionResult {
    /// 摘要消息。agent 可见，角色为 user（`summarize.rs:146`）。goose 里它是
    /// `MessageMetadata::agent_only()`（`context_mgmt/mod.rs:148`）。
    pub summary: CompactionMessage,
    /// 续跑提示，assistant 角色（`context_mgmt/mod.rs:160-164`）。
    pub continuation: CompactionMessage,
    /// 原样保留的最近一条纯文本 user 消息副本（`context_mgmt/mod.rs:96-129`、`:169-172`）。
    pub preserved_user: Option<CompactionMessage>,
    /// 归档：压缩前的原历史，全部保留但不再是 agent 可见（`:142-146`）。
    pub archived: Vec<CompactionMessage>,
    /// 本次摘要调用的**计费用量**（按原始输出算，见 [`summarize`]）。
    pub usage: TokenUsage,
    /// 压缩前后的用量账（判据「用量前后连续」）。
    pub accounting: UsageAccounting,
}

/// 压缩前后的用量账。判据：**超阈值真的发生压缩，用量前后连续**。
///
/// 三条恒等式（本模块的构造保证，单测逐条钉住）：
/// 1. `before == retained + summarized`：压缩前的可见历史被**完整划分**成
///    「保留」与「被摘要替代」两块 —— 既不凭空丢，也不重复计；
/// 2. `after == retained + summary + continuation`：压缩后的可见历史
///    = 保留 + 摘要自身 + 续跑提示（续跑提示是 goose 附带的一条常量，
///    `context_mgmt/mod.rs:160-164`；若不计它，就是「保留总量 + 摘要自身用量」）；
/// 3. 计费口径见 [`billable_total_after`]：`压缩前累计 + 摘要自身用量 = 压缩后累计`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageAccounting {
    /// 压缩前 agent 可见历史的总量。
    pub before_tokens: usize,
    /// 被摘要替代（归档后不再 agent 可见）的那部分。
    pub summarized_tokens: usize,
    /// 原样保留的那部分（保留的 user 消息副本）。
    pub retained_tokens: usize,
    /// 摘要消息自身的量（进历史的那条，即结构化渲染后的大小；goose 的
    /// `CompactionResult.retained_context_tokens` 就是把这类消息再数一遍，
    /// `context_mgmt/mod.rs:56`、`:194-200`）。
    pub summary_tokens: usize,
    /// 续跑提示消息的量。
    pub continuation_tokens: usize,
    /// 压缩后 agent 可见历史的总量。
    pub after_tokens: usize,
}

impl UsageAccounting {
    /// 由四个实测数字推出另外两个，并在 debug 下自检恒等式 1、2。
    ///
    /// 前提：注入的分词器是**可加的**（`count_chat_tokens` == 逐条 `count_message_tokens` 之和），
    /// [`TokenEstimator`] 的默认实现保证这一点。
    fn new(
        before_tokens: usize,
        retained_tokens: usize,
        summary_tokens: usize,
        continuation_tokens: usize,
    ) -> Self {
        let summarized_tokens = before_tokens.saturating_sub(retained_tokens);
        let after_tokens = retained_tokens + summary_tokens + continuation_tokens;
        debug_assert_eq!(
            retained_tokens + summarized_tokens,
            before_tokens,
            "用量账：before 必须被 retained/summarized 完整划分"
        );
        Self {
            before_tokens,
            summarized_tokens,
            retained_tokens,
            summary_tokens,
            continuation_tokens,
            after_tokens,
        }
    }
}

/// 压缩的结果：要么原样不动，要么真的压了。
///
/// **差异**：goose 没有这个枚举——阈值判定（`check_if_compaction_needed`）与执行
/// （`compact_messages`）是两个函数，调用方自己决定要不要调。quill 把它们合成一次
/// 调用，是为了让「未超阈值 → 原样返回、且**不碰模型**」这条能在一个测试里钉死
/// （goose 的 `check_if_compaction_needed` 返回 `false` 时也确实不会调模型，
/// `context_mgmt/mod.rs:224-274`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionOutcome {
    /// 未超阈值：历史原样返回，模型一次都没调。
    Unchanged { history: Vec<CompactionMessage> },
    /// 超阈值：真的压缩了（Box 只是为了不把枚举撑大）。
    Compacted(Box<CompactionResult>),
}

/// 一次压缩的全过程：先判阈值，超了才摘要，然后给出压缩后的形状与用量账。
///
/// `threshold_tokens` 是 quill 口径的**绝对**阈值（[`should_compact_over_tokens`]）。
/// **本函数不读配置**：`compaction_threshold_tokens` 的值由壳侧（Q018）读出来传入。
pub async fn compact(
    model: &dyn CompactionModel,
    estimator: &dyn TokenEstimator,
    history: &[CompactionMessage],
    threshold_tokens: usize,
    mode: CompactionMode,
) -> Result<CompactionOutcome, CompactionError> {
    let before_tokens = estimator.count_chat_tokens("", history);
    if !should_compact_over_tokens(before_tokens, threshold_tokens) {
        // 判据：未超阈值不许「顺手压一下」——这里直接原样返回，模型调用在这里
        // 之后才可能发生（测试用「调用次数 == 0」钉住）。
        return Ok(CompactionOutcome::Unchanged {
            history: history.to_vec(),
        });
    }

    let summary = summarize(model, estimator, history).await?;

    // 保留最近一条纯文本 user 消息（goose `context_mgmt/mod.rs:96-129` 的反向查找 +
    // `:169-172` 的追加副本）；手动压缩不保留（`:96-129` 的 `!manual_compact` 分支）。
    //
    // **差异**：goose 的 `has_text_only`（`:81-93`）要求「有文本块且无工具内容」，
    // 本模块的消息模型分不出「没有文本块」与「空文本」，故把空文本按「没有文本」
    // 处理（跳过）。turn-context 事件（`:122`）没有对应概念，跳过。
    let preserved = match mode {
        CompactionMode::Manual => None,
        CompactionMode::Auto => history.iter().enumerate().rev().find(|(_, message)| {
            message.role == MessageRole::User
                && !message.has_tool_response
                && !message.text.is_empty()
        }),
    };

    // 续跑提示文案（goose `context_mgmt/mod.rs:152-158`）。
    // `is_most_recent` 在 goose 里是「保留的 user 消息之后只剩 turn-context 事件」
    // （`:122`）；本模块简化为「它就在历史末尾」（模块头差异第 6 条）。
    let is_most_recent = preserved
        .map(|(index, _)| index + 1 == history.len())
        .unwrap_or(false);
    let continuation_text = match mode {
        CompactionMode::Manual => MANUAL_COMPACT_CONTINUATION_TEXT,
        CompactionMode::Auto if is_most_recent => CONVERSATION_CONTINUATION_TEXT,
        CompactionMode::Auto => TOOL_LOOP_CONTINUATION_TEXT,
    };

    let summary_message = CompactionMessage::user(summary.text);
    let continuation_message = CompactionMessage::assistant(continuation_text);
    let preserved_copy = preserved.map(|(_, message)| message.clone());

    let retained_tokens = preserved_copy
        .as_ref()
        .map(|message| estimator.count_message_tokens(message))
        .unwrap_or(0);
    let summary_tokens = estimator.count_text_tokens(&summary_message.text);
    let continuation_tokens = estimator.count_text_tokens(&continuation_message.text);

    Ok(CompactionOutcome::Compacted(Box::new(CompactionResult {
        summary: summary_message,
        continuation: continuation_message,
        preserved_user: preserved_copy,
        // goose 把原消息全部留在会话里只翻 agent 可见位（`:142-146`），本模块交回调用方。
        archived: history.to_vec(),
        usage: summary.usage,
        accounting: UsageAccounting::new(
            before_tokens,
            retained_tokens,
            summary_tokens,
            continuation_tokens,
        ),
    })))
}

/// 续跑提示：这次压缩发生在一次普通对话之后（goose `context_mgmt/mod.rs:33-36`）。
const CONVERSATION_CONTINUATION_TEXT: &str =
    "Your context was compacted. The previous message contains a summary of the conversation so far.
Do not mention that you read a summary or that conversation summarization occurred.
Just continue the conversation naturally based on the summarized context.";

/// 续跑提示：这次压缩发生在工具循环中间（goose `context_mgmt/mod.rs:38-41`）。
const TOOL_LOOP_CONTINUATION_TEXT: &str =
    "Your context was compacted. The previous message contains a summary of the conversation so far.
Do not mention that you read a summary or that conversation summarization occurred.
Continue calling tools as necessary to complete the task.";

/// 续跑提示：用户在界面上手动触发压缩（goose `context_mgmt/mod.rs:43-46`）。
const MANUAL_COMPACT_CONTINUATION_TEXT: &str =
    "Your context was compacted at the user's request. The previous message contains a summary of the conversation so far.
Do not mention that you read a summary or that conversation summarization occurred.
Just continue the conversation naturally based on the summarized context.";

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 确定性分词器：一个词一个 token。单测里所有 token 数都由它定义，
    /// 真实分词器（Q018 注入）与本模块的账无关。
    struct WordCounter;

    impl TokenEstimator for WordCounter {
        fn count_text_tokens(&self, text: &str) -> usize {
            text.split_whitespace().count()
        }
    }

    /// 固定响应的假模型，记录每次收到的 system prompt。
    struct FixedModel {
        raw_output: String,
        usage: TokenUsage,
        prompts: Mutex<Vec<String>>,
    }

    impl FixedModel {
        fn new(raw_output: &str, usage: TokenUsage) -> Self {
            Self {
                raw_output: raw_output.to_string(),
                usage,
                prompts: Mutex::new(Vec::new()),
            }
        }

        fn call_count(&self) -> usize {
            self.prompts.lock().unwrap().len()
        }
    }

    impl CompactionModel for FixedModel {
        fn complete<'a>(
            &'a self,
            system: &'a str,
            _request: &'a [CompactionMessage],
        ) -> Pin<Box<dyn Future<Output = Result<GeneratedSummary, CompactionError>> + Send + 'a>>
        {
            self.prompts.lock().unwrap().push(system.to_string());
            let generated = GeneratedSummary {
                raw_output: self.raw_output.clone(),
                usage: self.usage,
            };
            Box::pin(async move { Ok(generated) })
        }
    }

    /// 每次都报「装不下」的假模型，记录每次收到的 system prompt。
    struct OverflowModel {
        prompts: Mutex<Vec<String>>,
    }

    impl OverflowModel {
        fn new() -> Self {
            Self {
                prompts: Mutex::new(Vec::new()),
            }
        }

        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().unwrap().clone()
        }
    }

    impl CompactionModel for OverflowModel {
        fn complete<'a>(
            &'a self,
            system: &'a str,
            _request: &'a [CompactionMessage],
        ) -> Pin<Box<dyn Future<Output = Result<GeneratedSummary, CompactionError>> + Send + 'a>>
        {
            self.prompts.lock().unwrap().push(system.to_string());
            Box::pin(async move {
                Err(CompactionError::ContextLengthExceeded(
                    "Prompt exceeds context limit".to_string(),
                ))
            })
        }
    }

    /// 模型响应：草稿 + 围栏 JSON（与 goose `structured.rs:296-315` 的样例同形）。
    const STRUCTURED_RAW: &str = r#"<analysis>
The user asked to fix a bug in parser.rs.
</analysis>

```json
{
  "user_intent": ["Fix the parser bug"],
  "files": [
    {"path": "src/parser.rs", "summary": "Fixed off-by-one in scan loop", "key_code": "fn scan(&mut self) { .. }"}
  ],
  "pending_tasks": ["Add a regression test"]
}
```"#;

    /// 5 条历史，最后一条是纯文本 user 消息（会被保留）。
    /// 词数：alpha beta gamma=3, delta=1, epsilon=1, zeta eta=2, theta=1 → before=8。
    fn history() -> Vec<CompactionMessage> {
        vec![
            CompactionMessage::user("alpha beta gamma"),
            CompactionMessage::assistant("delta"),
            CompactionMessage::user("epsilon"),
            CompactionMessage::assistant("zeta eta"),
            CompactionMessage::user("theta"),
        ]
    }

    fn reported_usage() -> TokenUsage {
        TokenUsage {
            input_tokens: Some(100),
            output_tokens: Some(50),
            total_tokens: Some(200), // 故意与 input+output 不同，见下面 usage 测试
        }
    }

    fn estimator() -> WordCounter {
        WordCounter
    }

    /// `count_chat_tokens("", messages)` 应该等于逐条计数之和（分词器可加性，
    /// 用量账的成立前提）。
    fn expected_before(messages: &[CompactionMessage]) -> usize {
        messages
            .iter()
            .map(|message| estimator().count_message_tokens(message))
            .sum()
    }

    // ---------------------------------------------------------------
    // 阈值判定
    // ---------------------------------------------------------------

    #[test]
    fn threshold_ratio_matches_goose_strictly_greater_and_disabled_convention() {
        // 800/1000 = 0.8：恰好等于阈值 → 不压（严格大于，goose context_mgmt/mod.rs:271）。
        // 【坏样子】把 `usage_ratio > threshold` 改成 `>=`，这条会红。
        assert!(!should_compact(800, 1000, DEFAULT_COMPACTION_THRESHOLD));
        // 801/1000 = 0.801 > 0.8 → 压。
        assert!(should_compact(801, 1000, DEFAULT_COMPACTION_THRESHOLD));
        // 阈值 0/≥1 = 关闭（goose context_mgmt/mod.rs:268-269）。
        // 【坏样子】删掉 `threshold <= 0.0 || threshold >= 1.0` 这条关闭分支，
        //   最后两行会红（0.0 时任何占用都会「该压」）。
        assert!(!should_compact(usize::MAX, 1000, 0.0));
        assert!(!should_compact(usize::MAX, 1000, -1.0));
        assert!(!should_compact(usize::MAX, 1000, 1.0));
        assert!(!should_compact(usize::MAX, 1000, 2.0));
        // 无历史：0/1000 = 0 < 0.8 → 不压。
        assert!(!should_compact(0, 1000, DEFAULT_COMPACTION_THRESHOLD));
    }

    #[test]
    fn token_threshold_boundary_is_strictly_greater() {
        // 【坏样子】把 `current_tokens > threshold_tokens` 改成 `>=`，第一条会红。
        assert!(!should_compact_over_tokens(8000, 8000)); // 恰好相等 → 不压
        assert!(should_compact_over_tokens(8001, 8000));
        assert!(!should_compact_over_tokens(0, 8000));
    }

    // ---------------------------------------------------------------
    // 未超阈值：原样返回、不碰模型 / 边界
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn below_threshold_returns_history_unchanged_without_calling_model() {
        let model = FixedModel::new(STRUCTURED_RAW, reported_usage());
        let history = history();

        // 恰好相等（8 == 8）与远低于（8 < 100）两种都不许压。
        for threshold in [8usize, 100] {
            let outcome = compact(
                &model,
                &estimator(),
                &history,
                threshold,
                CompactionMode::Auto,
            )
            .await
            .expect("未超阈值不该失败");
            match outcome {
                CompactionOutcome::Unchanged { history: returned } => {
                    assert_eq!(returned, history, "未超阈值必须原样返回历史");
                }
                CompactionOutcome::Compacted(_) => {
                    panic!("未超阈值却压了（判据不许「顺手压一下」）")
                }
            }
        }

        // 【坏样子】把 compact 里的 `if !should_compact_over_tokens(...)` 去掉（或把
        //   `!` 删掉），上面两次都会走压缩分支、这条断言会红。
        assert_eq!(model.call_count(), 0, "未超阈值时模型一次都不该被调用");
    }

    #[tokio::test]
    async fn empty_history_never_compacts() {
        let model = FixedModel::new(STRUCTURED_RAW, reported_usage());
        let empty: Vec<CompactionMessage> = Vec::new();

        // 阈值 0：0 > 0 = false；阈值 1000 更不用说。
        for threshold in [0usize, 1000] {
            let outcome = compact(
                &model,
                &estimator(),
                &empty,
                threshold,
                CompactionMode::Auto,
            )
            .await
            .expect("空历史不该失败");
            assert_eq!(
                outcome,
                CompactionOutcome::Unchanged {
                    history: empty.clone()
                }
            );
        }
        // 【坏样子】把 `should_compact_over_tokens` 改成 `>=`，阈值 0 这一次会压，
        //   上面 first 断言与这条会红。
        assert_eq!(model.call_count(), 0);
    }

    // ---------------------------------------------------------------
    // 超阈值：真的压了（形状确实变了）
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn above_threshold_compacts_and_changes_visible_shape() {
        let model = FixedModel::new(STRUCTURED_RAW, reported_usage());
        let history = history();
        let threshold = 7; // before = 8 > 7 → 压

        let outcome = compact(
            &model,
            &estimator(),
            &history,
            threshold,
            CompactionMode::Auto,
        )
        .await
        .expect("应当压缩成功");
        assert_eq!(model.call_count(), 1, "超阈值必须且只调一次模型");

        let CompactionOutcome::Compacted(result) = outcome else {
            panic!("超阈值却没压");
        };

        // 形态确实变了：原 5 条全部归档，agent 可见集变成 摘要 + 续跑提示 + 保留副本 = 3 条。
        // 【坏样子】把 compact 末尾的 `Compacted(...)` 改回 `Unchanged { history }`，
        //   上面那条 let-else 会红；把 preserved_copy 去掉，可见条数 3 变 2、会红。
        assert_eq!(result.archived, history, "原历史必须整段归档，不能丢");
        assert_eq!(result.archived.len(), 5);
        let agent_visible = 1 // 摘要
            + 1 // 续跑提示
            + usize::from(result.preserved_user.is_some());
        assert_eq!(agent_visible, 3);
        assert_ne!(
            agent_visible,
            result.archived.len(),
            "压缩前后可见条数必须真的不同"
        );
        assert_eq!(result.preserved_user.as_ref().unwrap().text, "theta");
        assert_eq!(
            result.summary.role,
            MessageRole::User,
            "摘要进历史时是 user 角色（goose summarize.rs:146）"
        );
        assert_eq!(result.continuation.role, MessageRole::Assistant);
        // 保留的是历史末尾那条 user 消息 → 普通对话续跑文案（goose context_mgmt/mod.rs:152-155）。
        assert_eq!(result.continuation.text, CONVERSATION_CONTINUATION_TEXT);

        // 结构化摘要被渲染成 Markdown，而不是原始 JSON/草稿。
        assert!(result.summary.text.starts_with("# Conversation Summary"));
        assert!(result.summary.text.contains("## User Intent"));
        assert!(result.summary.text.contains("- Fix the parser bug"));
        assert!(result.summary.text.contains("### src/parser.rs"));
        assert!(result.summary.text.contains("## Pending Tasks"));
        assert!(!result.summary.text.contains("<analysis>"));
        assert!(!result.summary.text.contains("```json"));
    }

    #[tokio::test]
    async fn compaction_shrinks_visible_context_for_a_long_history() {
        // 用一段「真的长」的历史证明压缩的目的达成：可见上下文确实变小。
        // （形状测试用的 5 条小历史里，续跑提示这条常量在词数口径下就会占大头，
        //   在那里比大小没有意义；压缩本身是结构变化，已由形状测试钉住。）
        let model = FixedModel::new(STRUCTURED_RAW, reported_usage());
        let mut history = Vec::new();
        for index in 0..20 {
            history.push(CompactionMessage::user(format!(
                "tool_payload_{index} line with a longer body of text that costs tokens"
            )));
            history.push(CompactionMessage::assistant(
                "ack and a longer assistant reply body that also costs tokens".to_string(),
            ));
        }

        let CompactionOutcome::Compacted(result) =
            compact(&model, &estimator(), &history, 100, CompactionMode::Auto)
                .await
                .unwrap()
        else {
            panic!("超阈值应当压缩");
        };

        // 【坏样子】把摘要正文从「渲染后的 Markdown」换成原始输出（含草稿区），
        //   或者不压缩直接返回原历史，这条会红。
        assert!(
            result.accounting.after_tokens < result.accounting.before_tokens,
            "压缩后可见上下文必须更小：before={} after={}",
            result.accounting.before_tokens,
            result.accounting.after_tokens
        );
        assert_eq!(result.accounting.before_tokens, expected_before(&history));
    }

    #[tokio::test]
    async fn single_assistant_message_has_no_preserved_user() {
        let model = FixedModel::new(STRUCTURED_RAW, reported_usage());
        let history = vec![CompactionMessage::assistant("alpha beta")]; // before = 2

        let outcome = compact(&model, &estimator(), &history, 1, CompactionMode::Auto)
            .await
            .unwrap();
        let CompactionOutcome::Compacted(result) = outcome else {
            panic!("超阈值应当压缩");
        };

        // 只有一条消息、且不是「纯文本 user」→ 没有可保留的副本。
        // 【坏样子】把 preserved 的查找条件里的 `message.role == MessageRole::User`
        //   去掉，这条会红（assistant 也会被保留）。
        assert!(result.preserved_user.is_none());
        assert_eq!(result.accounting.retained_tokens, 0);
        assert_eq!(result.accounting.summarized_tokens, 2);
        // 没保留 user 消息 → is_most_recent=false → 工具循环续跑文案（goose :157）。
        assert_eq!(result.continuation.text, TOOL_LOOP_CONTINUATION_TEXT);
    }

    #[tokio::test]
    async fn manual_mode_drops_preserved_user_and_uses_manual_continuation() {
        let model = FixedModel::new(STRUCTURED_RAW, reported_usage());
        let history = history();

        let outcome = compact(&model, &estimator(), &history, 7, CompactionMode::Manual)
            .await
            .unwrap();
        let CompactionOutcome::Compacted(result) = outcome else {
            panic!("超阈值应当压缩");
        };

        // 【坏样子】把 manual 分支也走 Auto 的查找，preserved_user 会 Some、这条红。
        assert!(result.preserved_user.is_none());
        assert_eq!(result.continuation.text, MANUAL_COMPACT_CONTINUATION_TEXT);
        assert_eq!(result.accounting.retained_tokens, 0);
        assert_eq!(result.accounting.summarized_tokens, 8);
    }

    // ---------------------------------------------------------------
    // 用量连续
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn accounting_partitions_before_into_retained_and_summarized() {
        let model = FixedModel::new(STRUCTURED_RAW, reported_usage());
        let history = history();

        let CompactionOutcome::Compacted(result) =
            compact(&model, &estimator(), &history, 7, CompactionMode::Auto)
                .await
                .unwrap()
        else {
            panic!("超阈值应当压缩");
        };

        let accounting = result.accounting;
        // 具体数字：before = 8，保留 "theta" = 1，被摘要替代 = 7。
        assert_eq!(accounting.before_tokens, 8);
        assert_eq!(accounting.retained_tokens, 1);
        assert_eq!(accounting.summarized_tokens, 7);
        // 恒等式 1：before 被完整划分，没丢也没翻倍。
        // 【坏样子】把 `summarized = before - retained` 改成 `summarized = 0`（丢了）或
        //   `summarized = before`（翻倍），这条会红。
        assert_eq!(
            accounting.retained_tokens + accounting.summarized_tokens,
            accounting.before_tokens
        );
    }

    #[tokio::test]
    async fn accounting_visible_after_is_retained_plus_summary_plus_continuation() {
        let model = FixedModel::new(STRUCTURED_RAW, reported_usage());
        let history = history();

        let CompactionOutcome::Compacted(result) =
            compact(&model, &estimator(), &history, 7, CompactionMode::Auto)
                .await
                .unwrap()
        else {
            panic!("超阈值应当压缩");
        };

        let accounting = result.accounting;
        // 三项都得是真数（摘要与续跑提示都不是 0 条空话）。
        assert!(accounting.summary_tokens > 0);
        assert!(accounting.continuation_tokens > 0);
        // 恒等式 2：压缩后可见总量 = 保留 + 摘要自身 + 续跑提示。
        // 【坏样子】把 after 改成 `before + summary`（翻倍）或只写 `summary`（丢了），
        //   这条会红；把 summary_tokens 直接写成 usage.output_tokens 而 usage 是
        //   provider 报的原始输出（比渲染摘要大），这条也会红 —— 因为三项的实测口径变了。
        assert_eq!(
            accounting.after_tokens,
            accounting.retained_tokens + accounting.summary_tokens + accounting.continuation_tokens
        );
        // 判据原文口径的直读：不计续跑提示时，「保留总量 + 摘要自身用量 = 压缩后可见总量」。
        assert_eq!(
            accounting.retained_tokens + accounting.summary_tokens,
            accounting.after_tokens - accounting.continuation_tokens
        );
    }

    #[tokio::test]
    async fn billable_usage_is_continuous_across_compaction() {
        // provider 报了 total=200，但 input=100/output=50：goose 的 ensure_usage_tokens
        // 会重算 total = input + output（summarize.rs:116-118），本模块照抄。
        let model = FixedModel::new(STRUCTURED_RAW, reported_usage());
        let history = history();

        let CompactionOutcome::Compacted(result) =
            compact(&model, &estimator(), &history, 7, CompactionMode::Auto)
                .await
                .unwrap()
        else {
            panic!("超阈值应当压缩");
        };

        assert_eq!(result.usage.input_tokens, Some(100));
        assert_eq!(result.usage.output_tokens, Some(50));
        // 【坏样子】把 ensure_usage_tokens 里的 `total = input + output` 删掉，
        //   这条会红（会保留 provider 报的 200）。
        assert_eq!(result.usage.total_tokens, Some(150));
        assert_eq!(result.usage.total(), 150);

        // 计费口径连续性：压缩前的累计用量 + 摘要调用的用量 = 压缩后的累计用量。
        // 【坏样子】把 billable_total_after 改成 `usage.output_tokens`（丢了 input）
        //   或 `2 * usage.total()`（翻倍），这条会红。
        let billed_before = 12_345;
        assert_eq!(
            billable_total_after(billed_before, &result.usage),
            billed_before + 150
        );

        // provider 什么都没报：用注入的分词器补齐，账仍然连续、且三项都非 0。
        let silent = FixedModel::new(STRUCTURED_RAW, TokenUsage::default());
        let CompactionOutcome::Compacted(result) =
            compact(&silent, &estimator(), &history, 7, CompactionMode::Auto)
                .await
                .unwrap()
        else {
            panic!("超阈值应当压缩");
        };
        let usage = result.usage;
        assert!(usage.input_tokens.unwrap() > 0, "input 必须被估出来");
        assert!(usage.output_tokens.unwrap() > 0, "output 必须被估出来");
        // 【坏样子】把 ensure_usage_tokens 里补 total 的那一行删掉，total 会是 None、
        //   这条红（用量凭空消失）。
        assert_eq!(usage.total_tokens, Some(usage.total()));
        assert_eq!(
            usage.total(),
            usage.input_tokens.unwrap() + usage.output_tokens.unwrap()
        );
        assert!(billable_total_after(1000, &usage) > 1000);
    }

    // ---------------------------------------------------------------
    // 摘要阶梯：溢出时的逐级删除
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn summarizer_overflow_retries_with_progressively_fewer_tool_responses() {
        let model = OverflowModel::new();
        let mut history = vec![CompactionMessage::user("start")];
        for index in 0..4 {
            history.push(
                CompactionMessage::user(format!("tool_payload_{index}")).with_tool_response(),
            );
            history.push(CompactionMessage::assistant(format!("ack {index}")));
        }

        let error = summarize(&model, &estimator(), &history).await.unwrap_err();
        let prompts = model.prompts();

        // 5 档全试过（goose REMOVAL_PERCENTAGES，summarize.rs:14）。
        // 【坏样子】把重试阶梯去掉、第一次错误就返回，这条会红（会变成 1 次）。
        assert_eq!(prompts.len(), REMOVAL_PERCENTAGES.len());

        // 每次重试都真的少了一批工具响应：4 → 3 → 3 → 2 → 0。
        // 【坏样子】把 `filter_tool_responses` 改成恒返回全部消息，最后一次的 0 会变 4，这条红。
        let markers: Vec<usize> = prompts
            .iter()
            .map(|prompt| prompt.matches("tool_payload_").count())
            .collect();
        assert_eq!(markers[0], 4, "第一次必须带全部工具响应");
        assert_eq!(*markers.last().unwrap(), 0, "最后一次必须删光工具响应");
        assert!(
            markers.windows(2).all(|pair| pair[1] <= pair[0]),
            "删除量必须逐级不减：{markers:?}"
        );
        assert!(markers.windows(2).any(|pair| pair[1] < pair[0]));

        // 删光仍溢出 → goose 的第三条文案（summarize.rs:171-173）。
        assert_eq!(error, CompactionError::ContextExceededAfterRemoval);
        assert!(error
            .to_string()
            .contains("context limit exceeded even after removing all tool responses"));
    }

    #[tokio::test]
    async fn summarizer_overflow_without_tool_responses_fails_fast() {
        let model = OverflowModel::new();
        let history = vec![
            CompactionMessage::user("oversized conversation"),
            CompactionMessage::assistant("still oversized"),
        ];

        let error = summarize(&model, &estimator(), &history).await.unwrap_err();

        // 【坏样子】把 `if !has_tool_responses` 这个快速失败分支删掉，调用次数会变 5，这条红。
        assert_eq!(model.prompts().len(), 1, "没有可删的东西就不该重试");
        assert_eq!(error, CompactionError::NoRemovableContext);
        let message = error.to_string();
        assert!(message.contains("there are no tool responses to remove"));
        assert!(!message.contains("even after removing all tool responses"));
        assert!(message.contains("larger usable context"));
        assert!(message.contains("start a new session"));
    }

    // ---------------------------------------------------------------
    // 结构化摘要：解析与渲染
    // ---------------------------------------------------------------

    #[test]
    fn parses_fenced_json_after_analysis_and_renders_markdown() {
        // 【坏样子】把 `json_candidates` 里 `fenced_json_blocks(...)` 那一项去掉（不试围栏），
        //   或把 `render` 的章节标题改掉，这条会红。
        let summary = StructuredSummary::parse(STRUCTURED_RAW).expect("应当解析出来");
        assert_eq!(summary.user_intent, vec!["Fix the parser bug"]);
        assert_eq!(summary.files.len(), 1);
        assert_eq!(summary.files[0].path, "src/parser.rs");
        assert_eq!(summary.pending_tasks, vec!["Add a regression test"]);

        let rendered = summary.render();
        assert!(rendered.contains("# Conversation Summary"));
        assert!(rendered.contains("## User Intent"));
        assert!(rendered.contains("- Fix the parser bug"));
        assert!(rendered.contains("### src/parser.rs"));
        assert!(rendered.contains("```\nfn scan(&mut self) { .. }\n```"));
        assert!(rendered.contains("## Pending Tasks"));
        assert!(!rendered.contains("```json"));
        assert!(!rendered.contains("<analysis>"));
    }

    #[test]
    fn non_summary_responses_fall_back_to_raw_text() {
        // 【坏样子】把 `json_candidates` 里「候选必须包含其后所有终止符」这条过滤
        //   （`filter(|candidate| candidate.matches(TERMINATOR).count() == later_terminators)`）
        //   删掉，草稿区里的围栏示例会被当成摘要，最后两个用例会红。
        // 用例照 goose `structured.rs:332-357` 的 `unusable_responses_fall_back_to_raw_text`
        // 挑选（省略了与本模块消息模型无关的几条）。
        for text in [
            // 纯散文，没有 JSON 文档
            "Here is a summary of the conversation. The user asked about compaction.",
            // 没有可见内容
            "{}",
            r#"{"notes": "unknown fields alone are not a summary"}"#,
            r#"{"current_work": ""}"#,
            r#"{"files": [{}], "user_intent": [" "]}"#,
            // 输出在 JSON 中途被截断：不修复（goose structured.rs:209-214）
            "```json\n{\"user_intent\": [\"Fix the bug\"], \"pending_tasks\": [\"Write tests\", \"Update docs",
            // 散文里引用的 JSON 不贴标记，不算数
            r#"The session focused on the parser migration. The tracker entry {"current_work": "migrate parser"} is unchanged."#,
            // 草稿区里被围栏包住的示例不是摘要
            "<analysis>\nThe target shape is:\n```json\n{\"user_intent\": [\"example only\"]}\n```\nNow let me review.\n</analysis>\nSorry, I ran out of room.",
            // 草稿区内部引用终止符时，同样不能把示例漏出来
            "<analysis>\nThe prompt ends with </analysis> and shows the shape:\n```json\n{\"user_intent\": [\"example only\"]}\n```\nNow let me review.\n</analysis>\nSorry, I ran out of room.",
        ] {
            assert_eq!(
                apply_structured_summary(text),
                text,
                "必须无损回退为原始文本：{text}"
            );
        }
    }

    #[test]
    fn lenient_shapes_are_stringified_not_rejected() {
        // 照 goose `structured.rs:397-422` 的 `lenient_shapes_are_stringified_not_rejected`。
        // 【坏样子】让 from_value 在「字段不是字符串」时丢弃整个摘要（返回默认值），
        //   下面的断言会红。
        let text = r#"{
            "user_intent": "fix the flaky test",
            "errors_and_fixes": [
                {"error": "cursor drifted after replay batch 34", "fix": "bounded mpsc channel"},
                "plain string entry",
                null
            ],
            "pending_tasks": [42],
            "current_work": {"task": "regression test", "status": "in progress"}
        }"#;
        let summary = StructuredSummary::parse(text).expect("宽松解析应当成功");
        assert_eq!(summary.user_intent, vec!["fix the flaky test"]);
        assert_eq!(
            summary.errors_and_fixes,
            vec![
                "error: cursor drifted after replay batch 34; fix: bounded mpsc channel",
                "plain string entry",
            ]
        );
        assert_eq!(summary.pending_tasks, vec!["42"]);
        // 注意键序：goose 的 serde_json 开了 preserve_order，期望 "task: ...; status: ..."
        // （插入序）；quill-core 的 serde_json 没开，是字典序，故 "status" 在前。
        // 差异已写在 `stringify_lenient` 的文档里；字段取舍与内容一致。
        assert_eq!(
            summary.current_work.as_deref(),
            Some("status: in progress; task: regression test")
        );

        // files 里混字符串也照收（照 goose `structured.rs:425-442`）。
        let files = r#"{"files": [
            "src/parser.rs",
            {"path": "tests/parser.rs", "summary": "Added regression test"},
            {"path": "src/scan.rs", "summary": 42, "key_code": ["fn a() {}", "fn b() {}"]},
            ""
        ]}"#;
        let summary = StructuredSummary::parse(files).expect("宽松解析应当成功");
        assert_eq!(summary.files.len(), 3);
        assert_eq!(summary.files[0].path, "src/parser.rs");
        assert_eq!(summary.files[0].summary, "");
        assert_eq!(summary.files[2].summary, "42");
        assert_eq!(
            summary.files[2].key_code.as_deref(),
            Some("fn a() {}; fn b() {}")
        );
    }

    #[test]
    fn render_fences_exceed_backtick_runs_in_key_code() {
        // 照 goose `structured.rs:464-483` 的 `render_fences_exceed_backtick_runs_in_key_code`：
        // key_code 里最长的反引号串是 4 个，围栏就必须是 5 个。
        // 【坏样子】把 `code_fence` 的 `(longest_run + 1).max(3)` 改成 `.max(3)`（不加一），
        //   围栏会变成 4 个，这条会红。
        let summary = StructuredSummary {
            files: vec![FileActivity {
                path: "docs/build.md".to_string(),
                summary: "Documented the build".to_string(),
                key_code: Some(
                    "```bash\ncargo build\n```\n````\nnested fence docs\n````".to_string(),
                ),
            }],
            errors_and_fixes: vec!["None".to_string()],
            ..Default::default()
        };
        let rendered = summary.render();
        assert_eq!(rendered.matches("\n`````\n").count(), 2);
        let errors_heading = rendered.find("## Errors + Fixes").unwrap();
        let closing_fence = rendered.rfind("\n`````\n").unwrap();
        assert!(errors_heading > closing_fence);
    }

    #[test]
    fn formats_messages_like_goose() {
        assert_eq!(
            format_message_for_compacting(&CompactionMessage::user("hello")),
            "[user]: hello"
        );
        assert_eq!(
            format_message_for_compacting(&CompactionMessage::assistant("world")),
            "[assistant]: world"
        );
        // 【坏样子】把空文本分支去掉，这条会红（goose format.rs:78-82 的 <empty message>）。
        assert_eq!(
            format_message_for_compacting(&CompactionMessage::user("")),
            "[user]: <empty message>"
        );
    }
}
