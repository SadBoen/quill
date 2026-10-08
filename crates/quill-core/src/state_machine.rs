//! 对话状态机（queue Q020）—— goose 的「状态 + 转移」**纯逻辑**移植。
//!
//! 本模块没有 I/O、没有 provider、没有工具、没有会话存储：它只说清**一轮对话
//! 有哪些状态、状态之间怎么走、哪些走法非法**。把状态机接到真实执行（异步效果、
//! provider 往返、工具调用、会话读写）是 Q012 的壳侧接线，见
//! `docs/ARCHITECTURE.md` §3.1/§3.3。
//!
//! # 抄了 goose 的哪一块
//!
//! goose 的「状态」不是写死的 enum，而是一条**有序、可重入的流水线**
//! （`vendor/goose/crates/goose-agent/src/machine.rs:1-6`）：每个 step 要么
//! `NotApplicable`（交给下一个），要么 `Applied`（落一串效果，然后**重新从会话
//! 装载再跑一遍**，`machine.rs:79-176`）。状态是从**会话尾部形态**推导出来的——
//! 每个 op 用 `operation.rs` 里的纯函数判断自己在当前会话上适不适用。
//!
//! 所以本模块的做法是：**先照抄推导规则，再把推导出来的状态枚举化**。
//! 逐块出处（下表 `file:line` 均相对仓库根）：
//!
//! | 来源 | 抄了什么 |
//! |---|---|
//! | `goose-agent/src/machine.rs:79-176` | 执行协议：`NotApplicable` 顺延 / `Applied` 后重跑 / `yield_to_client` 后退出 |
//! | `goose-agent/src/operation.rs:25-76` | 判定纯函数：`messages_since_kickoff`、`trailing_error`、`last_effective_role`、`assistant_turn_count`、`ends_turn` |
//! | `goose-agent/src/inference.rs:239-257` | `llm` 何时适用：尾部有效角色是 User/Tool（`should_infer`） |
//! | `goose/src/agents/agent.rs:1690-1768` | step 注册顺序 = 转移优先级：`entry_hook, slash_command, steer, max_turns, bang_shell, compaction, tool_pair_compaction, tool_approval, doctor, project, skills, recipe, tool_execution, unknown_tool, retry, stop_hook, exit_on_error, status, llm` |
//! | `goose/src/agents/state_machine/ops_*.rs` | 各状态的适用条件与收尾方式（下表逐条引用） |
//! | `goose/src/agents/agent.rs:1652-1768`, `agent.rs:2012` | 状态机**每轮新建**（`create_state_machine` 在每次 reply 里调用）→ 本模块的 `TurnMachine` 也是**单轮**的 |
//! | `goose/src/agents/state_machine/effects.rs:8-17` / `session.rs:39-152` | 效果的类型与落库都在壳侧 → 本模块只产出「下一个状态」，不产出效果 |
//!
//! # 状态对照表（goose 依据 → quill 枚举）
//!
//! | quill `ConversationState` | goose 的判定（file:line） | goose 里下一步跑哪个 op |
//! |---|---|---|
//! | `NoKickoff` | `messages_since_kickoff` 直接报错（`operation.rs:25-36`） | 无（正常路径由壳保证先落库用户消息，`session.rs:175`） |
//! | `AwaitingInference` | `should_infer`：尾部有效角色是 User/Tool（`inference.rs:248-257`、`inference.rs:239-246`） | `llm` |
//! | `AwaitingToolExecution` | `pending_advertised_tool_requests` 非空（`ops_toolcalling.rs:678-685`、`ops_toolcalling.rs:838-861`） | `tool_approval` → `skills`/`recipe`/`tool_execution`/`unknown_tool` |
//! | `AwaitingApproval` | 待办请求被标 `executable=false` 且已发 `ActionRequired`（`ops_tool_approval.rs:113-133`、`ops_tool_approval.rs:234-250`；`tool_execution` 对未决请求跳过，`ops_toolcalling.rs:659-666`） | 等客户端；`tool_approval` |
//! | `Compacting` | 尾部 `ContextLengthExceeded`（`ops_compaction.rs:264-279`） | `compaction` |
//! | `TurnEnded` | `ends_turn`：助手尾、无错误、无 `ToolRequest`/`ActionRequired`（`operation.rs:65-76`） | `retry` → `stop_hook`（然后 `run()` 退出，`machine.rs:161-176`） |
//! | `MaxTurnsReached` | `assistant_turn_count >= max_turns`（`ops_maxturns.rs:53-67`） | `max_turns`（在 `llm` 之前拦下） |
//! | `Errored` | `trailing_error(conversation).is_some()`（`operation.rs:38-40`） | `exit_on_error` |
//! | `Cancelled` | 取消令牌（`machine.rs:85-87`、`machine.rs:116-135`；`inference.rs:340-357` 取消后补收尾消息） | `Operation::cancel`（`operation.rs:95-103`） |
//!
//! # 事件对照表
//!
//! | quill `TurnEvent` | 壳侧什么时候发 | goose 里等价的动作 |
//! |---|---|---|
//! | `UserInput` | 用户消息已落库、本轮开始 | `Agent::reply` 先 `add_message` 再 `run_goose`（`session.rs:175`） |
//! | `ModelText` | `llm` 返回了正文、**没有**工具请求 | `InferenceRunner::infer` 的 `applied(usage_effects)`（`inference.rs:547-549`） |
//! | `ModelToolRequest` | `llm` 返回里带工具请求（正文与工具同帧时也算这条） | 同上；判定见 `ends_turn` 对 `ToolRequest` 的排除（`operation.rs:65-76`） |
//! | `ToolResults` | 工具应答消息落库 | `ToolExecutionOperation` 的 `applied`（`ops_toolcalling.rs:1020-1024`） |
//! | `ApprovalRequested` | 待办请求需要用户确认，`ActionRequired` 已落库 | `ops_tool_approval.rs:125-133` |
//! | `ApprovalDecided{granted}` | 用户决定已落库 | `tool_confirmation.rs:75-117` |
//! | `ContextError` | 尾部错误是 `ContextLengthExceeded` | `trailing_error` + `MessageErrorKind`（`operation.rs:38-40`、`ops_compaction.rs:264-267`） |
//! | `Error` | 尾部是别的错误消息 | `exit_on_error`（`ops_exit_on_error.rs:21-32`） |
//! | `Compacted` | 压缩结果落库 | `GooseEffect::CompactConversation`（`effects.rs:10-13`、`ops_compaction.rs:342-354`） |
//! | `Cancelled` | 取消令牌触发 | `machine.rs:116-135`、`inference.rs:340-357` |
//!
//! # 转移表（`TurnMachine::apply` / `transition` 的全部合法走法）
//!
//! | 起点 × 事件 | 终点 | goose 依据 |
//! |---|---|---|
//! | `NoKickoff` × `UserInput` | 预算够 → `AwaitingInference`；`max_turns == 0` → `MaxTurnsReached` | `session.rs:175`；`ops_maxturns.rs:53-67` |
//! | `AwaitingInference` × `ModelText` | 还有预算 → `TurnEnded`；没有 → `MaxTurnsReached` | `operation.rs:65-76`；`ops_maxturns.rs:53-67` |
//! | `AwaitingInference` × `ModelToolRequest` | 还有预算 → `AwaitingToolExecution`；没有 → `MaxTurnsReached` | `ops_toolcalling.rs:678-685`；`ops_maxturns.rs:53-67` |
//! | `AwaitingInference` × `ContextError` | 前 3 次 → `Compacting`；第 4 次起 → `Errored` | `ops_compaction.rs:264-279`（`context_errors > MAX_CONTEXT_ERROR_COMPACTIONS` → 落到 `exit_on_error`） |
//! | `AwaitingInference` × `Error` | 还有预算 → `Errored`；没有 → `MaxTurnsReached` | `operation.rs:38-40`；`ops_exit_on_error.rs:21-32` |
//! | `AwaitingToolExecution` × `ToolResults` | `AwaitingInference` | 工具尾 → `ends_with_provider_turn`（`inference.rs:239-257`） |
//! | `AwaitingToolExecution` × `ApprovalRequested` | `AwaitingApproval` | `ops_tool_approval.rs:113-133`；`ops_toolcalling.rs:659-666` |
//! | `AwaitingApproval` × `ApprovalDecided{granted:true}` | `AwaitingToolExecution` | `ops_tool_approval.rs:63-82`；`ops_toolcalling.rs:659-660` |
//! | `AwaitingApproval` × `ApprovalDecided{granted:false}` | `AwaitingToolExecution`（`Decline` 也要有应答） | `ops_toolcalling.rs:936-944`、`ops_toolcalling.rs:179-184` |
//! | `Compacting` × `Compacted` | `AwaitingInference` | `effects.rs:10-13`；`machine.rs:161-176` 重跑 |
//! | `Compacting` × `Error` | `TurnEnded`（压缩失败改为发正文收尾） | `ops_compaction.rs:356-364` |
//! | 任意非终态 × `Cancelled` | `Cancelled` | `machine.rs:85-87/116-135` |
//! | 任意终态 × 任意事件 | `Err(TransitionError::Terminal)` | `machine.rs:161-176`（`yield_to_client` 后 `run()` 退出；新一轮由新的 `StateMachine` 跑，`agent.rs:1652`/`agent.rs:2012`） |
//!
//! 「还有预算」= `assistant_turn_count < max_turns`（`ops_maxturns.rs:60`）。
//! 模型侧产物（正文、工具请求、错误消息）落地时都先过 `max_turns` 闸门，
//! 因为 `max_turns` 在 `llm` **之前**执行（`agent.rs:1690-1768`），而
//! `assistant_turn_count` 数的是 assistant 消息块、不看内容（`operation.rs:49-63`）。
//!
//! # 没搬的 ops（逐条，含原因）
//!
//! 下面每一块都**没有**搬进本模块。缺口列在这里，让「哪些状态上的动作还没铺」一眼可见。
//!
//! - `ops_llm.rs:187-554`（`InferenceRunner`）、`ops_llm.rs:23-31`（`GooseInferenceProvider`）、
//!   `inference_preparation.rs:19-85`（请求准备）：provider 往返。需要 provider/模型配置/
//!   prompt manager（Q012 的端口）。
//! - `ops_toolcalling.rs:315-1026`（`ToolExecutionOperation`）：工具执行与 hook 生命周期。
//!   需要 `ExtensionManager`、工具运行时、取消令牌（Q012）。本模块只保留它对状态图的影响：
//!   `AwaitingToolExecution` → `ToolResults`。
//! - `ops_tool_approval.rs:26-155`（`ToolApprovalOperation`）：权限审批与安全检查。
//!   需要工具检查管理器 + 权限管理器（quill 的权限在 `quill-control`，多租户口径，Q027）。
//!   本模块只保留 `AwaitingApproval` 这个状态与它的进出口。
//! - `ops_unknown_tool.rs:26-198`：未注册工具改写为错误应答。需要工具清单 + hook 管理器。
//! - `ops_compaction.rs:81-367`：压缩执行（含主动阈值入口 `ops_compaction.rs:283-296`）。
//!   需要 provider 与 `context_mgmt`（quill 的 `compaction.rs` 由另一条任务搬）。
//!   本模块只保留**反应式**入口（尾部 `ContextLengthExceeded`）与次数闸门。
//! - `ops_tool_pair_compaction.rs:21-167`：旧工具对摘成摘要。需要 provider 摘要调用与 cutoff 计算。
//! - `ops_steer.rs:22-78`：运行中追加指令。需要 `SteerQueue`（`ops_steer.rs:20`）与 hook 管理器；
//!   它只在「轮次之间」追加一条用户消息（`ops_steer.rs:48-53`），不产生新状态（Q023）。
//! - `ops_retry.rs:30-285`：goal/grind nudge 与校验脚本重试。需要子进程校验、recipe、
//!   goal/grind 状态。注意它会在 `ends_turn` 之后**把轮次重新打开**（追加一条隐藏用户消息，
//!   `ops_retry.rs:196-205`）→ 本模块把 `TurnEnded` 当本轮终态，不表达这条回边。
//! - `ops_stop_hook.rs:47-120`：Stop hook 允许/阻止收尾。需要 `HookManager`；阻止收尾时同样
//!   重新打开轮次（`ops_stop_hook.rs:112-116`），同上未表达。
//! - `ops_entry_hook.rs:13-77`：`SessionStart`/`UserPromptSubmit` 钩子。需要 hook 管理器；
//!   只做副作用、不改状态（`ops_entry_hook.rs:75` 固定返回 `NotApplicable`）。
//! - `ops_slash_command.rs:15-84`（含 `ops_status.rs`、`ops_doctor.rs`、`ops_recipe.rs`、
//!   `ops_skills.rs` 的命令分支）：斜杠命令拦截 kickoff。需要命令注册表、extension manager、
//!   recipe/final output、skills 目录、doctor 子系统；是 kickoff 上的**拦截分支**，没搬。
//! - `ops_bang_shell.rs:26-75`：`!cmd` 直接改写为 shell 工具请求。需要 `Emitter` 与工具名约定；
//!   拦截后等价于 `AwaitingToolExecution`，但改写本身没搬。
//! - `ops_project.rs:11-39`：把项目说明拼进 prompt parts。需要 `session.project_id` 与 sources。
//! - `session.rs:22-152`：会话装载与效果落库（`SessionManager`）。需要会话存储（Q012/Q030）。
//! - `effects.rs:8-51`：`GooseEffect` 及各效果的落库分支。效果执行在壳侧
//!   （`session.rs:39-152`）；本模块只产出「下一个状态」，不产出效果。
//! - `tool_confirmation.rs:27-117`：确认请求/决定的持久化。需要 `SessionManager`；
//!   本模块用事件 `ApprovalDecided` 表达。
//! - `usage.rs`：token 用量记账。需要 provider 与会话存储（Q026）。
//! - `machine.rs:56-207` 的其余执行件：`Step`/`StateMachine` 的异步调度、取消令牌、
//!   `InferenceInput` 组装与工具名去重。这些是**执行层**，本模块只在「调度规则」层面
//!   对齐（`NotApplicable` 顺延 / `Applied` 后重跑 / yield 后退出）。
//!
//! # 本轮没做的（别误读）
//!
//! - **未接线**：状态机还没接进 `quill-server/src/api_chat.rs` 的真实对话路径。
//!   那条路径现在仍是过程式代码（局部变量 + 消息表 `status` 列），跑的还是它自己的
//!   `MAX_TOOL_ROUNDS` 循环；接线属于 Q012/Q018，本模块**不改** `api_chat`。
//! - **未验证**：纯逻辑经过了单元测试（本文件 `tests`），但没有在真实对话里跑过；
//!   `TurnEvent` 的发送时机由壳侧决定，壳还没写。
//! - 未表达的 goose 行为：retry/stop-hook 的「重新打开轮次」回边、steer、斜杠命令与
//!   `!` 命令的 kickoff 拦截、主动阈值压缩（见上面「没搬的 ops」）。
//!
//! # quill 侧的现状（为什么需要这个模块）
//!
//! `quill-server/src/api_chat.rs:1033-1072` 的工具往返循环里，状态只以
//! `reply.tool_calls.is_empty()`、`rounds`、`reply.answer().is_empty()` 这些局部量存在，
//! 会话状态则散在消息表的 `status` 列上（`api_chat.rs:821`、`api_chat.rs:1207` 写
//! `"complete"`）——既不可枚举也不可测。本模块先把 goose 的状态与转移固定下来。

