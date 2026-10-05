use std::collections::BTreeMap;
use std::sync::Mutex;

use quill_adapters::{
    check_chain, AbortScope, AdapterError, ChainHop, ExpertId, MemberExecutor, MemberId,
    MemberOutcome, MemberStartRequest, MemberStatus, Message, SessionId, UserId,
};
use quill_domain::Team;

use crate::error::{chain_check_error, AgentError, MemberRejectKind};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DispatchKey {
    owner: UserId,
    room_id: String,
    round: u32,
    member_expert: ExpertId,
}

impl DispatchKey {
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

    pub fn owner(&self) -> UserId {
        self.owner
    }

    pub fn room_id(&self) -> &str {
        &self.room_id
    }

    pub fn round(&self) -> u32 {
        self.round
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DispatchState {
    Pending,

    Running,

    Asking,

    Done,

    Failed,

    Cancelled,
}

impl DispatchState {
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

    pub fn is_inflight(&self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Asking)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for DispatchState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRecord {
    key: DispatchKey,
    member: MemberId,
    state: DispatchState,
    outcome: Option<MemberOutcome>,
    error: Option<AgentError>,

    ask_depth: u32,
}

impl DispatchRecord {
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

    pub fn key(&self) -> &DispatchKey {
        &self.key
    }

    pub fn member(&self) -> &MemberId {
        &self.member
    }

    pub fn state(&self) -> DispatchState {
        self.state
    }

    pub fn outcome(&self) -> Option<&MemberOutcome> {
        self.outcome.as_ref()
    }

    pub fn error(&self) -> Option<&AgentError> {
        self.error.as_ref()
    }

    pub fn ask_depth(&self) -> u32 {
        self.ask_depth
    }

    pub fn mark_running(&mut self) -> Result<(), AgentError> {
        self.transition(DispatchState::Running)
    }

    pub fn mark_asking(&mut self, depth: u32) -> Result<(), AgentError> {
        if depth == 0 {
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

    pub fn settle_done(&mut self, outcome: MemberOutcome) -> Result<(), AgentError> {
        if !outcome.status().is_deliverable() {
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

    pub fn settle_failed(&mut self, error: AgentError) -> Result<(), AgentError> {
        self.require_inflight("FAILED")?;
        self.state = DispatchState::Failed;
        self.outcome = None;
        self.error = Some(error);
        Ok(())
    }

    pub fn settle_cancelled(&mut self) -> Result<(), AgentError> {
        self.require_inflight("CANCELLED")?;
        self.state = DispatchState::Cancelled;
        self.outcome = None;
        self.error = None;
        Ok(())
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberResult {
    Delivered {
        member: MemberId,

        outcome: MemberOutcome,
    },

    Failed {
        member: MemberId,

        error: AgentError,
    },
}

impl MemberResult {
    pub fn member(&self) -> &MemberId {
        match self {
            Self::Delivered { member, .. } | Self::Failed { member, .. } => member,
        }
    }

    pub fn is_delivered(&self) -> bool {
        matches!(self, Self::Delivered { .. })
    }

    pub fn error(&self) -> Option<&AgentError> {
        match self {
            Self::Failed { error, .. } => Some(error),
            Self::Delivered { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginOutcome {
    Created(DispatchRecord),

    Existed(DispatchRecord),
}

impl BeginOutcome {
    pub fn record(&self) -> &DispatchRecord {
        match self {
            Self::Created(r) | Self::Existed(r) => r,
        }
    }

    pub fn is_created(&self) -> bool {
        matches!(self, Self::Created(_))
    }
}

pub trait DispatchLedger: Send + Sync + 'static {
    fn begin(&self, record: &DispatchRecord) -> Result<BeginOutcome, AgentError>;

    fn put(&self, record: &DispatchRecord) -> Result<(), AgentError>;

    fn get(&self, key: &DispatchKey) -> Result<Option<DispatchRecord>, AgentError>;

    fn inflight(&self, owner: &UserId) -> Result<Vec<DispatchRecord>, AgentError>;

    fn list_round(&self, key_prefix: &RoundPrefix) -> Result<Vec<DispatchRecord>, AgentError>;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoundPrefix {
    pub owner: UserId,

    pub room_id: String,

    pub round: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchTask {
    pub expert: ExpertId,

    pub member: MemberId,

    pub title: String,

    pub instructions: String,
}

impl DispatchTask {
    pub fn new(
        expert: ExpertId,
        member: MemberId,
        title: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Result<Self, AgentError> {
        let title = title.into();
        let instructions = instructions.into();

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchReport {
    pub room_id: String,

    pub round: u32,

    pub results: Vec<MemberResult>,

    pub skipped_as_duplicate: Vec<MemberId>,

    pub recovered_for_retry: Vec<DispatchKey>,
}

impl DispatchReport {
    pub fn delivered_count(&self) -> usize {
        self.results.iter().filter(|r| r.is_delivered()).count()
    }

    pub fn failed_count(&self) -> usize {
        self.results.iter().filter(|r| !r.is_delivered()).count()
    }

    pub fn is_total_failure(&self) -> bool {
        !self.results.is_empty() && self.failed_count() == self.results.len()
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundRequest<'a> {
    pub owner: UserId,

    pub session: SessionId,

    pub team: &'a Team,

    pub room_id: &'a str,

    pub round: u32,

    pub tasks: &'a [DispatchTask],

    pub chain: &'a [ChainHop],
}

#[derive(Debug)]
pub struct Dispatcher<E: MemberExecutor, L: DispatchLedger> {
    executor: E,
    ledger: L,
}

impl<E: MemberExecutor, L: DispatchLedger> Dispatcher<E, L> {
    pub fn new(executor: E, ledger: L) -> Self {
        Self { executor, ledger }
    }

    pub fn ledger(&self) -> &L {
        &self.ledger
    }

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

        for task in tasks {
            if !team.has_member(&task.expert) && !team.is_leader(&task.expert) {
                return Err(AgentError::TeamInvalid(
                    quill_domain::TeamError::UnknownExpert(task.expert.clone()),
                ));
            }

            if let Some(err) = chain_check_error(check_chain(chain, task.expert.as_str())) {
                return Err(err);
            }
        }

        for task in tasks {
            let key = DispatchKey::new(owner, room_id, round, task.expert.clone())?;
            let begun = self
                .ledger
                .begin(&DispatchRecord::pending(key.clone(), task.member.clone()))?;

            let is_recovery = !begun.is_created();
            match begun.record().state() {
                DispatchState::Pending => {
                    if is_recovery {
                        recovered.push(key.clone());
                    }
                }
                s if s.is_terminal() => {
                    skipped.push(task.member.clone());
                    continue;
                }
                _ => {
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

        let started = block_on(self.executor.start(req));

        let result = match started {
            Ok(outcome) => {
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

    pub fn steer(&self, member: &MemberId, text: &str) -> Result<(), AgentError> {
        let msg = Message::user(text).map_err(|e| AgentError::DispatchRequestInvalid {
            reason: e.to_string(),
        })?;
        block_on(self.executor.steer(member, msg))
            .map_err(|e| AgentError::from_member_error(member, e))
    }

    pub fn abort(&self, member: &MemberId, scope: AbortScope) -> Result<(), AgentError> {
        block_on(self.executor.abort(member, scope))
            .map_err(|e| AgentError::from_member_error(member, e))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    pub checked: usize,

    pub safe_to_retry: Vec<DispatchKey>,

    pub needs_confirmation: Vec<DispatchKey>,
}

impl RecoveryReport {
    pub fn summary(&self) -> String {
        format!(
            "崩溃恢复：已检查 {} 条在途派工，可安全重派 {} 条，需人工确认 {} 条。\
             需人工确认的原因：执行可能已产生副作用，系统不自动重跑（契约 docs/07 §4.3）",
            self.checked,
            self.safe_to_retry.len(),
            self.needs_confirmation.len()
        )
    }

    pub fn is_clean(&self) -> bool {
        self.checked == 0
    }
}

#[derive(Debug, Default)]
pub struct MemDispatchLedger {
    rows: Mutex<BTreeMap<DispatchKey, DispatchRecord>>,
}

impl MemDispatchLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.rows.lock().expect("账本锁不应被毒化").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl DispatchLedger for MemDispatchLedger {
    fn begin(&self, record: &DispatchRecord) -> Result<BeginOutcome, AgentError> {
        let mut g = self.rows.lock().expect("账本锁不应被毒化");
        if let Some(existing) = g.get(record.key()) {
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

#[derive(Debug)]
pub struct SharedExecutor<E: MemberExecutor> {
    inner: std::sync::Arc<E>,
}

impl<E: MemberExecutor> SharedExecutor<E> {
    pub fn new(inner: std::sync::Arc<E>) -> Self {
        Self { inner }
    }

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
