//! 专家团派工：派任务 → 收集 `MemberOutcome` → 处理失败成员。
//!
//! # 幂等：`ux_dispatch_once`
//!
//! `crates/quill-store/migrations/0001_init.sql` 已有
//!
//! ```sql
//! CREATE UNIQUE INDEX ux_dispatch_once
//!   ON task_dispatches (user_id, room_id, round, member_expert_id);
//! ```
//!
//! 也就是说**「同一用户 + 同一房间 + 同一轮 + 同一成员专家」只允许有一条派工**。
//! 本模块的 [`DispatchKey`] 就是这条索引的四元组，
//! [`DispatchLedger::begin`] 里的「已存在则返回既有记录」就是这条约束的
//! Rust 侧载体。
//!
//! ⚠️ **为什么幂等必须在本层判定而不能只靠 DB**：DB 的唯一索引只在
//! 「真的写库」那一刻生效；而崩溃恢复路径要处理的是
//! 「派工已发出、结果未落库」这种**半完成**状态 —— 那一刻索引帮不上忙，
//! 只能靠本层的账本先查后写。
//!
//! # 崩溃恢复：`ix_dispatch_inflight`
//!
//! ```sql
//! CREATE INDEX ix_dispatch_inflight ON task_dispatches (user_id, dispatched_at)
//!   WHERE state IN ('PENDING','RUNNING','ASKING');
//! ```
//!
//! 进程被杀后重启，这三类记录就是「悬挂派工」。本模块的
//! [`DispatchLedger::inflight`] 复刻这个部分索引，
//! [`Dispatcher::recover`] 决定每一条的处置。
//!
//! # 为什么状态机有 5 个状态而不是 2 个
//!
//! `PENDING`（已记账、尚未执行）与 `RUNNING`（已发出、等结果）必须分开：
//! 崩溃后 `PENDING` 可以**安全重派**（成员没跑过，副作用不存在），
//! 而 `RUNNING` 重派就是**重复副作用**（成员可能已经改过文件、写过库）。
//! 把两者合并成一个 `RUNNING`，恢复逻辑就只能一刀切地「全部不重派」，
//! 于是「已记账但没发出的派工」永远悬挂 —— 一个被合并出来的数据丢失 bug。

use std::collections::BTreeMap;
use std::sync::Mutex;

use quill_adapters::{
    check_chain, AbortScope, AdapterError, ChainHop, ExpertId, MemberExecutor, MemberId,
    MemberOutcome, MemberStartRequest, MemberStatus, Message, SessionId, UserId,
};
use quill_domain::Team;

use crate::error::{chain_check_error, AgentError, MemberRejectKind};

/// 派工键：`ux_dispatch_once` 唯一索引的四元组。
///
/// ⚠️ **必须与索引列顺序一致** —— 顺序不同不影响 SQLite 的正确性
/// （唯一性是集合语义），但会让「读索引的人」对键的含义产生第二种理解。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DispatchKey {
    owner: UserId,
    room_id: String,
    round: u32,
    member_expert: ExpertId,
}

impl DispatchKey {
    /// 组装派工键。
    ///
    /// `room_id` 非空校验放在派工层而不是这里：`room_id` 允许含中文
    /// （它是房间名不是标识），只挡空白。
    pub fn new(
        owner: UserId,
        room_id: impl Into<String>,
        round: u32,
        member_expert: ExpertId,
    ) -> Result<Self, AgentError> {
        let room_id = room_id.into();
        if room_id.trim().is_empty() {
            return Err(AgentError::DispatchRequestInvalid {
                reason: "房间标识为空：派工必须归属一个房间，否则崩溃恢复找不到它".into(),
            });
        }
        Ok(Self {
            owner,
            room_id,
            round,
            member_expert,
        })
    }

    /// 属主用户。
    pub fn owner(&self) -> UserId {
        self.owner
    }

    /// 房间标识。
    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    /// 轮次（从 0 起；`task_dispatches.round CHECK (round >= 0)`）。
    pub fn round(&self) -> u32 {
        self.round
    }

    /// 被派的成员专家。
    pub fn member_expert(&self) -> &ExpertId {
        &self.member_expert
    }
}

impl std::fmt::Display for DispatchKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/round-{}/{}",
            self.room_id, self.round, self.member_expert
        )
    }
}

/// 派工状态（`task_dispatches.state` 的 6 个合法值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DispatchState {
    /// 已记账，尚未执行 —— **崩溃后可安全重派**。
    Pending,
    /// 已发出，等结果 —— **崩溃后不可重派**（副作用可能已发生）。
    Running,
    /// 成员在反问主持人（`ask_depth > 0`）。
    Asking,
    /// 已完成（`DONE` / `PARTIAL` 都归这里）。
    Done,
    /// 失败。
    Failed,
    /// 已取消。
    Cancelled,
}