use std::fmt;

/// goose 的 `max_turns` 收尾语（逐字抄自 `ops_maxturns.rs:14`）。
pub const MAX_TURNS_MESSAGE: &str = "I've reached the maximum number of actions I can do without user input. Would you like me to continue?";

/// goose 的同一个常量（`ops_compaction.rs:28`）：反应式压缩的次数闸门。
/// 判定是 `context_errors > MAX_CONTEXT_ERROR_COMPACTIONS`（`ops_compaction.rs:269-279`），
/// 所以前 3 次上下文错误都能触发压缩，第 4 次落到 `exit_on_error`。
pub const MAX_CONTEXT_ERROR_COMPACTIONS: u32 = 2;

/// 一轮对话里会话所处的位置。
///
/// 每个变体都能对上一个 goose 的判定函数 / op 名（见模块文档的对照表）；
/// `goose_owner()` 返回在这些状态上跑的 op 名（`Operation::name()`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConversationState {
    /// 还没有 kickoff 消息。goose 的 `messages_since_kickoff` 在这里直接报错
    /// （`operation.rs:25-36`），正常路径由壳先落库用户消息（`session.rs:175`）。
    NoKickoff,
    /// 尾部有效角色是 User/Tool，轮到模型：goose 的 `should_infer`
    /// （`inference.rs:248-257`），下一步是 `llm`。
    AwaitingInference,
    /// 尾部是助手消息且带**未应答**的工具请求：goose 的
    /// `pending_advertised_tool_requests`（`ops_toolcalling.rs:678-685`）。
    AwaitingToolExecution,
    /// 工具请求被标成「需确认」且确认已发出、还没决定：`ActionRequired` 已落库
    /// （`ops_tool_approval.rs:113-133`），`tool_execution` 跳过未决请求
    /// （`ops_toolcalling.rs:659-666`）——等客户端。
    AwaitingApproval,
    /// 正在压缩上下文：尾部是 `ContextLengthExceeded` 错误
    /// （`ops_compaction.rs:264-279`）。
    Compacting,
    /// 助手回合正常结束：`ends_turn`（`operation.rs:65-76`）。
    TurnEnded,
    /// 助手回合数用尽：`assistant_turn_count >= max_turns`（`ops_maxturns.rs:53-67`），
    /// 收尾语是 [`MAX_TURNS_MESSAGE`]。
    MaxTurnsReached,
    /// 尾部是错误消息：`trailing_error`（`operation.rs:38-40`）+ `exit_on_error`
    /// （`ops_exit_on_error.rs:21-32`）。
    Errored,
    /// 取消令牌触发（`machine.rs:85-87`、`inference.rs:340-357`）。
    Cancelled,
}

