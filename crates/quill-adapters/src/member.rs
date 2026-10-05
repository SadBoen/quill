//! `MemberExecutor` —— 成员执行注入点（P0-2）
//!
//! 依据 `docs/07_实例间委派设计.md` §0.1.1（**v2 修正版**，
//! 见 `docs/DECISIONS.md` D-2026-10-05-02）：
//! `start` 返回 `MemberOutcome`（最终结果）而非流式 `MemberHandle`，
//! `probe_capabilities` 已砍掉。
//!
//! # 为什么这个 trait 是 M2 的第一件事（不是"抽象洁癖"）
//!
//! `docs/03_智能体编排设计.md` §2.12.2 的论据 4：
//! 编排层若直接 `Agent::with_config(...)`，成员执行就**内嵌在调用方生命周期内**，
//! 于是 T-A9~T-A24 全套状态机测试**必须起真 provider** → 违反
//! `V1_SCOPE_CONSTRAINTS` §五「测试是唯一防线」。有 trait 才有 mock。
//!
//! # 为什么用原生 `impl Future + Send` 而不是 `#[async_trait]`
//!
//! `async-trait` **不在 `Cargo.lock` 里**，新增依赖须主理人裁决。
//! `rustc 1.99` 已稳定支持 RPITIT，且契约层**零依赖**是硬约束
//! （`scripts/check-crate-deps.sh` G40）。故走原生。
//!
//! ⚠️ **为什么不是裸 `async fn in trait`**（两条硬理由，不是风格偏好）：
//! 1. 裸 `async fn` 触发 `async_fn_in_trait` 警告，而本项目
//!    `clippy -- -D warnings` —— 警告即红灯。
//! 2. 裸 `async fn` **无法声明 `Send`**，而成员 task 要 `tokio::spawn`；
//!    没有 `Send` 就无法在多线程运行时里跑（`quill-server` 是多用户单进程）。
//!
//! ⚠️ **已知代价（如实记录，不是"无所谓"）**：RPITIT 形态的 trait
//! **不是 dyn-compatible**，因此不能写 `Arc<dyn MemberExecutor>`。
//! 当前编排层（`quill-agent`）尚未落地，泛型注入已足够。
//! 若将来需要「按字符串键持有异构 executor 集合」，须改为装箱 future
//! （`Pin<Box<dyn Future + Send>>`）或裁决引入 `async-trait`。
//! **这一点须主理人裁决，我没有自行引入依赖。**

use crate::ids::{ExpertId, MemberId, SessionId, UserId};

// ─────────────────────────── 错误类型 ───────────────────────────

/// 契约层统一错误（`docs/PHASE2_CONTRACT.md` §三，7 个变体）。
///
/// ⚠️ **手写 `Display` 而不用 `thiserror`**：`thiserror` 在 lock 里但不在本 crate 的
/// 依赖表里，新增依赖须裁决。契约层零依赖比"少写 7 行 derive"重要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    /// 未认证。
    Unauthorized(String),
    /// 已认证但无权（对端策略拒绝）。
    Forbidden(String),
    /// 目标不存在（成员已结束 / 专家未注册）。
    NotFound(String),
    /// 状态冲突（成员已中止 / 重复派工）。
    Conflict(String),
    /// 上游 provider 或隧道返回的错误。
    Provider(String),
    /// 存储错误。
    Storage(String),
    /// 内部错误（不变量被破坏）。
    Internal(String),
}

impl AdapterError {
    /// 取出错误的人类可读正文（不含变体前缀）。
    pub fn detail(&self) -> &str {
        match self {
            Self::Unauthorized(s)
            | Self::Forbidden(s)
            | Self::NotFound(s)
            | Self::Conflict(s)
            | Self::Provider(s)
            | Self::Storage(s)
            | Self::Internal(s) => s,
        }
    }

    /// 是否为**可重试**的传输层失败。
    ///
    /// 依据 `docs/07_实例间委派设计.md` §4.3 降级链：
    /// `peer_unreachable` ❌ 不重试；执行中断链 ❌ 不自动重跑（副作用不可逆）；
    /// 但 provider 5xx 可退避重试。
    /// ⚠️ 这里只区分 `Provider` 与其余；**具体错误码由传输层下沉到执行器层**
    /// （`docs/07` §1.2 表第 1 行），契约层不猜。
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Provider(_))
    }
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = match self {
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not found",
            Self::Conflict(_) => "conflict",
            Self::Provider(_) => "provider error",
            Self::Storage(_) => "storage error",
            Self::Internal(_) => "internal",
        };
        write!(f, "{prefix}: {}", self.detail())
    }
}