impl DispatchState {
    /// 线格式（与 schema 的 CHECK 逐字一致）。
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Running => "RUNNING",
            Self::Asking => "ASKING",
            Self::Done => "DONE",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
        }
    }

    /// 由线格式解析。
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "PENDING" => Some(Self::Pending),
            "RUNNING" => Some(Self::Running),
            "ASKING" => Some(Self::Asking),
            "DONE" => Some(Self::Done),
            "FAILED" => Some(Self::Failed),
            "CANCELLED" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// 是否为**在途**状态（对应 `ix_dispatch_inflight` 的部分索引条件）。
    pub fn is_inflight(&self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Asking)
    }

    /// 是否为**终态**（对应 `settled_at IS NOT NULL` 的 CHECK）。
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for DispatchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

/// 一条派工记录（`task_dispatches` 的领域投影）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRecord {
    key: DispatchKey,
    member: MemberId,
    state: DispatchState,
    outcome: Option<MemberOutcome>,
    error: Option<AgentError>,
    /// 该成员的反问深度（`ask_depth`）。
    ask_depth: u32,
}

impl DispatchRecord {
    /// 新建一条 `PENDING` 派工记录。
    ///
    /// ⚠️ `ask_depth` 固定 0：schema 有
    /// `CHECK ((state = 'ASKING') = (ask_depth > 0))`，
    /// 所以非 `ASKING` 状态**必须**是 0。
    pub fn pending(key: DispatchKey, member: MemberId) -> Self {
        Self {
            key,
            member,
            state: DispatchState::Pending,
            outcome: None,
            error: None,
            ask_depth: 0,
        }
    }

    /// 派工键。
    pub fn key(&self) -> &DispatchKey {
        &self.key
    }

    /// 成员实例标识。
    pub fn member(&self) -> &MemberId {
        &self.member
    }

    /// 状态。
    pub fn state(&self) -> DispatchState {
        self.state
    }

    /// 成员结果（仅终态且成功时有）。
    pub fn outcome(&self) -> Option<&MemberOutcome> {
        self.outcome.as_ref()
    }

    /// 失败原因（仅 `FAILED` 时有）。
    pub fn error(&self) -> Option<&AgentError> {
        self.error.as_ref()
    }

    /// 反问深度。
    pub fn ask_depth(&self) -> u32 {
        self.ask_depth
    }

    /// 跃迁到 `RUNNING`。只允许从 `PENDING` 出发。
    pub fn mark_running(&mut self) -> Result<(), AgentError> {
        self.transition(DispatchState::Running)
    }

    /// 跃迁到 `ASKING`（`ask_depth` 从 1 起）。
    pub fn mark_asking(&mut self, depth: u32) -> Result<(), AgentError> {
        if depth == 0 {
            // 反向不变量：`ASKING` ⇔ `ask_depth > 0`。深度 0 进 ASKING
            // 会让 schema 的 CHECK 在写库那一刻失败。
            return Err(AgentError::DispatchIllegalTransition {
                detail: format!("跃迁到 ASKING 时 ask_depth 必须 > 0，实际给了 {depth}"),
            });
        }
        if !self.state.is_inflight() {
            return Err(self.illegal("ASKING"));
        }
        self.state = DispatchState::Asking;
        self.ask_depth = depth;
        Ok(())
    }

    /// 结算为 `DONE`（`MemberStatus::Done` 与 `Partial` 都走这里）。
    pub fn settle_done(&mut self, outcome: MemberOutcome) -> Result<(), AgentError> {
        if !outcome.status().is_deliverable() {
            // 反向不变量：把「成员说失败」记成 DONE，主持人会把空产出
            // 当成果汇总。`docs/07` §2.3 明确 partial 要带 completed_scope。
            return Err(AgentError::DispatchIllegalTransition {
                detail: format!(
                    "不能用状态 {} 的结果结算为 DONE（无产出正文）",
                    outcome.status()
                ),
            });
        }
        self.require_inflight("DONE")?;
        self.state = DispatchState::Done;
        self.outcome = Some(outcome);
        self.error = None;
        Ok(())
    }

    /// 结算为 `FAILED`。
    pub fn settle_failed(&mut self, error: AgentError) -> Result<(), AgentError> {
        self.require_inflight("FAILED")?;
        self.state = DispatchState::Failed;
        self.outcome = None;
        self.error = Some(error);
        Ok(())
    }