impl ConversationState {
    /// 全部状态，给「枚举要完整」的测试用。
    pub const ALL: [ConversationState; 9] = [
        ConversationState::NoKickoff,
        ConversationState::AwaitingInference,
        ConversationState::AwaitingToolExecution,
        ConversationState::AwaitingApproval,
        ConversationState::Compacting,
        ConversationState::TurnEnded,
        ConversationState::MaxTurnsReached,
        ConversationState::Errored,
        ConversationState::Cancelled,
    ];

    /// 本轮的终态：`run()` 已经（或将）退出。终态不再接受任何事件，
    /// 新一轮要新建 `TurnMachine`（goose 每轮新建状态机，`agent.rs:1652`）。
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            ConversationState::TurnEnded
                | ConversationState::MaxTurnsReached
                | ConversationState::Errored
                | ConversationState::Cancelled
        )
    }

    /// 这些状态上 goose 会跑哪些 op（`Operation::name()` 的原字符串，
    /// 见 `agent.rs:1690-1768` 的注册顺序）。
    pub const fn goose_owner(self) -> &'static [&'static str] {
        match self {
            ConversationState::NoKickoff => &[],
            ConversationState::AwaitingInference => &["llm"],
            ConversationState::AwaitingToolExecution => &[
                "tool_approval",
                "skills",
                "recipe",
                "tool_execution",
                "unknown_tool",
            ],
            ConversationState::AwaitingApproval => &["tool_approval"],
            ConversationState::Compacting => &["compaction"],
            ConversationState::TurnEnded => &["retry", "stop_hook"],
            ConversationState::MaxTurnsReached => &["max_turns"],
            ConversationState::Errored => &["exit_on_error"],
            ConversationState::Cancelled => &["cancel"],
        }
    }

    /// 这个状态只等什么事件（写进非法转移的报错里）。
    pub const fn expects(self) -> &'static str {
        match self {
            ConversationState::NoKickoff => "用户消息（开启一轮）",
            ConversationState::AwaitingInference => "模型应答（正文或工具请求）或上下文错误",
            ConversationState::AwaitingToolExecution => "工具应答，或「该请求需要确认」",
            ConversationState::AwaitingApproval => "用户对确认的决定",
            ConversationState::Compacting => "压缩结果",
            ConversationState::TurnEnded
            | ConversationState::MaxTurnsReached
            | ConversationState::Errored
            | ConversationState::Cancelled => "什么都不等——本轮已经结束",
        }
    }
}