impl std::error::Error for AdapterError {}

// ─────────────────────────── 支撑类型 ───────────────────────────

/// 中止范围（`docs/03_智能体编排设计.md` §2.6.5 · 两个 scope）。
///
/// **为什么必须是两个而不是一个「停止」按钮**：
/// 契约七.2 要求「停止主持人**不能**连带停成员」，而 PRD §12.5 要求能中止整个房间。
/// 合成一个布尔量会让其中一条契约必然被违反。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbortScope {
    /// 只停本轮，房间保持，成员跑完仍回叫。
    StopRound,
    /// 中止整个房间，所有成员停。
    AbortRoom,
}

impl AbortScope {
    /// 是否波及成员执行。
    pub fn halts_members(&self) -> bool {
        matches!(self, Self::AbortRoom)
    }
}

impl std::fmt::Display for AbortScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StopRound => f.write_str("StopRound"),
            Self::AbortRoom => f.write_str("AbortRoom"),
        }
    }
}

/// 注入成员的消息（`steer` 的载荷）。
///
/// ⚠️ **契约层不 import 上游 `goose` 的 `Message`**：
/// `quill-*` 禁止依赖 `vendor/goose`（`PHASE2_CONTRACT.md` §一）。
/// 因此这里定义**契约自己的**最小消息形态，由适配层负责与上游结构互转。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    role: MessageRole,
    text: String,
}

/// 消息角色（契约最小集）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageRole {
    /// 主持人/用户注入。
    User,
    /// 成员产出（回叫）。
    Assistant,
    /// 工具结果。
    Tool,
}

impl MessageRole {
    /// 契约层线格式（供隧道传输与审计日志用）。
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

impl Message {
    /// 构造消息。**空文本一律判失败**。
    ///
    /// 反向用例的意义：空 `steer` 若被放行，成员会收到一次"什么都没有"的
    /// 唤醒，白烧一轮 token 且在时间线上留下不可解释的空洞。
    pub fn new(role: MessageRole, text: impl Into<String>) -> Result<Self, InvalidMessage> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(InvalidMessage::EmptyText);
        }
        Ok(Self { role, text })
    }

    /// 便捷构造：用户注入。
    pub fn user(text: impl Into<String>) -> Result<Self, InvalidMessage> {
        Self::new(MessageRole::User, text)
    }

    /// 角色。
    pub fn role(&self) -> MessageRole {
        self.role
    }

    /// 正文。
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// 消息构造错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidMessage {
    /// 文本为空或全空白。
    EmptyText,
}

impl std::fmt::Display for InvalidMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyText => f.write_str("消息正文为空或全空白"),
        }
    }
}

impl std::error::Error for InvalidMessage {}

/// 委派链上的一跳（`docs/07_实例间委派设计.md` §2.1 · `chain` 数组）。
///
/// **环防护的载体**：A→B→A 的 B 派 A 必须靠这里的历史被检出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainHop {
    node: String,
    task: String,
}

impl ChainHop {
    /// 构造一跳。节点名与任务号都不得为空。
    pub fn new(node: impl Into<String>, task: impl Into<String>) -> Result<Self, InvalidChainHop> {
        let node = node.into();
        let task = task.into();
        if node.trim().is_empty() {
            return Err(InvalidChainHop::EmptyNode);
        }
        if task.trim().is_empty() {
            return Err(InvalidChainHop::EmptyTask);
        }
        Ok(Self { node, task })
    }

    /// 节点名。
    pub fn node(&self) -> &str {
        &self.node
    }

    /// 任务号。
    pub fn task(&self) -> &str {
        &self.task
    }
}

/// 委派链一跳的构造错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidChainHop {
    /// 节点名为空。
    EmptyNode,
    /// 任务号为空。
    EmptyTask,
}

impl std::fmt::Display for InvalidChainHop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyNode => f.write_str("委派链节点名为空"),
            Self::EmptyTask => f.write_str("委派链任务号为��"),
        }
    }
}

impl std::error::Error for InvalidChainHop {}