    /// 结算为 `CANCELLED`。
    pub fn settle_cancelled(&mut self) -> Result<(), AgentError> {
        self.require_inflight("CANCELLED")?;
        self.state = DispatchState::Cancelled;
        self.outcome = None;
        self.error = None;
        Ok(())
    }

    /// 跃迁到终态的公共前置检查。
    fn require_inflight(&self, target: &str) -> Result<(), AgentError> {
        if self.state.is_terminal() {
            return Err(self.illegal(target));
        }
        Ok(())
    }

    fn transition(&mut self, target: DispatchState) -> Result<(), AgentError> {
        if self.state != DispatchState::Pending {
            return Err(self.illegal(target.as_wire()));
        }
        self.state = target;
        Ok(())
    }

    fn illegal(&self, target: &str) -> AgentError {
        AgentError::DispatchIllegalTransition {
            detail: format!(
                "{} 处于 {}{}，不能跃迁到 {target}",
                self.key,
                self.state,
                if self.state.is_terminal() {
                    "（已结算，重复结算会让结果被覆盖）"
                } else {
                    ""
                }
            ),
        }
    }
}

/// 派工结果的一条成员记录（`MemberOutcome` 或失败原因）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberResult {
    /// 成员交付了内容（含 `Partial`）。
    Delivered {
        /// 成员标识。
        member: MemberId,
        /// 成员结果。
        outcome: MemberOutcome,
    },
    /// 成员失败 / 被拒 / 中断。
    Failed {
        /// 成员标识。
        member: MemberId,
        /// 失败原因（中文人话 + 修复命令）。
        error: AgentError,
    },
}

impl MemberResult {
    /// 成员标识。
    pub fn member(&self) -> &MemberId {
        match self {
            Self::Delivered { member, .. } | Self::Failed { member, .. } => member,
        }
    }

    /// 是否交付了内容。
    pub fn is_delivered(&self) -> bool {
        matches!(self, Self::Delivered { .. })
    }

    /// 失败原因（仅失败时有）。
    pub fn error(&self) -> Option<&AgentError> {
        match self {
            Self::Failed { error, .. } => Some(error),
            Self::Delivered { .. } => None,
        }
    }
}

/// 记账结果：区分「新建」与「命中幂等键」。
///
/// # 为什么必须是枚举而不是只返回 `DispatchRecord`
///
/// `ux_dispatch_once` 的语义是「重复插入不生效」，而**重复插入时
/// 拿回来的记录本身无法区分它是自己刚插入的还是上次就在的**。
///
/// 若靠「返回值 == 传入值」来推断（`begin` 写入前先查），
/// 那就等于**先查后插**——两次操作之间存在竞态窗口，
/// 两个并发派工都查到「不存在」然后都插入，
/// 靠 DB 唯一索引兜底时其中一个会拿到**别的**记录，
/// 然后它会以为自己成功记账了。
///
/// 判别必须由**存储层在同一次操作里**给出，这正是
/// `INSERT … ON CONFLICT DO NOTHING RETURNING` 能回答、
/// 而「先 SELECT 再 INSERT」回答不了的问题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginOutcome {
    /// 本次真的插入了新记录（键此前不存在）。
    Created(DispatchRecord),
    /// 键已存在，返回既有记录（未覆盖）。
    Existed(DispatchRecord),
}

impl BeginOutcome {
    /// 记录本身（不论新建还是既有）。
    pub fn record(&self) -> &DispatchRecord {
        match self {
            Self::Created(r) | Self::Existed(r) => r,
        }
    }

    /// 是否是本次新建的。
    pub fn is_created(&self) -> bool {
        matches!(self, Self::Created(_))
    }
}

/// 派工账本端口（由 `quill-server` 用 `sqlx` 实现）。
///
/// ⚠️ **每个方法的第一个参数都是 `owner: &UserId`**：
/// `task_dispatches` 的主键是 `(user_id, id)`，而跨用户隔离是
/// `V1_SCOPE_CONSTRAINTS.md` §五点名的**最高优先级**安全边界。
/// 把 `UserId` 放进签名里，漏传就在编译期断掉。
pub trait DispatchLedger: Send + Sync + 'static {
    /// 记账并返回**判别结果**。若键已存在则返回既有记录（不覆盖）——
    /// 这就是 `ux_dispatch_once` 的语义。见 [`BeginOutcome`] 的说明。
    fn begin(&self, record: &DispatchRecord) -> Result<BeginOutcome, AgentError>;

    /// 覆盖写入（状态机跃迁后调用）。
    fn put(&self, record: &DispatchRecord) -> Result<(), AgentError>;

    /// 按键取记录。
    fn get(&self, key: &DispatchKey) -> Result<Option<DispatchRecord>, AgentError>;

    /// 列出某用户在途派工（对应 `ix_dispatch_inflight`）。
    fn inflight(&self, owner: &UserId) -> Result<Vec<DispatchRecord>, AgentError>;

    /// 列出某房间某轮的全部派工（按 `member_expert` 序）。
    fn list_round(&self, key_prefix: &RoundPrefix) -> Result<Vec<DispatchRecord>, AgentError>;
}