impl fmt::Display for ConversationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ConversationState::NoKickoff => "NoKickoff",
            ConversationState::AwaitingInference => "AwaitingInference",
            ConversationState::AwaitingToolExecution => "AwaitingToolExecution",
            ConversationState::AwaitingApproval => "AwaitingApproval",
            ConversationState::Compacting => "Compacting",
            ConversationState::TurnEnded => "TurnEnded",
            ConversationState::MaxTurnsReached => "MaxTurnsReached",
            ConversationState::Errored => "Errored",
            ConversationState::Cancelled => "Cancelled",
        };
        f.write_str(name)
    }
}

/// 推进状态机的外部事件。发送时机见模块文档的「事件对照表」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnEvent {
    /// 用户消息已落库、本轮开始。
    UserInput,
    /// 模型返回正文、没有工具请求。
    ModelText,
    /// 模型返回里带工具请求（正文与工具同帧时也算这条）。
    ModelToolRequest,
    /// 工具应答消息已落库。
    ToolResults,
    /// 待办工具请求需要用户确认，`ActionRequired` 已落库。
    ApprovalRequested,
    /// 用户对确认做出了决定。
    ApprovalDecided { granted: bool },
    /// 尾部错误是上下文超限（`ContextLengthExceeded`）。
    ContextError,
    /// 尾部是别的错误消息。
    Error,
    /// 压缩结果已落库。
    Compacted,
    /// 取消令牌触发。
    Cancelled,
}

impl TurnEvent {
    /// 全部事件（`ApprovalDecided` 的两种取值各算一条），给「终态拒绝一切」的测试用。
    pub const ALL: [TurnEvent; 11] = [
        TurnEvent::UserInput,
        TurnEvent::ModelText,
        TurnEvent::ModelToolRequest,
        TurnEvent::ToolResults,
        TurnEvent::ApprovalRequested,
        TurnEvent::ApprovalDecided { granted: true },
        TurnEvent::ApprovalDecided { granted: false },
        TurnEvent::ContextError,
        TurnEvent::Error,
        TurnEvent::Compacted,
        TurnEvent::Cancelled,
    ];
}

