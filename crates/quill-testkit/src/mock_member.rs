//! `MockMemberExecutor` —— 成员执行注入点的测试替身
//!
//! 依据 `docs/07_实例间委派设计.md` §0.1.1 与 `docs/DECISIONS.md` D-2026-10-05-02。
//!
//! # 这个 mock 为什么是本任务真正的价值
//!
//! `docs/03_智能体编排设计.md` §2.12.2 论据 3：
//! 「编排层可在测试中用 mock executor 替换」——没有它，T-A9~T-A24 全套状态机测试
//! **必须起真 provider**，违反 `V1_SCOPE_CONSTRAINTS` §五「测试是唯一防线」。
//!
//! 且 §0.1.2 明确：**委派走隧道 = 网络失败模式（超时 / 断链 / 对端拒收）
//! 必须能在测试中复现**。这三种失败模式正是 [`FaultKind`] 的前三个变体。
//!
//! # 为什么调用记录是「一等公民」而不是 debug 输出
//!
//! 编排层要断言的是**时序**（「`steer` 在 `start` 之后」）
//! 与**次数**（「重试了 2 次才成功」）。这两类断言若靠日志文本，
//! 就落进铁律十九说的「结论来自输出文本而非返回值」。因此记录是结构化的
//! [`CallRecord`]，可被 `assert_eq!` 直接比对。

use std::collections::BTreeMap;
use std::sync::Mutex;

use quill_adapters::{
    AbortScope, AdapterError, ChainHop, MemberExecutor, MemberId, MemberOutcome,
    MemberStartRequest, MemberStatus, Message,
};

// ─────────────────────────── 故障注入 ───────────────────────────

/// 成员执行的失败模式。
///
/// 前三个对应 `docs/07` §0.1.2 点名的**委派网络失败模式**；
/// 后两个是本地执行器的失败模式。**刻意不设「默认成功」之外的隐式行为**：
/// 未命中脚本时 mock 返回 `Conflict`（脚本耗尽），而不是悄悄成功——
/// 悄悄成功 = 假闸门（铁律：未命中须显式失败）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultKind {
    /// 超时（`docs/07` §4.2.1 `timeout`）。
    Timeout,
    /// 断链（`docs/07` §4.2.1 `peer_offline_midtask`）。
    LinkDropped,
    /// 对端拒收（`docs/07` §4.2.1 `delegation_rejected`）。
    Rejected,
    /// 对端不可达（`docs/07` §4.2.1 `peer_unreachable`）。
    PeerUnreachable,
    /// 对端撤销了我方 key（`docs/07` §4.2.1 `peer_revoked`）。
    PeerRevoked,
    /// 委派链成环（`docs/07` §4.2.1 `chain_too_deep` / 环）。
    CycleDetected,
    /// 成员专属错误（用于脚本化自定义文案）。
    Member(String),
}

impl FaultKind {
    /// 转成契约层错误。
    ///
    /// ⚠️ **变体到错误码的映射是有语义的**，不是随手挑的：
    /// `Rejected` → `Forbidden`（对端策略拒绝，是权限问题不是传输问题）；
    /// 其余网络类 → `Provider`（可重试，见 `AdapterError::is_retryable`）。
    /// 若映射反了，编排层的降级链（`docs/07` §4.3）会走错分支。
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

/// 一步脚本：成功或故障。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// 成员成功返回该结果。
    Outcome(MemberOutcome),
    /// 成员返回该故障。
    Fault(FaultKind),
}

/// mock 的编程接口（构造期配置）。
#[derive(Debug, Default)]
pub struct MockMemberExecutor {
    script: Mutex<Vec<Step>>,
    calls: Mutex<Vec<CallRecord>>,
    aborted: Mutex<BTreeMap<MemberId, AbortScope>>,
}

impl MockMemberExecutor {
    /// 构造空脚本的 mock。
    pub fn new() -> Self {
        Self::default()
    }

    /// 压入一步脚本（`Vec` 的顺序即执行顺序）。
    pub fn push(&self, step: Step) -> &Self {
        self.lock_script().push(step);
        self
    }

    /// 压入成功步骤。
    pub fn push_ok(&self, member: MemberId, output: &str) -> &Self {
        self.push(Step::Outcome(
            MemberOutcome::done(member, "已完成", output).expect("测试用结果应合法"),
        ))
    }

    /// 压入故障步骤。
    pub fn push_fault(&self, kind: FaultKind) -> &Self {
        self.push(Step::Fault(kind))
    }

    /// 调用记录（**按调用顺序**）。
    pub fn calls(&self) -> Vec<CallRecord> {
        self.lock_calls().clone()
    }