/// 轮次前缀（`ux_dispatch_once` 的前三列）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoundPrefix {
    /// 属主用户。
    pub owner: UserId,
    /// 房间标识。
    pub room_id: String,
    /// 轮次。
    pub round: u32,
}

/// 派工单：一条待派的成员任务。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchTask {
    /// 派给哪个专家。
    pub expert: ExpertId,
    /// 成员实例标识（**必须以专家名 + `-` 开头**，由契约层校验）。
    pub member: MemberId,
    /// 任务标题（UI 可见）。
    pub title: String,
    /// 任务正文（**必须自包含** —— 成员看不到主持人的上下文）。
    pub instructions: String,
}

impl DispatchTask {
    /// 组装派工单并校验。
    pub fn new(
        expert: ExpertId,
        member: MemberId,
        title: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Result<Self, AgentError> {
        let title = title.into();
        let instructions = instructions.into();
        // ⚠️ 这里**复用契约层的校验**而不是自己写一套：
        // 同一份不变量有两处实现，迟早会漂移。
        // 契约层的 `MemberStartRequest::new` 同时校验标题/正文非空
        // 与「成员标识须以专家名开头」。
        MemberStartRequest::new(
            UserId::from_bytes([0; 16]),
            SessionId::from_bytes([0; 16]),
            expert.clone(),
            member.clone(),
            title.clone(),
            instructions.clone(),
        )
        .map_err(|e| AgentError::DispatchRequestInvalid {
            reason: e.to_string(),
        })?;
        Ok(Self {
            expert,
            member,
            title,
            instructions,
        })
    }

    /// 便捷构造：成员标识用 `MemberId::for_expert(expert, seq)`。
    pub fn with_seq(
        expert: ExpertId,
        seq: u32,
        title: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Result<Self, AgentError> {
        let member =
            MemberId::for_expert(&expert, seq).map_err(|e| AgentError::DispatchRequestInvalid {
                reason: format!("成员标识构造失败：{e}"),
            })?;
        Self::new(expert, member, title, instructions)
    }
}

/// 一轮派工的完整报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchReport {
    /// 房间标识。
    pub room_id: String,
    /// 轮次。
    pub round: u32,
    /// 每个成员一条结果（按派工单顺序）。
    pub results: Vec<MemberResult>,
    /// 因幂等而**跳过**的成员（键已存在且已终态）。
    pub skipped_as_duplicate: Vec<MemberId>,
    /// 崩溃恢复时判定为「可安全重派」的键。
    pub recovered_for_retry: Vec<DispatchKey>,
}

impl DispatchReport {
    /// 交付成功（含 `Partial`）的成员数。
    pub fn delivered_count(&self) -> usize {
        self.results.iter().filter(|r| r.is_delivered()).count()
    }

    /// 失败的成员数。
    pub fn failed_count(&self) -> usize {
        self.results.iter().filter(|r| !r.is_delivered()).count()
    }

    /// 是否「全员失败」。
    ///
    /// ⚠️ 存在的理由：上层据此决定「是否要告诉用户这轮白跑了」。
    /// 没有它就只能靠 `failed_count() > 0` —— 那在「1 成功 1 失败」时
    /// 会误报成全失败。
    pub fn is_total_failure(&self) -> bool {
        !self.results.is_empty() && self.failed_count() == self.results.len()
    }

