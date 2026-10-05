
use std::collections::BTreeMap;
use std::sync::Mutex;

use quill_adapters::{
    AbortScope, AdapterError, ChainHop, MemberExecutor, MemberId, MemberOutcome,
    MemberStartRequest, MemberStatus, Message,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultKind {

    Timeout,

    LinkDropped,

    Rejected,

    PeerUnreachable,

    PeerRevoked,

    CycleDetected,

    Member(String),
}

impl FaultKind {

    pub fn to_error(&self, member: &MemberId) -> AdapterError {
        match self {
            Self::Timeout => AdapterError::Provider(format!("成员 {member} 超时")),
            Self::LinkDropped => AdapterError::Provider(format!("成员 {member} 执行中断链")),
            Self::Rejected => AdapterError::Forbidden(format!("对端拒绝接收成员 {member} 的任务")),
            Self::PeerUnreachable => {
                AdapterError::Provider(format!("对端不可达，无法启动成员 {member}"))
            }
            Self::PeerRevoked => {
                AdapterError::Unauthorized(format!("对端已撤销我方 key，成员 {member} 无法执行"))
            }
            Self::CycleDetected => AdapterError::Conflict(format!("委派链成环，拒绝 {member}")),
            Self::Member(msg) => AdapterError::Provider(msg.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {

    Outcome(MemberOutcome),

    Fault(FaultKind),
}

#[derive(Debug, Default)]
pub struct MockMemberExecutor {
    script: Mutex<Vec<Step>>,
    calls: Mutex<Vec<CallRecord>>,
    aborted: Mutex<BTreeMap<MemberId, AbortScope>>,
}

impl MockMemberExecutor {

    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, step: Step) -> &Self {
        self.lock_script().push(step);
        self
    }

    pub fn push_ok(&self, member: MemberId, output: &str) -> &Self {
        self.push(Step::Outcome(
            MemberOutcome::done(member, "已完成", output).expect("测试用结果应合法"),
        ))
    }

    pub fn push_fault(&self, kind: FaultKind) -> &Self {
        self.push(Step::Fault(kind))
    }

    pub fn calls(&self) -> Vec<CallRecord> {
        self.lock_calls().clone()
    }

    pub fn call_count(&self) -> usize {
        self.lock_calls().len()
    }

    pub fn count_of(&self, method: &str) -> usize {
        self.lock_calls()
            .iter()
            .filter(|c| c.method() == method)
            .count()
    }

    pub fn called_in_order(&self, first: &str, second: &str) -> bool {
        let calls = self.lock_calls();
        let first_at = calls.iter().position(|c| c.method() == first);
        let second_at = calls.iter().position(|c| c.method() == second);
        match (first_at, second_at) {
            (Some(a), Some(b)) => a < b,

            _ => false,
        }
    }

    pub fn aborted(&self) -> BTreeMap<MemberId, AbortScope> {
        self.lock_aborted().clone()
    }

    pub fn halted_members(&self) -> Vec<String> {
        self.lock_aborted()
            .iter()
            .filter(|(_, s)| s.halts_members())
            .map(|(m, _)| m.as_str().to_string())
            .collect()
    }

    pub fn clear_calls(&self) {
        self.lock_calls().clear();
    }

    fn lock_script(&self) -> std::sync::MutexGuard<'_, Vec<Step>> {

        self.script.lock().expect("mock 脚本锁不应被毒化")
    }

    fn lock_calls(&self) -> std::sync::MutexGuard<'_, Vec<CallRecord>> {
        self.calls.lock().expect("mock 调用锁不应被毒化")
    }

    fn lock_aborted(&self) -> std::sync::MutexGuard<'_, BTreeMap<MemberId, AbortScope>> {
        self.aborted.lock().expect("mock 中止锁不应被毒化")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    seq: u64,
    method: &'static str,
    member: Option<MemberId>,
    detail: String,
}

impl CallRecord {

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn method(&self) -> &'static str {
        self.method
    }

    pub fn member(&self) -> Option<&MemberId> {
        self.member.as_ref()
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }

    fn next_seq(&self) -> u64 {
        self.seq + 1
    }
}

impl MemberExecutor for MockMemberExecutor {
    fn start(
        &self,
        req: MemberStartRequest,
    ) -> impl std::future::Future<Output = Result<MemberOutcome, AdapterError>> + Send {
        let member = req.member().clone();
        {
            let mut calls = self.lock_calls();
            let seq = calls.last().map_or(1, CallRecord::next_seq);
            calls.push(CallRecord {
                seq,
                method: "start",
                member: Some(member.clone()),
                detail: req.expert().as_str().to_string(),
            });
        }
        let step = self.lock_script().first().cloned();
        if step.is_some() {
            self.lock_script().remove(0);
        }
        let outcome = match step {
            Some(Step::Outcome(o)) => Ok(o),
            Some(Step::Fault(f)) => Err(f.to_error(&member)),
            None => {
                let checked = self.call_count();
                Err(AdapterError::Conflict(format!(
                    "mock 脚本已耗尽：成员 {member} 的 start 无对应步骤（已记录 {checked} 次调用）"
                )))
            }
        };
        async move { outcome }
    }

    fn steer(
        &self,
        member: &MemberId,
        m: Message,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        {
            let mut calls = self.lock_calls();
            let seq = calls.last().map_or(1, CallRecord::next_seq);
            calls.push(CallRecord {
                seq,
                method: "steer",
                member: Some(member.clone()),
                detail: m.text().to_string(),
            });
        }

        let result = self.next_steer_result(member);
        async move { result }
    }

    fn abort(
        &self,
        member: &MemberId,
        scope: AbortScope,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        {
            let mut calls = self.lock_calls();
            let seq = calls.last().map_or(1, CallRecord::next_seq);
            calls.push(CallRecord {
                seq,
                method: "abort",
                member: Some(member.clone()),
                detail: scope.to_string(),
            });
        }
        let result = match scope {
            AbortScope::StopRound => Ok(()),
            AbortScope::AbortRoom => {
                self.lock_aborted().insert(member.clone(), scope);
                Ok(())
            }
        };
        async move { result }
    }
}

impl MockMemberExecutor {

    fn next_steer_result(&self, member: &MemberId) -> Result<(), AdapterError> {
        let mut script = self.lock_script();
        match script.first().cloned() {
            Some(Step::Fault(f)) => {
                script.remove(0);
                Err(f.to_error(member))
            }
            Some(Step::Outcome(_)) => {

                Err(AdapterError::Internal(format!(
                    "mock 脚本错位：成员 {member} 的 steer 撞上了 start 的成功步"
                )))
            }
            None => {
                let checked = self.lock_calls().len();
                Err(AdapterError::Conflict(format!(
                    "mock 脚本已耗尽：成员 {member} 的 steer 无对应步骤（已记录 {checked} 次调用）"
                )))
            }
        }
    }
}

pub fn chain(pairs: &[(&str, &str)]) -> Vec<ChainHop> {
    pairs
        .iter()
        .map(|(n, t)| ChainHop::new(*n, *t).expect("测试用链跳应合法"))
        .collect()
}

pub fn outcome(
    member: &MemberId,
    status: MemberStatus,
    scope: &str,
    output: &str,
) -> MemberOutcome {
    quill_adapters::MemberOutcome::new(member.clone(), status, scope, output)
        .expect("测试用结果应合法")
}