/// 成员启动请求（`MemberStartRequest`）。
///
/// ⚠️ **协议里没有凭据字段**（`docs/07` §2.2 P0-9）：
/// 本结构体**故意不含**任何 secret 形态的字段，
/// 因此「凭据离开本进程」在类型层面就无法表达。
///
/// 依据 `docs/07` §0.1.1 表：`chain` 保留（环防护与传输层无关），
/// `required_capabilities` 降为可选（`Option`，`None` = 不要求）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStartRequest {
    owner: UserId,
    session: SessionId,
    expert: ExpertId,
    member: MemberId,
    title: String,
    instructions: String,
    chain: Vec<ChainHop>,
    required_capabilities: Option<Vec<String>>,
}

/// 启动请求的构造错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidStartRequest {
    /// 标题为空或全空白。
    EmptyTitle,
    /// instructions 为空或全空白 —— 自包含约束（`docs/07` §2.1）。
    EmptyInstructions,
    /// 成员标识串了专家：`req.expert != req.member` 的前缀推导。
    MemberExpertMismatch {
        /// 请求里的专家。
        expert: String,
        /// 请求里的成员标识。
        member: String,
    },
}

impl std::fmt::Display for InvalidStartRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyTitle => f.write_str("任务标题为空"),
            Self::EmptyInstructions => f.write_str("任务 instructions 为空：委派任务必须自包含"),
            Self::MemberExpertMismatch { expert, member } => write!(
                f,
                "成员标识 {member:?} 与专家 {expert:?} 不匹配（成员标识须以专家名开头）"
            ),
        }
    }
}

impl std::error::Error for InvalidStartRequest {}

impl MemberStartRequest {
    /// 构造启动请求。
    ///
    /// 不变量检查：
    /// 1. `title` 非空；
    /// 2. `instructions` 非空（自包含）；
    /// 3. `member` 必须是 `expert` 的执行实例（`expert-` 前缀）——
    ///    这一条挡住「把 A 专家的任务派给 B 专家的成员」这类串号。
    pub fn new(
        owner: UserId,
        session: SessionId,
        expert: ExpertId,
        member: MemberId,
        title: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Result<Self, InvalidStartRequest> {
        let title = title.into();
        let instructions = instructions.into();
        if title.trim().is_empty() {
            return Err(InvalidStartRequest::EmptyTitle);
        }
        if instructions.trim().is_empty() {
            return Err(InvalidStartRequest::EmptyInstructions);
        }
        if !member
            .as_str()
            .starts_with(&format!("{}-", expert.as_str()))
        {
            return Err(InvalidStartRequest::MemberExpertMismatch {
                expert: expert.as_str().to_string(),
                member: member.as_str().to_string(),
            });
        }
        Ok(Self {
            owner,
            session,
            expert,
            member,
            title,
            instructions,
            chain: Vec::new(),
            required_capabilities: None,
        })
    }

    /// 追加委派链上游（builder）。
    pub fn with_chain(mut self, hop: ChainHop) -> Self {
        self.chain.push(hop);
        self
    }

    /// 设置所需能力（builder；`None` = 不要求）。
    ///
    /// `docs/07` §0.1.1 表把 `required_capabilities` 降为可选：
    /// 隧道失败即不支持，可先试再探。
    pub fn with_required_capabilities(mut self, caps: Option<Vec<String>>) -> Self {
        self.required_capabilities = caps;
        self
    }

    /// 属主用户。
    pub fn owner(&self) -> UserId {
        self.owner
    }

    /// 所属会话。
    pub fn session(&self) -> SessionId {
        self.session
    }

    /// 专家身份（可复用）。
    pub fn expert(&self) -> &ExpertId {
        &self.expert
    }

    /// 成员实例标识（一次性）。
    pub fn member(&self) -> &MemberId {
        &self.member
    }

    /// 任务标题（UI 可见）。
    pub fn title(&self) -> &str {
        &self.title
    }

    /// 任务正文（自包含）。
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// 委派链上游。
    pub fn chain(&self) -> &[ChainHop] {
        &self.chain
    }

    /// 所需能力。
    pub fn required_capabilities(&self) -> Option<&[String]> {
        self.required_capabilities.as_deref()
    }
}

// ─────────────────────────── 成员结果 ───────────────────────────

/// 成员执行的最终状态（`docs/07` §2.3 · `status` 四值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberStatus {
    /// 全部完成。
    Done,
    /// 部分完成（`completed_scope` 说明做完了什么）。
    Partial,
    /// 失败。
    Failed,
    /// 被取消（用户 `AbortRoom`）。
    Cancelled,
}