    /// 汇总文本（主持人 / `quill doctor` 用）。
    ///
    /// ⚠️ **失败条目只打错误码，不重复打修复命令**：`AgentError` 的
    /// `Display` 尾部**已经**带了 `fix_command()`。这里再打一遍会出现
    /// 两条一模一样的命令，而用户会以为是两个不同动作 ——
    /// 那是文案层的第二份真相源。命令的唯一出口是 `Display`。
    pub fn summary(&self) -> String {
        let mut s = format!(
            "房间 {} 第 {} 轮：交付 {} / 失败 {} / 幂等跳过 {}",
            self.room_id,
            self.round,
            self.delivered_count(),
            self.failed_count(),
            self.skipped_as_duplicate.len()
        );
        for r in self.results.iter().filter(|r| !r.is_delivered()) {
            let e = r.error().expect("失败条目必有 error（构造时保证）");
            s.push_str(&format!(
                "\n  ✗ 成员 {}（错误码 {}）：{}",
                r.member(),
                e.code(),
                e
            ));
        }
        s
    }
}

/// 一轮派工的入参。
///
/// # 为什么是结构体而不是 8 个位置参数
///
/// `dispatch_round(owner, session, team, room_id, round, tasks, chain)` 有 7 个
/// 业务参数 —— 全部是同一种类型（`UserId` / `SessionId` / `&str` / `u32` …）
/// 时，**传错顺序在编译期也是合法的**：`round: u32` 与 `seq: u32` 互换
/// 不会有任何编译错误，只会让幂等键的轮次整体偏移。
///
/// 结构体让调用方必须写字段名，`round: 0` 不会被误填成 `session: 0`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundRequest<'a> {
    /// 属主用户（同时是账本隔离的 `user_id`）。
    pub owner: UserId,
    /// 主持人会话标识。
    pub session: SessionId,
    /// 团队（提供成员关系与主持人身份）。
    pub team: &'a Team,
    /// 房间标识。
    pub room_id: &'a str,
    /// 轮次（`ux_dispatch_once` 的第三列）。
    pub round: u32,
    /// 派工单列表。
    pub tasks: &'a [DispatchTask],
    /// 委派链上游（环防护用）。
    pub chain: &'a [ChainHop],
}

/// 派工器：把派工单发给成员执行器，把结果收回账本。
#[derive(Debug)]
pub struct Dispatcher<E: MemberExecutor, L: DispatchLedger> {
    executor: E,
    ledger: L,
}

impl<E: MemberExecutor, L: DispatchLedger> Dispatcher<E, L> {
    /// 组装派工器。
    pub fn new(executor: E, ledger: L) -> Self {
        Self { executor, ledger }
    }

    /// 只读访问账本（供上层做一致性对账）。
    pub fn ledger(&self) -> &L {
        &self.ledger
    }

    /// 派一轮工。
    ///
    /// # 幂等语义（逐条）
    ///
    /// | 键的既有状态 | 本次行为 |
    /// |---|---|
    /// | 不存在 | 记 `PENDING` → 执行 → 结算 |
    /// | 终态（`DONE`/`FAILED`/`CANCELLED`） | **跳过**，记入 `skipped_as_duplicate`，不碰执行器 |
    /// | `PENDING` | 视为上次崩溃留下的未发出派工 → 重新执行（见下） |
    /// | `RUNNING`/`ASKING` | **跳过并如实标记**：执行可能已产生副作用，重派就是重复执行 |
    ///
    /// ⚠️ `PENDING` 之所以可安全重派：成员**从未被调用**，
    /// 副作用不可能发生。若把它也一并跳过，
    /// 「记账后立刻崩溃」就会永久丢一个成员 —— 一个静默的数据丢失 bug。
    pub fn dispatch_round(&self, req: &RoundRequest<'_>) -> Result<DispatchReport, AgentError> {
        let RoundRequest {
            owner,
            session,
            team,
            room_id,
            round,
            tasks,
            chain,
        } = *req;
        let mut results = Vec::new();
        let mut skipped = Vec::new();
        let mut recovered = Vec::new();

        // ⚠️ **校验必须整轮先做完，再开派**。
        // 若边派边校验，第 3 个成员非法时前 2 个已经执行完并落账，
        // 而函数返回 `Err` 让调用方**拿不到**它们的结果 ——
        // 成员跑了、token 烧了、结果被丢掉，且账本里留下两条调用方不知道的 DONE。
        // 那是一次**完全无报错**的静默数据丢失（错误来自别的成员）。
        //
        // ⚠️ 解构出的是**引用**（`&[DispatchTask]` / `&[ChainHop]` / `&Team`），
        // 因此下面一律用 `tasks` / `chain` / `team` 原样，
        // 不做 `*tasks` 解引用 —— 切片不能按值迭代。
        for task in tasks {
            // ① 团队成员关系（名册口径由 quill-domain 的 Team 持有）
            if !team.has_member(&task.expert) && !team.is_leader(&task.expert) {
                // ⚠️ 反向用例：不校验成员关系就等于允许「派工给团外专家」，
                // 而 Team 的存在意义恰恰是划定这条边界。
                return Err(AgentError::TeamInvalid(
                    quill_domain::TeamError::UnknownExpert(task.expert.clone()),
                ));
            }
            // ② 环 / 超深防护（在触达执行器之前）
            if let Some(err) = chain_check_error(check_chain(chain, task.expert.as_str())) {
                return Err(err);
            }
        }

        for task in tasks {
            let key = DispatchKey::new(owner, room_id, round, task.expert.clone())?;
            let begun = self
                .ledger
                .begin(&DispatchRecord::pending(key.clone(), task.member.clone()))?;

            // ⚠️ **只有「本次新建」或「上次留下的 PENDING」才继续执行**。
            // `BeginOutcome` 的判别由存储层在同一次操作里给出，
            // 「先查后插」会有竞态窗口（见该枚举的文档）。
            let is_recovery = !begun.is_created();
            match begun.record().state() {
                DispatchState::Pending => {
                    if is_recovery {
                        // 上次崩溃留下的 PENDING：成员从未被调用，可安全重派。
                        recovered.push(key.clone());
                    }
                }
                s if s.is_terminal() => {
                    skipped.push(task.member.clone());
                    continue;
                }
                _ => {
                    // RUNNING / ASKING：执行可能已产生副作用。
                    // ⚠️ 契约（docs/07 §4.3 + PHASE2 §七.4）：
                    //    派工失败**不自动重跑**。这里同样如实跳过。
                    skipped.push(task.member.clone());
                    continue;
                }
            }

            results.push(self.run_one(owner, session, key, task, chain)?);
        }

        Ok(DispatchReport {
            room_id: room_id.to_string(),
            round,
            results,
            skipped_as_duplicate: skipped,
            recovered_for_retry: recovered,
        })
    }