impl fmt::Display for TurnEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TurnEvent::UserInput => f.write_str("UserInput"),
            TurnEvent::ModelText => f.write_str("ModelText"),
            TurnEvent::ModelToolRequest => f.write_str("ModelToolRequest"),
            TurnEvent::ToolResults => f.write_str("ToolResults"),
            TurnEvent::ApprovalRequested => f.write_str("ApprovalRequested"),
            TurnEvent::ApprovalDecided { granted } => {
                write!(f, "ApprovalDecided(granted={granted})")
            }
            TurnEvent::ContextError => f.write_str("ContextError"),
            TurnEvent::Error => f.write_str("Error"),
            TurnEvent::Compacted => f.write_str("Compacted"),
            TurnEvent::Cancelled => f.write_str("Cancelled"),
        }
    }
}

/// 状态机的计数与预算。
///
/// goose 从**会话内容**重算这些数（`assistant_turn_count`，`operation.rs:49-63`；
/// 隐藏的上下文错误条数，`ops_compaction.rs:269-279`）。纯逻辑里没有会话，
/// 所以由壳/测试把它们维护在这里；语义与 goose 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnContext {
    /// 本轮已产生的 assistant 消息块数（一次推理算一块，`operation.rs:49-63`）。
    pub assistant_turns: u32,
    /// 轮次上限（goose 的 `max_turns`，`ops_maxturns.rs:16-19`）。
    pub max_turns: u32,
    /// 已经因上下文超限而压缩掉的错误条数（goose 数的是
    /// `agent_visible=false` 的 `ContextLengthExceeded` 消息，`ops_compaction.rs:269-277`）。
    pub hidden_context_errors: u32,
}

impl TurnContext {
    /// 新的一轮：计数归零，只带预算。
    pub const fn new(max_turns: u32) -> Self {
        Self {
            assistant_turns: 0,
            max_turns,
            hidden_context_errors: 0,
        }
    }

    /// goose 的闸门取反：`!(assistant_turn_count < max_turns)`
    /// （`ops_maxturns.rs:60`）。`max_turns == 0` 时立即为真（一次推理都不给）。
    pub const fn budget_exhausted(&self) -> bool {
        self.assistant_turns >= self.max_turns
    }

    /// 一次模型侧产物落地（正文 / 工具请求 / 错误消息都算一个 assistant 块）。
    fn after_model_output(self) -> Self {
        Self {
            assistant_turns: self.assistant_turns.saturating_add(1),
            ..self
        }
    }

    /// 一次反应式压缩成功，隐藏掉的那条上下文错误计入闸门。
    fn after_reactive_compaction(self) -> Self {
        Self {
            hidden_context_errors: self.hidden_context_errors.saturating_add(1),
            ..self
        }
    }
}

/// 一条被接受的转移，带 goose 出处，便于测试与排查时核对。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub from: ConversationState,
    pub event: TurnEvent,
    pub to: ConversationState,
    /// 转移**之后**的计数。
    pub context: TurnContext,
    /// 这条转移的依据（goose 的 `file:line` 与判定函数/op 名）。
    pub goose_rule: &'static str,
}

/// 非法转移。每一种都带得出原因，不静默通过。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// 还没有 kickoff：goose 的 `messages_since_kickoff` 会直接报错
    /// （`operation.rs:25-36`）。
    NoKickoff { event: TurnEvent },
    /// 事件与当前状态不匹配：该状态下 goose 的流水线不会消费这个事件。
    Unexpected {
        state: ConversationState,
        event: TurnEvent,
    },
    /// 本轮已结束：goose 的 `run()` 在 `yield_to_client` 后退出
    /// （`machine.rs:161-176`），新一轮要新建状态机（`agent.rs:1652`）。
    Terminal {
        state: ConversationState,
        event: TurnEvent,
    },
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransitionError::NoKickoff { event } => write!(
                f,
                "会话还没有 kickoff：goose 的 messages_since_kickoff 在这里直接报错\
                 （operation.rs:25-36），状态机不接受 {event}；先落库用户消息再驱动本轮"
            ),
            TransitionError::Unexpected { state, event } => write!(
                f,
                "状态 {state} 不接受事件 {event}：该状态只等 {}，\
                 由 goose 的 {} 处理（agent.rs:1690-1768）",
                state.expects(),
                if state.goose_owner().is_empty() {
                    "（无）".to_string()
                } else {
                    state.goose_owner().join("/")
                }
            ),
            TransitionError::Terminal { state, event } => write!(
                f,
                "状态 {state} 是本轮终态，不再接受任何输入（收到 {event}）：\
                 goose 的 run() 在 yield_to_client 后退出（machine.rs:161-176），\
                 新一轮由新的状态机跑（agent.rs:1652）"
            ),
        }
    }
}

impl std::error::Error for TransitionError {}