impl MemberStatus {
    /// 线格式（隧道协议 + UI 共用）。
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// 是否算「成员成功交付了东西」。
    ///
    /// ⚠️ `Partial` 算成功：`docs/07` §2.3 明确部分成功要带 `completed_scope`，
    /// 主持人需要它做汇总；把 partial 当失败会丢掉已完成的工作。
    pub fn is_deliverable(&self) -> bool {
        matches!(self, Self::Done | Self::Partial)
    }
}

impl std::fmt::Display for MemberStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

/// 成员执行结果（`MemberOutcome`）。
///
/// ⚠️ v2：`start` 直接返回它（请求-响应式），不是"活流 + 独立完成通道"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberOutcome {
    member: MemberId,
    status: MemberStatus,
    completed_scope: String,
    output: String,
}

/// 结果构造错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidOutcome {
    /// `completed_scope` 为空 —— 失败也必须说明"做到哪"（`docs/07` §2.3）。
    EmptyScope,
    /// `Done` / `Partial` 却没产出正文 —— 对外说"完成了"却无内容可汇总。
    MissingOutput,
    /// 产出正文全空白。
    BlankOutput,
}

impl std::fmt::Display for InvalidOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyScope => f.write_str("completed_scope 为空"),
            Self::MissingOutput => f.write_str("状态为 done/partial 但产出正文为空：无法汇总"),
            Self::BlankOutput => f.write_str("产出正文全空白"),
        }
    }
}

impl std::error::Error for InvalidOutcome {}

impl MemberOutcome {
    /// 构造结果，校验状态与产出的一致性。
    pub fn new(
        member: MemberId,
        status: MemberStatus,
        completed_scope: impl Into<String>,
        output: impl Into<String>,
    ) -> Result<Self, InvalidOutcome> {
        let completed_scope = completed_scope.into();
        let output = output.into();
        if completed_scope.trim().is_empty() {
            return Err(InvalidOutcome::EmptyScope);
        }
        if status.is_deliverable() {
            if output.trim().is_empty() {
                return Err(InvalidOutcome::MissingOutput);
            }
        } else if !output.trim().is_empty() {
            // ⚠️ 反向不变量：`failed` / `cancelled` 带产出正文会让主持人
            // 把半截结果当成功内容汇总。这是**真实**的静默错误，故判失败。
            return Err(InvalidOutcome::BlankOutput);
        }
        Ok(Self {
            member,
            status,
            completed_scope,
            output,
        })
    }

    /// 便捷构造：成功。
    pub fn done(member: MemberId, scope: &str, output: &str) -> Result<Self, InvalidOutcome> {
        Self::new(member, MemberStatus::Done, scope, output)
    }

    /// 便捷构造：失败（无产出）。
    pub fn failed(member: MemberId, scope: &str) -> Result<Self, InvalidOutcome> {
        Self::new(member, MemberStatus::Failed, scope, "")
    }

    /// 成员标识。
    pub fn member(&self) -> &MemberId {
        &self.member
    }

    /// 状态。
    pub fn status(&self) -> MemberStatus {
        self.status
    }

    /// 已完成范围（失败时也要填：`docs/07` §2.3）。
    pub fn completed_scope(&self) -> &str {
        &self.completed_scope
    }

    /// 产出正文。
    pub fn output(&self) -> &str {
        &self.output
    }
}

// ─────────────────────────── 环防护 ───────────────────────────

/// 委派链环检测结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainCheck {
    /// 无环，链长 `depth`。
    Ok {
        /// 链长（含本跳）。
        depth: usize,
    },
    /// 检测到环：节点 `node` 在链上出现了第二次。
    Cycle {
        /// 重复出现的节点名。
        node: String,
        /// 该节点在链中首次出现的位置（0 起）。
        first_at: usize,
    },
    /// 链超长。
    TooDeep {
        /// 实际长度。
        depth: usize,
        /// 上限。
        max: usize,
    },
}

/// 委派链最大长度（`docs/07` §4.2.1 `chain_too_deep`）。
///
/// 取 4：`A→B→C→D` 已足够表达"多跳"，再深对家用场景无意义，
/// 而无限深会让分布式死循环难以察觉。
pub const MAX_CHAIN_DEPTH: usize = 4;