    /// 调用次数（"已检查 N 个"用，避免与"0 次"混淆）。
    pub fn call_count(&self) -> usize {
        self.lock_calls().len()
    }

    /// 某方法被调用了几次（`"start"` / `"steer"` / `"abort"`）。
    pub fn count_of(&self, method: &str) -> usize {
        self.lock_calls()
            .iter()
            .filter(|c| c.method() == method)
            .count()
    }

    /// 是否出现过「方法 A 在方法 B 之后」的有序对。
    ///
    /// 编排层的核心时序断言（「`steer` 在 `start` 之后」）用这个表达。
    pub fn called_in_order(&self, first: &str, second: &str) -> bool {
        let calls = self.lock_calls();
        let first_at = calls.iter().position(|c| c.method() == first);
        let second_at = calls.iter().position(|c| c.method() == second);
        match (first_at, second_at) {
            (Some(a), Some(b)) => a < b,
            // ⚠️ 「只有一端出现」**不是** true：没检查到的东西不能算通过。
            _ => false,
        }
    }

    /// 已中止成员 → scope（`AbortScope::StopRound` 不该出现在这里，见下）。
    pub fn aborted(&self) -> BTreeMap<MemberId, AbortScope> {
        self.lock_aborted().clone()
    }

    /// 曾被中止且**波及成员执行**的成员标识（排序后）。
    pub fn halted_members(&self) -> Vec<String> {
        self.lock_aborted()
            .iter()
            .filter(|(_, s)| s.halts_members())
            .map(|(m, _)| m.as_str().to_string())
            .collect()
    }

    /// 清空调用记录（用于分阶段断言）。
    pub fn clear_calls(&self) {
        self.lock_calls().clear();
    }

    fn lock_script(&self) -> std::sync::MutexGuard<'_, Vec<Step>> {
        // ⚠️ 不用 `unwrap_or_else(|e| e.into_inner())` 兜底：
        // 毒化的锁说明已有线程 panic 过，吞掉它就是「静默失败」。
        self.script.lock().expect("mock 脚本锁不应被毒化")
    }

    fn lock_calls(&self) -> std::sync::MutexGuard<'_, Vec<CallRecord>> {
        self.calls.lock().expect("mock 调用锁不应被毒化")
    }

    fn lock_aborted(&self) -> std::sync::MutexGuard<'_, BTreeMap<MemberId, AbortScope>> {
        self.aborted.lock().expect("mock 中止锁不应被毒化")
    }
}

/// 一次调用的记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    seq: u64,
    method: &'static str,
    member: Option<MemberId>,
    detail: String,
}

impl CallRecord {
    /// 全局序号（从 1 起，跨方法统一）。
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// 方法名（`"start"` / `"steer"` / `"abort"`）。
    pub fn method(&self) -> &'static str {
        self.method
    }

    /// 涉及��成员标识（`start` 由请求派生）。
    pub fn member(&self) -> Option<&MemberId> {
        self.member.as_ref()
    }

    /// 补充说明（`start` 是专家名，`steer` 是消息正文，`abort` 是 scope）。
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
        // ⚠️ `steer` 失败**不改变成员状态**（`docs/07` §0.1.1 保留 G2）：
        // 隧道 WS 断开 ≠ 成员执行失败。mock 只返回错误，不动 aborted 表。
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
    /// `steer` 的结果由「下一个未消费的脚本步」决定。
    fn next_steer_result(&self, member: &MemberId) -> Result<(), AdapterError> {
        let mut script = self.lock_script();
        match script.first().cloned() {
            Some(Step::Fault(f)) => {
                script.remove(0);
                Err(f.to_error(member))
            }
            Some(Step::Outcome(_)) => {
                // `start` 的成功步若被 steer 取走，脚本就会错位。
                // 显式报错而不是静默跳过：错位必须被看见。
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

/// 便捷构造：`[节点, 任务]` → `Vec<ChainHop>`（供请求组装）。
pub fn chain(pairs: &[(&str, &str)]) -> Vec<ChainHop> {
    pairs
        .iter()
        .map(|(n, t)| ChainHop::new(*n, *t).expect("测试用链跳应合法"))
        .collect()
}

/// 便捷构造：一个成功的 `MemberOutcome`。
pub fn outcome(
    member: &MemberId,
    status: MemberStatus,
    scope: &str,
    output: &str,
) -> MemberOutcome {
    quill_adapters::MemberOutcome::new(member.clone(), status, scope, output)
        .expect("测试用结果应合法")
}