/// 纯转移函数：`(当前状态, 事件, 计数) → 下一个状态 + 新计数`。
///
/// 非法转移返回 [`TransitionError`]，不静默通过。合法走法见模块文档的转移表。
pub fn transition(
    from: ConversationState,
    event: TurnEvent,
    context: TurnContext,
) -> Result<Transition, TransitionError> {
    if from.is_terminal() {
        return Err(TransitionError::Terminal { state: from, event });
    }

    let step = |to: ConversationState, context: TurnContext, goose_rule: &'static str| Transition {
        from,
        event,
        to,
        context,
        goose_rule,
    };

    match (from, event) {
        // 用户消息落库后流水线第一次装载；max_turns 在 llm 之前（agent.rs:1690-1768）。
        (ConversationState::NoKickoff, TurnEvent::UserInput) => {
            if context.budget_exhausted() {
                Ok(step(
                    ConversationState::MaxTurnsReached,
                    context,
                    "ops_maxturns.rs:53-67；agent.rs:1690-1768（max_turns 在 llm 之前）",
                ))
            } else {
                Ok(step(
                    ConversationState::AwaitingInference,
                    context,
                    "session.rs:175 先装载会话；inference.rs:248-257 should_infer",
                ))
            }
        }

        // 模型侧产物都先过 max_turns 闸门，再看各自的收尾。
        (ConversationState::AwaitingInference, TurnEvent::ModelText) => {
            let context = context.after_model_output();
            if context.budget_exhausted() {
                Ok(step(
                    ConversationState::MaxTurnsReached,
                    context,
                    "ops_maxturns.rs:53-67（assistant_turn_count >= max_turns 时先拦截）",
                ))
            } else {
                Ok(step(
                    ConversationState::TurnEnded,
                    context,
                    "operation.rs:65-76 ends_turn：助手尾、无错误、无 ToolRequest/ActionRequired",
                ))
            }
        }
        (ConversationState::AwaitingInference, TurnEvent::ModelToolRequest) => {
            let context = context.after_model_output();
            if context.budget_exhausted() {
                Ok(step(
                    ConversationState::MaxTurnsReached,
                    context,
                    "ops_maxturns.rs:53-67（第 max_turns 块 assistant 消息落地即收尾）",
                ))
            } else {
                Ok(step(
                    ConversationState::AwaitingToolExecution,
                    context,
                    "ops_toolcalling.rs:678-685/838-861 pending_advertised_tool_requests",
                ))
            }
        }
        (ConversationState::AwaitingInference, TurnEvent::ContextError) => {
            if context.hidden_context_errors > MAX_CONTEXT_ERROR_COMPACTIONS {
                Ok(step(
                    ConversationState::Errored,
                    context,
                    "ops_compaction.rs:269-279（超过次数闸门 → 落到 exit_on_error）",
                ))
            } else {
                Ok(step(
                    ConversationState::Compacting,
                    context,
                    "ops_compaction.rs:264-279 尾部 ContextLengthExceeded → 反应式压缩",
                ))
            }
        }
        (ConversationState::AwaitingInference, TurnEvent::Error) => {
            let context = context.after_model_output();
            if context.budget_exhausted() {
                // 错误消息也是 assistant 块，同样会被 max_turns 闸门拦下。
                Ok(step(
                    ConversationState::MaxTurnsReached,
                    context,
                    "operation.rs:49-63（错误消息也是 assistant 块）+ ops_maxturns.rs:53-67",
                ))
            } else {
                Ok(step(
                    ConversationState::Errored,
                    context,
                    "operation.rs:38-40 trailing_error + ops_exit_on_error.rs:21-32",
                ))
            }
        }

        // 工具应答落库后尾部有效角色是 Tool → 回到模型。
        (ConversationState::AwaitingToolExecution, TurnEvent::ToolResults) => Ok(step(
            ConversationState::AwaitingInference,
            context,
            "conversation.rs:645-654 EffectiveRole::Tool + inference.rs:239-257 ends_with_provider_turn",
        )),
        (ConversationState::AwaitingToolExecution, TurnEvent::ApprovalRequested) => Ok(step(
            ConversationState::AwaitingApproval,
            context,
            "ops_tool_approval.rs:113-133 发 ActionRequired；ops_toolcalling.rs:659-666 未决请求被跳过",
        )),

        // 确认决定：允许 → 去执行；拒绝 → 也要有一条应答（Decline），同样由 tool_execution 落库。
        (
            ConversationState::AwaitingApproval,
            TurnEvent::ApprovalDecided { granted },
        ) => Ok(step(
            ConversationState::AwaitingToolExecution,
            context,
            if granted {
                "ops_tool_approval.rs:63-82 写入 executable=true；ops_toolcalling.rs:659-660 Execute"
            } else {
                "ops_toolcalling.rs:936-944/179-184 Decline 也由 tool_execution 产出应答"
            },
        )),

        // 压缩：成功回推理；失败改为发正文收尾（ops_compaction.rs:356-364）。
        (ConversationState::Compacting, TurnEvent::Compacted) => Ok(step(
            ConversationState::AwaitingInference,
            context.after_reactive_compaction(),
            "effects.rs:10-13 CompactConversation 落库；machine.rs:161-176 重跑",
        )),
        (ConversationState::Compacting, TurnEvent::Error) => Ok(step(
            ConversationState::TurnEnded,
            context,
            "ops_compaction.rs:356-364 压缩失败：发一条助手正文并 yield",
        )),

        // 取消不挑状态；取消后本轮结束。
        (_, TurnEvent::Cancelled) => Ok(step(
            ConversationState::Cancelled,
            context,
            "machine.rs:85-87/116-135 取消令牌；inference.rs:340-357 取消后补收尾消息",
        )),

        (ConversationState::NoKickoff, _) => Err(TransitionError::NoKickoff { event }),
        _ => Err(TransitionError::Unexpected {
            state: from,
            event,
        }),
    }
}

/// 单轮状态机：`NoKickoff` 起步，`apply` 逐个事件推进，终态后拒绝一切输入。
///
/// goose 的状态机**每轮新建**（`create_state_machine`，`agent.rs:1652`/`agent.rs:2012`），
/// 所以这里不做「跨轮」复用：一轮结束就丢掉，下一轮 `TurnMachine::new` 重来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMachine {
    state: ConversationState,
    context: TurnContext,
    history: Vec<Transition>,
}

impl TurnMachine {
    /// 新建一轮状态机，只设预算；计数从零开始。
    pub const fn new(max_turns: u32) -> Self {
        Self {
            state: ConversationState::NoKickoff,
            context: TurnContext::new(max_turns),
            history: Vec::new(),
        }
    }

    /// 当前状态。
    pub const fn state(&self) -> ConversationState {
        self.state
    }

    /// 当前计数/预算。
    pub const fn context(&self) -> TurnContext {
        self.context
    }

    /// 本轮已接受的转移（含每条的依据），按发生顺序。
    pub fn history(&self) -> &[Transition] {
        &self.history
    }