    /// 执行单个成员任务并结算。
    fn run_one(
        &self,
        owner: UserId,
        session: SessionId,
        key: DispatchKey,
        task: &DispatchTask,
        chain: &[ChainHop],
    ) -> Result<MemberResult, AgentError> {
        let mut record = DispatchRecord::pending(key, task.member.clone());
        record.mark_running()?;
        self.ledger.put(&record)?;

        let mut req = MemberStartRequest::new(
            owner,
            session,
            task.expert.clone(),
            task.member.clone(),
            task.title.clone(),
            task.instructions.clone(),
        )
        .map_err(|e| AgentError::DispatchRequestInvalid {
            reason: e.to_string(),
        })?;
        for hop in chain {
            req = req.with_chain(hop.clone());
        }

        // ⚠️ 这里是**唯一**把控制权交给执行器的地方。
        // 执行器（`MemberExecutor`）是 trait，测试用 `MockMemberExecutor`，
        // 生产用真实实现 —— 编排层不区分两者。
        let started = block_on(self.executor.start(req));

        let result = match started {
            Ok(outcome) => {
                // 成员说「完成 / 部分完成」→ 记 DONE。
                // 成员说「失败 / 取消」→ 记 FAILED（**不冒充成功**）。
                if outcome.status().is_deliverable() {
                    record.settle_done(outcome.clone())?;
                    self.ledger.put(&record)?;
                    MemberResult::Delivered {
                        member: task.member.clone(),
                        outcome,
                    }
                } else {
                    let err = AgentError::MemberRejected {
                        member: task.member.clone(),
                        kind: match outcome.status() {
                            MemberStatus::Cancelled => MemberRejectKind::Cancelled,
                            _ => MemberRejectKind::SelfReportedFailure,
                        },
                        detail: format!(
                            "成员自报状态 {}，已完成范围：{}",
                            outcome.status(),
                            outcome.completed_scope()
                        ),
                        // ⚠️ 成员自报失败**不可重试**：它可能已经做了一半。
                        retryable: false,
                    };
                    record.settle_failed(err.clone())?;
                    self.ledger.put(&record)?;
                    MemberResult::Failed {
                        member: task.member.clone(),
                        error: err,
                    }
                }
            }
            Err(adapter_err) => {
                // ⚠️ `AdapterError` → `AgentError` 时**必须**传成员标识：
                // 契约层的错误载荷里没有成员号（见 error.rs 的说明）。
                let err = AgentError::from_member_error(&task.member, adapter_err);
                record.settle_failed(err.clone())?;
                self.ledger.put(&record)?;
                MemberResult::Failed {
                    member: task.member.clone(),
                    error: err,
                }
            }
        };
        Ok(result)
    }