/// 检查委派链是否成环 / 超长。
///
/// **新增节点为 `node`**：`chain` 是上游历史，`node` 是即将加入的那一跳。
///
/// 环防护是**分布式问题**，命名空间方案不解决（A→B→A 的 B 派 A）
/// ——`docs/07` §0.0 已把它列为「不可替代的 trait 抽象」之一。
pub fn check_chain(chain: &[ChainHop], node: &str) -> ChainCheck {
    let depth = chain.len() + 1;
    if depth > MAX_CHAIN_DEPTH {
        return ChainCheck::TooDeep {
            depth,
            max: MAX_CHAIN_DEPTH,
        };
    }
    if let Some(first_at) = chain.iter().position(|h| h.node() == node) {
        return ChainCheck::Cycle {
            node: node.to_string(),
            first_at,
        };
    }
    ChainCheck::Ok { depth }
}

// ─────────────────────────── trait 本体 ───────────────────────────

/// 成员执行注入点（P0-2）。
///
/// ⚠️ **v2 三方法**（`docs/DECISIONS.md` D-2026-10-05-02）：
/// - `start` → `MemberOutcome`（**不是**流式 `MemberHandle`）
/// - `probe_capabilities` **已砍掉**（隧道 GET 即可）
///
/// 使用原生 `async fn in trait`（见模块头「为什么不用 async-trait」）。
/// 使用 `-> impl Future + Send` 而非裸 `async fn`（**不用 `async-trait`**）。
///
/// ⚠️ 裸 `async fn in trait` 会触发 `async_fn_in_trait` 警告，
/// 而本项目 `clippy -- -D warnings` —— 警告即红灯，等于没有闸门。
/// 更重要的是：裸 `async fn` **无法表达 `Send`**，而 `MemberExecutor` 的实现方
/// 要 `tokio::spawn` 成员 task，没有 `Send` 就无法在多线程运行时里跑。
/// `-> impl Future<Output=...> + Send` 二者兼得，且仍**零依赖**。
pub trait MemberExecutor: Send + Sync + 'static {
    /// ① 启动成员执行。
    ///
    /// **v2 修正**：返回最终结果，不是"活流 + 独立完成通道"
    /// （隧道是请求-响应式，`docs/07` §0.1.1）。
    fn start(
        &self,
        req: MemberStartRequest,
    ) -> impl std::future::Future<Output = Result<MemberOutcome, AdapterError>> + Send;

    /// ② 注入消息。
    ///
    /// ⚠️ **失败不得导致成员失败**（`docs/07` §0.1.1 保留 G2）：
    /// 隧道 WS 可能已断开，但这不代表成员执行失败——
    /// 调用方必须**如实上报注入失败**而**不中止成员**。
    fn steer(
        &self,
        member: &MemberId,
        m: Message,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send;

    /// ③ 请求中止（本地 cancel task / 远端发 tunnel abort 帧）。
    fn abort(
        &self,
        member: &MemberId,
        scope: AbortScope,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u8) -> UserId {
        UserId::from_bytes([n; 16])
    }

    fn s(n: u8) -> SessionId {
        SessionId::from_bytes([n; 16])
    }

    fn expert(name: &str) -> ExpertId {
        ExpertId::parse(name).expect("测试用专家名应合法")
    }

    fn member(name: &str) -> MemberId {
        MemberId::parse(name).expect("测试用成员标识应合法")
    }

    fn req(expert_name: &str, member_name: &str) -> MemberStartRequest {
        MemberStartRequest::new(
            u(1),
            s(2),
            expert(expert_name),
            member(member_name),
            "分析成本",
            "请分析 Q3 成本结构",
        )
        .expect("测试用启动请求应合法")
    }

    // ── AdapterError ──
    #[test]
    fn adapter_error_display_keeps_variant_prefix_and_detail() {
        let e = AdapterError::NotFound("成员 m-1 不存在".into());
        assert_eq!(e.to_string(), "not found: 成员 m-1 不存在");
        assert_eq!(e.detail(), "成员 m-1 不存在");
    }

    #[test]
    fn adapter_error_variants_have_distinct_wire_prefixes() {
        // 7 个变体两两不同：若两个变体 Display 撞了，UI 会分不清根因。
        let errs = [
            AdapterError::Unauthorized("a".into()),
            AdapterError::Forbidden("a".into()),
            AdapterError::NotFound("a".into()),
            AdapterError::Conflict("a".into()),
            AdapterError::Provider("a".into()),
            AdapterError::Storage("a".into()),
            AdapterError::Internal("a".into()),
        ];
        assert_eq!(errs.len(), 7, "契约 §三 定义了 7 个变体");
        let mut seen = std::collections::BTreeSet::new();
        for e in &errs {
            assert!(seen.insert(e.to_string()), "变体 Display 撞车：{e}");
        }
        assert_eq!(seen.len(), 7, "7 个变体必须产出 7 种不同的 Display");
    }

    #[test]
    fn only_provider_error_is_retryable() {
        assert!(AdapterError::Provider("tunnel 503".into()).is_retryable());
        for e in [
            AdapterError::Unauthorized("x".into()),
            AdapterError::Forbidden("x".into()),
            AdapterError::NotFound("x".into()),
            AdapterError::Conflict("x".into()),
            AdapterError::Storage("x".into()),
            AdapterError::Internal("x".into()),
        ] {
            assert!(!e.is_retryable(), "{e} 不该被当成可重试");
        }
    }

    // ── AbortScope ──
    #[test]
    fn abort_scope_only_room_scope_halts_members() {
        assert!(
            !AbortScope::StopRound.halts_members(),
            "契约七.2：停主持人不得连带停成员"
        );
        assert!(
            AbortScope::AbortRoom.halts_members(),
            "PRD §12.5：AbortRoom 必须波及成员"
        );
    }

    #[test]
    fn abort_scope_display_matches_contract_names() {
        assert_eq!(AbortScope::StopRound.to_string(), "StopRound");
        assert_eq!(AbortScope::AbortRoom.to_string(), "AbortRoom");
    }

    // ── Message ──
    #[test]
    fn message_accepts_non_empty_text_and_keeps_role() {
        let m = Message::user("请补充数据源").expect("非空应合法");
        assert_eq!(m.role(), MessageRole::User);
        assert_eq!(m.text(), "请补充数据源");
        assert_eq!(m.role().as_wire(), "user");
    }

    #[test]
    fn message_rejects_empty_and_whitespace_text() {
        // 反向用例：「有空白但无内容」是最容易被漏掉的一档。
        for bad in ["", " ", "\t", "\n", "  \r\n  "] {
            assert_eq!(
                Message::user(bad).unwrap_err(),
                InvalidMessage::EmptyText,
                "输入 {bad:?} 应被判为空消息"
            );
        }
    }

    // ── ChainHop ──
    #[test]
    fn chain_hop_accepts_valid_pair_and_exposes_both_fields() {
        let h = ChainHop::new("node-a", "t-001").expect("应合法");
        assert_eq!(h.node(), "node-a");
        assert_eq!(h.task(), "t-001");
    }

    #[test]
    fn chain_hop_rejects_empty_node_or_task() {
        assert_eq!(
            ChainHop::new("", "t-1").unwrap_err(),
            InvalidChainHop::EmptyNode
        );
        assert_eq!(
            ChainHop::new("  ", "t-1").unwrap_err(),
            InvalidChainHop::EmptyNode
        );
        assert_eq!(
            ChainHop::new("node-a", "").unwrap_err(),
            InvalidChainHop::EmptyTask
        );
        assert_eq!(
            ChainHop::new("node-a", "  ").unwrap_err(),
            InvalidChainHop::EmptyTask
        );
    }

    // ── MemberStartRequest ──
    #[test]
    fn start_request_keeps_owner_session_expert_and_member_distinct() {
        let r = req("cost-analyst", "cost-analyst-1");
        assert_eq!(r.owner(), u(1));
        assert_eq!(r.session(), s(2));
        assert_eq!(r.expert().as_str(), "cost-analyst");
        assert_eq!(r.member().as_str(), "cost-analyst-1");
        assert_ne!(
            r.owner().to_compact_hex(),
            r.session().to_compact_hex(),
            "已检查：owner 与 session 携带不同的身份值"
        );
        // ⚠️ 这里**不能**写 `assert_ne!(r.owner(), r.session())` ——
        // `UserId` 与 `SessionId` 是不同类型，编译器会直接判错
        // （见 crates/quill-testkit/tests/newtype_guard.rs 的反向自证）。
    }

    #[test]
    fn start_request_rejects_member_of_a_different_expert() {
        // 反向用例：把 cost 专家的任务派给 analyst 的成员实例 → 判失败。
        let err = MemberStartRequest::new(
            u(1),
            s(2),
            expert("cost-analyst"),
            member("growth-analyst-1"),
            "t",
            "i",
        )
        .unwrap_err();
        assert_eq!(
            err,
            InvalidStartRequest::MemberExpertMismatch {
                expert: "cost-analyst".into(),
                member: "growth-analyst-1".into()
            }
        );
    }

    #[test]
    fn start_request_rejects_empty_title_and_instructions() {
        let base = |title: &str, instr: &str| {
            MemberStartRequest::new(
                u(1),
                s(2),
                expert("cost-analyst"),
                member("cost-analyst-1"),
                title,
                instr,
            )
        };
        assert_eq!(base("", "i").unwrap_err(), InvalidStartRequest::EmptyTitle);
        assert_eq!(
            base("   ", "i").unwrap_err(),
            InvalidStartRequest::EmptyTitle
        );
        assert_eq!(
            base("t", "").unwrap_err(),
            InvalidStartRequest::EmptyInstructions
        );
        assert_eq!(
            base("t", "  \n ").unwrap_err(),
            InvalidStartRequest::EmptyInstructions,
            "全空白 instructions 等同无 instructions"
        );
    }

    #[test]
    fn start_request_chain_and_capabilities_are_opt_in() {
        let r = req("cost-analyst", "cost-analyst-1");
        assert!(r.chain().is_empty(), "默认无上游链");
        assert_eq!(
            r.required_capabilities(),
            None,
            "默认不要求能力（v2 降为可选）"
        );

        let r = r
            .with_chain(ChainHop::new("node-a", "t-001").expect("应合法"))
            .with_required_capabilities(Some(vec!["wiki_search".into()]));
        assert_eq!(r.chain().len(), 1, "已检查：链长应为 1");
        assert_eq!(r.chain()[0].node(), "node-a");
        assert_eq!(
            r.required_capabilities(),
            Some(&["wiki_search".to_string()][..])
        );
    }

    // ── MemberOutcome ──
    #[test]
    fn outcome_done_requires_output() {
        let o = MemberOutcome::done(member("m-1"), "完成分析", "结论：降 12%")
            .expect("done + 有产出应合法");
        assert_eq!(o.status(), MemberStatus::Done);
        assert_eq!(o.output(), "结论：降 12%");
        assert_eq!(o.completed_scope(), "完成分析");
        assert_eq!(o.member().as_str(), "m-1");
    }

    #[test]
    fn outcome_deliverable_status_without_output_is_rejected() {
        // 反向用例：对外说"完成"却无内容 → 判失败。
        for status in [MemberStatus::Done, MemberStatus::Partial] {
            assert_eq!(
                MemberOutcome::new(member("m-1"), status, "s", "").unwrap_err(),
                InvalidOutcome::MissingOutput,
                "状态 {status} 无产出必须判红"
            );
            assert_eq!(
                MemberOutcome::new(member("m-1"), status, "s", "   ").unwrap_err(),
                InvalidOutcome::MissingOutput,
                "全空白产出等同无产出（状态 {status}）"
            );
        }
    }

    #[test]
    fn outcome_non_deliverable_status_with_output_is_rejected() {
        // 反向用例：failed 却带产出 → 主持人会把半截结果当成功汇总。
        for status in [MemberStatus::Failed, MemberStatus::Cancelled] {
            assert_eq!(
                MemberOutcome::new(member("m-1"), status, "s", "半截内容").unwrap_err(),
                InvalidOutcome::BlankOutput,
                "状态 {status} 携带产出正文必须判红"
            );
        }
    }

    #[test]
    fn outcome_requires_scope_even_when_failed() {
        // 反向用例：失败也必须说"做到哪"，否则主持人无法汇总。
        for bad in ["", "  "] {
            assert_eq!(
                MemberOutcome::new(member("m-1"), MemberStatus::Failed, bad, "").unwrap_err(),
                InvalidOutcome::EmptyScope
            );
        }
        let o = MemberOutcome::failed(member("m-1"), "只完成了成本结构分析").expect("应合法");
        assert_eq!(o.status(), MemberStatus::Failed);
        assert_eq!(o.output(), "");
    }

    #[test]
    fn member_status_wire_names_are_distinct() {
        let all = [
            MemberStatus::Done,
            MemberStatus::Partial,
            MemberStatus::Failed,
            MemberStatus::Cancelled,
        ];
        let wires: std::collections::BTreeSet<&str> = all.iter().map(|s| s.as_wire()).collect();
        assert_eq!(wires.len(), 4, "4 个状态必须对应 4 个线格式名");
        assert!(wires.contains("done"));
        assert!(wires.contains("partial"));
        assert!(wires.contains("failed"));
        assert!(wires.contains("cancelled"));
    }

    #[test]
    fn partial_counts_as_deliverable_done_failed_cancelled_do_not() {
        assert!(MemberStatus::Done.is_deliverable());
        assert!(
            MemberStatus::Partial.is_deliverable(),
            "部分成功也要能汇总（docs/07 §2.3）"
        );
        assert!(!MemberStatus::Failed.is_deliverable());
        assert!(!MemberStatus::Cancelled.is_deliverable());
    }

    // ── 环防护 ──
    #[test]
    fn empty_chain_first_hop_is_ok_with_depth_one() {
        assert_eq!(
            check_chain(&[], "node-a"),
            ChainCheck::Ok { depth: 1 },
            "链起点必须允许"
        );
    }

    #[test]
    fn chain_detects_a_to_b_to_a_cycle() {
        // A→B→A：B 派 A 时必须被检出。
        let chain = vec![
            ChainHop::new("node-a", "t-001").expect("应合法"),
            ChainHop::new("node-b", "t-002").expect("应合法"),
        ];
        assert_eq!(
            check_chain(&chain, "node-a"),
            ChainCheck::Cycle {
                node: "node-a".into(),
                first_at: 0
            }
        );
    }

    #[test]
    fn chain_detects_self_delegation() {
        let chain = vec![ChainHop::new("node-a", "t-001").expect("应合法")];
        assert_eq!(
            check_chain(&chain, "node-a"),
            ChainCheck::Cycle {
                node: "node-a".into(),
                first_at: 0
            },
            "A 派 A 同样是环"
        );
    }

    #[test]
    fn chain_without_repeat_is_ok_and_reports_depth() {
        let chain = vec![
            ChainHop::new("node-a", "t-001").expect("应合法"),
            ChainHop::new("node-b", "t-002").expect("应合法"),
        ];
        assert_eq!(check_chain(&chain, "node-c"), ChainCheck::Ok { depth: 3 });
    }

    #[test]
    fn chain_too_deep_is_reported_with_actual_and_limit() {
        // 上限 4：第 5 跳必须判 TooDeep。
        let chain: Vec<ChainHop> = (0..4)
            .map(|i| ChainHop::new(format!("node-{i}"), format!("t-{i}")).expect("应合法"))
            .collect();
        assert_eq!(
            check_chain(&chain, "node-4"),
            ChainCheck::TooDeep {
                depth: 5,
                max: MAX_CHAIN_DEPTH
            }
        );
        // 边界：恰好到上限应通过（证明上限可达）。
        let chain3: Vec<ChainHop> = (0..3)
            .map(|i| ChainHop::new(format!("node-{i}"), format!("t-{i}")).expect("应合法"))
            .collect();
        assert_eq!(check_chain(&chain3, "node-3"), ChainCheck::Ok { depth: 4 });
    }

    #[test]
    fn chain_check_outcomes_are_mutually_exclusive_across_full_length_sweep() {
        // 全长度扫一遍：每一档都必须落进三态之一，无「没判定」。
        let mut ok = 0;
        let mut cycle = 0;
        let mut deep = 0;
        for len in 0..=6usize {
            let chain: Vec<ChainHop> = (0..len)
                .map(|i| ChainHop::new(format!("node-{i}"), format!("t-{i}")).expect("应合法"))
                .collect();
            match check_chain(&chain, "node-new") {
                ChainCheck::Ok { .. } => ok += 1,
                ChainCheck::Cycle { .. } => cycle += 1,
                ChainCheck::TooDeep { .. } => deep += 1,
            }
        }
        assert_eq!(ok + cycle + deep, 7, "已检查 7 档链长，每档都要有判定");
        assert_eq!(cycle, 0, "node-new 从未出现过，不该报环");
        assert_eq!(deep, 3, "长度 3/4/5 起的链超深（MAX=4）：3 档");
        assert_eq!(ok, 4, "长度 0..=2 通过：4 档");
    }
}