    /// 本轮是否已经结束。
    pub const fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// 推进一格；非法转移返回 [`TransitionError`]，状态与计数保持原样。
    pub fn apply(&mut self, event: TurnEvent) -> Result<ConversationState, TransitionError> {
        let next = transition(self.state, event, self.context)?;
        self.state = next.to;
        self.context = next.context;
        self.history.push(next);
        Ok(self.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 正常一轮：用户输入 → 模型推理 → 工具调用 → 回到推理 → 完成。
    ///
    /// 把「`AwaitingInference` × `ModelToolRequest` → `AwaitingToolExecution`」改坏
    /// （比如直接写成 `TurnEnded`），这条会红在第 3 步的状态断言上；
    /// 把「`AwaitingToolExecution` × `ToolResults` → `AwaitingInference`」改坏，
    /// 会红在第 4 步。
    #[test]
    fn one_round_walks_user_inference_tools_inference_done() {
        let mut machine = TurnMachine::new(8);

        assert_eq!(machine.state(), ConversationState::NoKickoff);
        assert_eq!(
            machine.apply(TurnEvent::UserInput).unwrap(),
            ConversationState::AwaitingInference
        );

        assert_eq!(
            machine.apply(TurnEvent::ModelToolRequest).unwrap(),
            ConversationState::AwaitingToolExecution
        );
        assert_eq!(
            machine.context().assistant_turns,
            1,
            "一次推理算一个 assistant 块"
        );

        assert_eq!(
            machine.apply(TurnEvent::ToolResults).unwrap(),
            ConversationState::AwaitingInference
        );

        assert_eq!(
            machine.apply(TurnEvent::ModelText).unwrap(),
            ConversationState::TurnEnded
        );
        assert!(machine.is_terminal(), "TurnEnded 是本轮终态");
        assert_eq!(machine.history().len(), 4);
        assert_eq!(
            machine.history().last().unwrap().event,
            TurnEvent::ModelText
        );
    }

    /// 工具循环超预算：第 `max_turns` 块 assistant 消息落地即被拦下，收尾语是 goose 的原文。
    ///
    /// 把「模型产物落地先过 max_turns 闸门」这层去掉（即 `after_model_output` 后不查
    /// `budget_exhausted`），这条会红在最后一轮的断言上（会停在 `AwaitingToolExecution`
    /// 而不是 `MaxTurnsReached`）。
    #[test]
    fn tool_loop_stops_when_the_turn_budget_is_used_up() {
        let max_turns = 3;
        let mut machine = TurnMachine::new(max_turns);
        machine.apply(TurnEvent::UserInput).unwrap();

        for round in 1..max_turns {
            assert_eq!(
                machine.apply(TurnEvent::ModelToolRequest).unwrap(),
                ConversationState::AwaitingToolExecution,
                "第 {round} 轮还有预算，应该去执行工具"
            );
            assert_eq!(
                machine.apply(TurnEvent::ToolResults).unwrap(),
                ConversationState::AwaitingInference,
                "第 {round} 轮工具应答后回到模型"
            );
        }

        // 第 max_turns 块 assistant 消息：goose 的 max_turns 先于 llm 收尾。
        assert_eq!(
            machine.apply(TurnEvent::ModelToolRequest).unwrap(),
            ConversationState::MaxTurnsReached
        );
        assert_eq!(machine.context().assistant_turns, max_turns);
        assert!(machine.is_terminal());
        assert!(
            MAX_TURNS_MESSAGE.starts_with("I've reached the maximum number of actions"),
            "收尾语逐字抄自 ops_maxturns.rs:14"
        );
    }

    /// 预算为 0：goose 的 `assistant_turn_count < max_turns` 一开始就不成立
    /// （`ops_maxturns.rs:60`），一次推理都不给。
    ///
    /// 把 `TurnContext::budget_exhausted` 的 `>=` 改成 `>`，这条会红。
    #[test]
    fn zero_budget_ends_the_turn_before_any_inference() {
        let mut machine = TurnMachine::new(0);
        assert_eq!(
            machine.apply(TurnEvent::UserInput).unwrap(),
            ConversationState::MaxTurnsReached
        );
    }

    /// 反应式压缩的次数闸门：前 3 次上下文错误压缩，第 4 次落到 `exit_on_error`。
    ///
    /// 把 `hidden_context_errors > MAX_CONTEXT_ERROR_COMPACTIONS` 的严格大于改成 `>=`，
    /// 这条会红在第 3 次压缩的断言上（会提前进 `Errored`）。
    #[test]
    fn context_errors_compact_three_times_then_error_out() {
        let mut machine = TurnMachine::new(8);
        machine.apply(TurnEvent::UserInput).unwrap();

        for attempt in 1..=3 {
            assert_eq!(
                machine.apply(TurnEvent::ContextError).unwrap(),
                ConversationState::Compacting,
                "第 {attempt} 次上下文错误应该先压缩"
            );
            assert_eq!(
                machine.apply(TurnEvent::Compacted).unwrap(),
                ConversationState::AwaitingInference,
                "压缩成功后回到推理"
            );
        }
        assert_eq!(machine.context().hidden_context_errors, 3);

        assert_eq!(
            machine.apply(TurnEvent::ContextError).unwrap(),
            ConversationState::Errored,
            "闸门用尽后不再压缩，落到 exit_on_error"
        );
    }

    /// 确认往返：允许与拒绝都回到「待执行」，由 tool_execution 落不同处置
    /// （`ops_toolcalling.rs:659-670`）。
    ///
    /// 把 `AwaitingToolExecution` × `ApprovalRequested` 这条去掉，会红在 `AwaitingApproval`
    /// 的断言上；把 `AwaitingApproval` × `ApprovalDecided` 的两种取值分开处理成不同目标，
    /// 会红在拒绝那半段。
    #[test]
    fn approval_round_trip_returns_to_tool_execution() {
        let mut machine = TurnMachine::new(8);
        machine.apply(TurnEvent::UserInput).unwrap();
        machine.apply(TurnEvent::ModelToolRequest).unwrap();

        machine.apply(TurnEvent::ApprovalRequested).unwrap();
        assert_eq!(machine.state(), ConversationState::AwaitingApproval);
        assert_eq!(
            machine
                .apply(TurnEvent::ApprovalDecided { granted: true })
                .unwrap(),
            ConversationState::AwaitingToolExecution
        );
        machine.apply(TurnEvent::ToolResults).unwrap();

        machine.apply(TurnEvent::ModelToolRequest).unwrap();
        machine.apply(TurnEvent::ApprovalRequested).unwrap();
        assert_eq!(
            machine
                .apply(TurnEvent::ApprovalDecided { granted: false })
                .unwrap(),
            ConversationState::AwaitingToolExecution,
            "拒绝也要经 tool_execution 产出 Decline 应答"
        );
        assert_eq!(
            machine.apply(TurnEvent::ToolResults).unwrap(),
            ConversationState::AwaitingInference
        );
    }

    /// 非法转移必须带原因被拒，不能静默通过。
    ///
    /// 把 `transition` 的兜底 `_ => Err(Unexpected)` 改成「返回当前状态」，
    /// 这条会红在第一个用例上。
    #[test]
    fn illegal_transitions_are_rejected_with_a_reason() {
        let ctx = TurnContext::new(8);

        // 没有待办工具请求，工具应答无处安放（ops_toolcalling.rs:840-842 返回 NotApplicable）。
        let err = transition(
            ConversationState::AwaitingInference,
            TurnEvent::ToolResults,
            ctx,
        )
        .unwrap_err();
        assert!(matches!(err, TransitionError::Unexpected { .. }));
        let text = err.to_string();
        assert!(
            text.contains("AwaitingInference") && text.contains("ToolResults"),
            "{text}"
        );
        assert!(
            text.contains("llm"),
            "报错要点出该状态的 goose 处理者：{text}"
        );

        // 工具请求还没应答，模型不会再被调用（inference.rs:248-257）。
        let err = transition(
            ConversationState::AwaitingToolExecution,
            TurnEvent::ModelText,
            ctx,
        )
        .unwrap_err();
        assert!(matches!(err, TransitionError::Unexpected { .. }));
        assert!(
            err.to_string().contains("tool_execution"),
            "报错要给出这个状态上的 op：{err}"
        );

        // 等确认时不会冒出新的工具请求。
        let err = transition(
            ConversationState::AwaitingApproval,
            TurnEvent::ModelToolRequest,
            ctx,
        )
        .unwrap_err();
        assert!(matches!(err, TransitionError::Unexpected { .. }));

        // 压缩中没有工具往返。
        let err =
            transition(ConversationState::Compacting, TurnEvent::ToolResults, ctx).unwrap_err();
        assert!(matches!(err, TransitionError::Unexpected { .. }));

        // 没有 kickoff 时除了用户消息与取消，什么都不接。
        let err = transition(ConversationState::NoKickoff, TurnEvent::ModelText, ctx).unwrap_err();
        assert!(matches!(err, TransitionError::NoKickoff { .. }));
        assert!(
            err.to_string().contains("operation.rs:25-36"),
            "报错要标出 goose 出处：{err}"
        );

        // 用户消息不能中途插进来（steer 才在轮次之间追加，ops_steer.rs:48-53）。
        let err = transition(
            ConversationState::AwaitingInference,
            TurnEvent::UserInput,
            ctx,
        )
        .unwrap_err();
        assert!(matches!(err, TransitionError::Unexpected { .. }));
    }

    /// 终态不再接受输入：四个终态 × 全部事件都必须被拒。
    ///
    /// 把 `transition` 开头的 `if from.is_terminal()` 去掉，这条会红。
    #[test]
    fn terminal_states_reject_every_further_input() {
        for state in ConversationState::ALL
            .into_iter()
            .filter(|s| s.is_terminal())
        {
            for event in TurnEvent::ALL {
                let err = transition(state, event, TurnContext::new(8)).unwrap_err();
                assert!(
                    matches!(err, TransitionError::Terminal { .. }),
                    "{state} × {event} 应该是终态拒绝，实际：{err:?}"
                );
                assert!(
                    err.to_string().contains("yield_to_client"),
                    "报错要说清为什么拒绝：{err}"
                );
            }
        }
    }

    /// 对照表本身可核对：每个非 `NoKickoff` 状态都指得上有 goose 出处，
    /// 且每个 op 名都来自 goose 的注册表。
    ///
    /// 往 `ConversationState` 里加一个新状态却忘了在 `goose_owner`/`expects` 里映射，
    /// 这条会红；把某个 op 名写错（比如 "tool_exec"），也会红。
    #[test]
    fn every_state_maps_back_to_goose_operations() {
        const GOOSE_OPERATION_NAMES: [&str; 20] = [
            "entry_hook",
            "slash_command",
            "steer",
            "max_turns",
            "bang_shell",
            "compaction",
            "tool_pair_compaction",
            "tool_approval",
            "doctor",
            "project",
            "skills",
            "recipe",
            "tool_execution",
            "unknown_tool",
            "retry",
            "stop_hook",
            "exit_on_error",
            "status",
            "llm",
            // 取消不是 op 名，是 Operation::cancel（operation.rs:95-103）。
            "cancel",
        ];

        assert_eq!(ConversationState::ALL.len(), 9);
        for state in ConversationState::ALL {
            assert!(!state.expects().is_empty(), "{state} 没有写明它等什么");
            if state == ConversationState::NoKickoff {
                assert!(state.goose_owner().is_empty(), "NoKickoff 上跑不了任何 op");
                continue;
            }
            assert!(
                !state.goose_owner().is_empty(),
                "{state} 指不出 goose 处理者"
            );
            for op in state.goose_owner() {
                assert!(
                    GOOSE_OPERATION_NAMES.contains(op),
                    "{state} 映射到了 goose 里不存在的 op 名：{op}"
                );
            }
        }
    }
}