    /// 崩溃恢复：处理进程被杀后留下的在途派工。
    ///
    /// # 处置口径（`ix_dispatch_inflight` 的三类状态）
    ///
    /// | 状态 | 处置 | 理由 |
    /// |---|---|---|
    /// | `PENDING` | 判定为**可安全重派** | 成员从未被调用，副作用不存在 |
    /// | `RUNNING` / `ASKING` | 判定为**必须人工确认** | 执行可能已产生副作用；契约规定不自动重跑 |
    ///
    /// ⚠️ 本方法**只判定、不执行**。自动重派需要一个
    /// 「拿什么任务正文重派」的来源，而 `task_dispatches` 表
    /// 只存 `task_digest`（摘要）不存正文 —— 在没有正文的情况下
    /// 自动重派等于凭空造任务，那比不重派更危险。
    pub fn recover(&self, owner: &UserId) -> Result<RecoveryReport, AgentError> {
        let inflight = self.ledger.inflight(owner)?;
        let mut safe = Vec::new();
        let mut needs_confirmation = Vec::new();
        for r in &inflight {
            match r.state() {
                DispatchState::Pending => safe.push(r.key().clone()),
                _ => needs_confirmation.push(r.key().clone()),
            }
        }
        Ok(RecoveryReport {
            checked: inflight.len(),
            safe_to_retry: safe,
            needs_confirmation,
        })
    }

    /// 注入消息给某个成员。
    ///
    /// ⚠️ `steer` 失败**不改变派工状态**（`docs/07` §0.1.1 保留 G2）：
    /// 隧道 WS 断开不代表成员执行失败。
    pub fn steer(&self, member: &MemberId, text: &str) -> Result<(), AgentError> {
        let msg = Message::user(text).map_err(|e| AgentError::DispatchRequestInvalid {
            reason: e.to_string(),
        })?;
        block_on(self.executor.steer(member, msg))
            .map_err(|e| AgentError::from_member_error(member, e))
    }

    /// 中止成员。`StopRound` 不停成员，`AbortRoom` 才停。
    pub fn abort(&self, member: &MemberId, scope: AbortScope) -> Result<(), AgentError> {
        block_on(self.executor.abort(member, scope))
            .map_err(|e| AgentError::from_member_error(member, e))
    }
}

/// 崩溃恢复报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// 实际检查到的在途派工数。
    ///
    /// ⚠️ 必须显式带上「检查了几个」：否则「0 条在途」与
    /// 「根本没查到」在屏幕上长得一样（铁律十六）。
    pub checked: usize,
    /// 可安全重派的键（`PENDING`）。
    pub safe_to_retry: Vec<DispatchKey>,
    /// 需人工确认的键（`RUNNING` / `ASKING`）。
    pub needs_confirmation: Vec<DispatchKey>,
}

impl RecoveryReport {
    /// 人类可读摘要。
    pub fn summary(&self) -> String {
        format!(
            "崩溃恢复：已检查 {} 条在途派工，可安全重派 {} 条，需人工确认 {} 条。\
             需人工确认的原因：执行可能已产生副作用，系统不自动重跑（契约 docs/07 §4.3）",
            self.checked,
            self.safe_to_retry.len(),
            self.needs_confirmation.len()
        )
    }

    /// 是否一切正常（无在途派工）。
    pub fn is_clean(&self) -> bool {
        self.checked == 0
    }
}

// ─────────────────────────── 内存账本 ───────────────────────────

/// 内存派工账本（测试用 + `quill-server` 未接库时的降级实现）。
///
/// ⚠️ **它复刻了 `ux_dispatch_once` 的唯一约束**，否则
/// 「幂等」这件事在测试里根本没被验证 —— 换成真库时才第一次发现
/// 「我以为 begin 是幂等的，其实每次都插了新行」。
#[derive(Debug, Default)]
pub struct MemDispatchLedger {
    rows: Mutex<BTreeMap<DispatchKey, DispatchRecord>>,
}

impl MemDispatchLedger {
    /// 构造空账本。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前记录数（「已检查 N 个」用）。
    pub fn len(&self) -> usize {
        self.rows.lock().expect("账本锁不应被毒化").len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl DispatchLedger for MemDispatchLedger {
    fn begin(&self, record: &DispatchRecord) -> Result<BeginOutcome, AgentError> {
        let mut g = self.rows.lock().expect("账本锁不应被毒化");
        if let Some(existing) = g.get(record.key()) {
            // ⚠️ 关键：返回**既有记录**而不报错。
            // 报错会让「重试」变成「失败」，而幂等要的正是
            // 「重试拿到同一个结果」。
            return Ok(BeginOutcome::Existed(existing.clone()));
        }
        g.insert(record.key().clone(), record.clone());
        Ok(BeginOutcome::Created(record.clone()))
    }

    fn put(&self, record: &DispatchRecord) -> Result<(), AgentError> {
        self.rows
            .lock()
            .expect("账本锁不应被毒化")
            .insert(record.key().clone(), record.clone());
        Ok(())
    }

    fn get(&self, key: &DispatchKey) -> Result<Option<DispatchRecord>, AgentError> {
        Ok(self
            .rows
            .lock()
            .expect("账本锁不应被毒化")
            .get(key)
            .cloned())
    }

    fn inflight(&self, owner: &UserId) -> Result<Vec<DispatchRecord>, AgentError> {
        Ok(self
            .rows
            .lock()
            .expect("账本锁不应被毒化")
            .values()
            .filter(|r| r.key().owner() == *owner && r.state().is_inflight())
            .cloned()
            .collect())
    }

    fn list_round(&self, prefix: &RoundPrefix) -> Result<Vec<DispatchRecord>, AgentError> {
        let mut out: Vec<DispatchRecord> = self
            .rows
            .lock()
            .expect("账本锁不应被毒化")
            .values()
            .filter(|r| {
                r.key().owner() == prefix.owner
                    && r.key().room_id() == prefix.room_id
                    && r.key().round() == prefix.round
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.key().member_expert().cmp(b.key().member_expert()));
        Ok(out)
    }
}

// ─────────────────────────── 共享执行器包装 ───────────────────────────

/// 共享执行器：把 `Arc<E>` 适配成 `MemberExecutor`。
///
/// # 为什么需要这个 newtype（而不是 blanket impl）
///
/// `MemberExecutor` 是 `quill-adapters` 的**外部** trait，而 `Arc` 也是外部类型，
/// 所以 `impl<E: MemberExecutor> MemberExecutor for Arc<E>` 会被
/// Rust 的孤儿规则拒绝（`Arc` 不是 `#[fundamental]`，且 `E` 是未覆盖类型参数）。
/// **本 crate 不能给外部 trait 补 blanket impl** —— 那是契约层的事，
/// 而契约层归别人所有。
///
/// # 为什么生产上也需要它
///
/// `MemberExecutor: Send + Sync + 'static` 的存在意义就是**跨线程共享**
/// （`quill-adapters` 的模块注释明说「成员 task 要 `tokio::spawn`」）。
/// 而 `Dispatcher` 按值持有 `E`，若 `E = MockMemberExecutor`，
/// 派工器就独占了这个 mock，调用方**拿不到句柄去断言调用次数** ——
/// 编排层的时序断言（「`steer` 在 `start` 之后」）就无从写起。
/// 本包装让「派工器持有」与「测试/上层持有」指向同一个执行器实例。
#[derive(Debug)]
pub struct SharedExecutor<E: MemberExecutor> {
    inner: std::sync::Arc<E>,
}

impl<E: MemberExecutor> SharedExecutor<E> {
    /// 包装一个 `Arc<E>`。
    pub fn new(inner: std::sync::Arc<E>) -> Self {
        Self { inner }
    }

    /// 取回内部 `Arc`（调用方需要它来断言调用记录）。
    pub fn handle(&self) -> &std::sync::Arc<E> {
        &self.inner
    }
}

impl<E: MemberExecutor> MemberExecutor for SharedExecutor<E> {
    fn start(
        &self,
        req: MemberStartRequest,
    ) -> impl std::future::Future<Output = Result<MemberOutcome, AdapterError>> + Send {
        self.inner.start(req)
    }

    fn steer(
        &self,
        member: &MemberId,
        m: Message,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        self.inner.steer(member, m)
    }

    fn abort(
        &self,
        member: &MemberId,
        scope: AbortScope,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        self.inner.abort(member, scope)
    }
}

// ─────────────────────────── 迷你执行器 ───────────────────────────

/// 把 future 跑到完成。
///
/// ⚠️ **为什么需要它**：`quill-agent` 的依赖表里没有 tokio
/// （新增外部依赖须主理人裁决），而派工必须 await `MemberExecutor`。
/// `Waker::noop()`（Rust 1.85+ 稳定）是**安全**的 noop waker，
/// 因此本 crate 不需要 `unsafe`（`unsafe_code = "forbid"`）。
///
/// ⚠️ **轮询上限必须有**：无上限的 `loop` 在 future 挂起时会变成
/// 无限空转，把一个死锁伪装成「慢」。超上限直接判失败 ——
/// 绝不「返回默认值」兜底（那会把「跑不完」变成「跑成了空结果」）。
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll};
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = Box::pin(fut);
    for _ in 0..10_000 {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
    panic!("派工 future 在 10000 次轮询后仍未完成：真实执行器需接 tokio（quill-server 侧注入）");
}
