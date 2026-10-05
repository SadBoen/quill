//! 派工编排的**真实行为**测试。
//!
//! 覆盖任务书点名的四类场景：
//! 1. 正常路径（成员交付内容）；
//! 2. 成员拒绝 / 超时 / 链路断开（`FaultKind` 的前三个网络失败模式）；
//! 3. 幂等重放（同一轮重跑不重复调用执行器）；
//! 4. 崩溃恢复（`PENDING` 可重派 vs `RUNNING` 需人工确认）。
//!
//! ⚠️ **为什么断言来自返回值而不是输出文本**（铁律十九）：
//! 每条断言都打在 `DispatchReport` / `MemDispatchLedger` / `MockMemberExecutor`
//! 的结构化返回值上。`report.summary()` 只在「文案必须带 doctor 命令」
//! 这一条里被断言 —— 因为那一断言的**对象就是文案本身**。
//!
//! ⚠️ **为什么用 `quill-testkit` 的 `MockMemberExecutor`**：
//! `V1_SCOPE_CONSTRAINTS.md` §五「测试是唯一防线」要求失败模式能在测试里复现。
//! 自己再造一个 mock 就等于让实现与 fixture 同源漂移（铁律十五）。

use std::sync::Arc;

use quill_adapters::{
    AbortScope, ChainHop, ExpertId, MemberId, MemberOutcome, MemberStatus, SessionId, UserId,
};
use quill_agent::{
    AgentError, DispatchKey, DispatchLedger, DispatchRecord, DispatchState, DispatchTask,
    Dispatcher, MemDispatchLedger, MemberResult, RoundPrefix, RoundRequest, SharedExecutor,
};
use quill_domain::team::{roster, team_of};
use quill_domain::Team;
use quill_testkit::mock_member::{chain, FaultKind, MockMemberExecutor, Step};

const ROOM: &str = "growth-room";

fn u(n: u8) -> UserId {
    UserId::from_bytes([n; 16])
}

fn s(n: u8) -> SessionId {
    SessionId::from_bytes([n; 16])
}

fn e(name: &str) -> ExpertId {
    ExpertId::parse(name).expect("测试用专家名应合法")
}

/// 建一个含 3 名成员的团队（leader + 2 成员）。
fn team3() -> Team {
    let mut t = team_of("growth-squad", "增长小队", "leader-bot").expect("应合法");
    let r = roster(&["cost-analyst", "growth-analyst", "leader-bot"]);
    for name in ["cost-analyst", "growth-analyst"] {
        t.add_member(e(name), &r).expect("加入应成功");
    }
    t
}

fn task(expert: &str, seq: u32) -> DispatchTask {
    DispatchTask::with_seq(
        e(expert),
        seq,
        format!("分析 {expert}"),
        format!("请完整分析 {expert} 的相关指标并给出结论"),
    )
    .expect("测试用派工单应合法")
}

/// 派工器类型别名：执行器经 [`SharedExecutor`] 包装成 `Arc`（见其文档）。
type TestDispatcher = Dispatcher<SharedExecutor<MockMemberExecutor>, MemDispatchLedger>;

/// 造一个派工器：给定脚本步骤。
///
/// 返回的 `Arc<MockMemberExecutor>` 与派工器内部**共享同一个 mock 实例** ——
/// 这样测试才能在派工之后断言调用次数与时序。
fn dispatcher(steps: Vec<Step>) -> (Arc<MockMemberExecutor>, TestDispatcher) {
    let m = Arc::new(MockMemberExecutor::new());
    for st in steps {
        m.push(st);
    }
    let d = Dispatcher::new(
        SharedExecutor::new(Arc::clone(&m)),
        MemDispatchLedger::new(),
    );
    (m, d)
}

/// 派一轮工（测试侧的便捷封装）。
///
/// ⚠️ 刻意**返回 `RoundRequest` 而不是直接派工**：让每个用例自己写
/// 字段名，才能让「字段写错」在测试里立刻看得见。
/// 同时避免出现一个 8 参数的测试辅助函数（clippy `too_many_arguments`）。
fn req<'a>(
    owner: UserId,
    session: SessionId,
    room: &'a str,
    team: &'a Team,
    round_no: u32,
    tasks: &'a [DispatchTask],
    chain: &'a [ChainHop],
) -> RoundRequest<'a> {
    RoundRequest {
        owner,
        session,
        team,
        room_id: room,
        round: round_no,
        tasks,
        chain,
    }
}

/// 常用组合：属主 u(1) / 会话 s(1) / 房间 `ROOM` / 链为空。
fn round<'a>(
    d: &TestDispatcher,
    team: &'a Team,
    round_no: u32,
    tasks: &'a [DispatchTask],
) -> Result<quill_agent::DispatchReport, AgentError> {
    d.dispatch_round(&req(u(1), s(1), ROOM, team, round_no, tasks, &[]))
}

fn ok_step(member: &str, output: &str) -> Step {
    Step::Outcome(
        MemberOutcome::done(MemberId::parse(member).expect("应合法"), "已完成", output)
            .expect("结果应合法"),
    )
}

// ─────────────────────── 正常路径 ───────────────────────

#[test]
fn all_members_deliver_and_report_carries_counts() {
    let (m, d) = dispatcher(vec![
        ok_step("cost-analyst-1", "成本集中在存储"),
        ok_step("growth-analyst-1", "增长来自自然流量"),
    ]);
    let report = round(
        &d,
        &team3(),
        0,
        &[task("cost-analyst", 1), task("growth-analyst", 1)],
    )
    .expect("派工应成功");

    assert_eq!(report.delivered_count(), 2, "已检查：2 名成员都应交付");
    assert_eq!(report.failed_count(), 0);
    assert!(!report.is_total_failure());
    assert!(report.skipped_as_duplicate.is_empty());
    assert!(report.recovered_for_retry.is_empty());
    assert_eq!(m.count_of("start"), 2, "已检查：执行器应被调用 2 次");
}

#[test]
fn partial_outcome_counts_as_delivered_with_its_completed_scope() {
    // 反向用例：把 partial 当失败会丢掉已完成的工作（docs/07 §2.3）。
    let partial = MemberOutcome::new(
        MemberId::parse("cost-analyst-1").expect("应合法"),
        MemberStatus::Partial,
        "完成成本结构分析，未取到实时账单",
        "部分结论：成本集中在存储",
    )
    .expect("结果应合法");
    let (_, d) = dispatcher(vec![Step::Outcome(partial)]);
    let report = round(&d, &team3(), 0, &[task("cost-analyst", 1)]).expect("派工应成功");

    assert_eq!(report.delivered_count(), 1, "partial 仍算交付");
    match &report.results[0] {
        MemberResult::Delivered { outcome, .. } => {
            assert_eq!(outcome.status(), MemberStatus::Partial);
            assert!(
                outcome.completed_scope().contains("未取到实时账单"),
                "须保留完成范围"
            );
        }
        other => panic!("应为 Delivered，实际 {other:?}"),
    }
}

#[test]
fn member_self_reported_failure_is_recorded_as_failed_not_delivered() {
    // 🔴 反向用例：成员自报 failed 却记成 DONE = 主持人把空产出当成果汇总。
    let failed = MemberOutcome::failed(
        MemberId::parse("cost-analyst-1").expect("应合法"),
        "只完成了成本结构分析",
    )
    .expect("结果应合法");
    let (_, d) = dispatcher(vec![Step::Outcome(failed)]);
    let report =
        round(&d, &team3(), 0, &[task("cost-analyst", 1)]).expect("派工本身应成功返回报告");

    assert_eq!(report.delivered_count(), 0, "自报失败不得算交付");
    assert_eq!(report.failed_count(), 1);
    assert!(report.is_total_failure());
    match &report.results[0] {
        MemberResult::Failed { error, .. } => {
            assert_eq!(error.code(), "member_reported_failure");
            assert!(!error.is_retryable(), "成员自报失败可能已做一半，不可重试");
        }
        other => panic!("应为 Failed，实际 {other:?}"),
    }
}

#[test]
fn dispatching_to_an_expert_outside_the_team_is_rejected_before_touching_the_executor() {
    // 反向用例：不校验成员关系 = 允许派工给团外专家，Team 就白建了。
    let (m, d) = dispatcher(vec![]);
    let err = round(&d, &team3(), 0, &[task("risk-reviewer", 1)]).expect_err("团外专家必须判红");
    assert!(
        matches!(err, AgentError::TeamInvalid(_)),
        "应为团队不变量错误：{err:?}"
    );
    assert_eq!(m.count_of("start"), 0, "已检查：执行器一次都不该被调用");
}

#[test]
fn one_invalid_task_aborts_the_whole_round_without_dispatching_any_member() {
    // 🔴 静默数据丢失用例：若校验是「边派边校验」，
    // 第 2 个成员非法时第 1 个已经执行完并落账，而函数返回 Err
    // 让调用方拿不到那条结果 —— 成员跑了、token 烧了、结果被丢掉，
    // 且账本里留下一条调用方不知道的 DONE。
    let (m, d) = dispatcher(vec![ok_step("cost-analyst-1", "会被丢掉的产出")]);
    let tasks = [task("cost-analyst", 1), task("risk-reviewer", 1)];
    let err = round(&d, &team3(), 0, &tasks).expect_err("团外专家必须判红");
    assert!(
        matches!(err, AgentError::TeamInvalid(_)),
        "应为团队错误：{err:?}"
    );
    assert_eq!(m.count_of("start"), 0, "已检查：合法成员也**不该**被执行");
    assert_eq!(
        d.ledger().len(),
        0,
        "已检查：整轮失败后账本不得留下任何记录"
    );
}

#[test]
fn a_cycle_in_a_later_task_also_prevents_every_earlier_dispatch() {
    // 与上一条同源：链检测也必须整轮前置。
    // ⚠️ 两个成员都必须在团内（`team3` 只有 cost/growth/leader），
    // 否则会先撞上「团外专家」判红，测的就不是链检测了。
    let (m, d) = dispatcher(vec![ok_step("cost-analyst-1", "会被丢掉的产出")]);
    let hops = chain(&[("growth-analyst", "t-001")]);
    let tasks = [task("cost-analyst", 1), task("growth-analyst", 1)];
    let err = d
        .dispatch_round(&req(u(1), s(1), ROOM, &team3(), 0, &tasks, &hops))
        .expect_err("成环必须判红");
    assert_eq!(err.code(), "chain_cycle");
    assert_eq!(m.count_of("start"), 0, "已检查：合法成员也**不该**被执行");
    assert_eq!(d.ledger().len(), 0, "已检查：账本不得留下任何记录");
}

#[test]
fn leader_may_receive_a_dispatch_even_though_it_is_not_in_the_member_set() {
    // 反向用例：Team 的 leader 不在 members 里（quill-domain 刻意如此），
    // 编排层若只查 has_member，主持人自己就派不到工。
    let (_, d) = dispatcher(vec![ok_step("leader-bot-1", "主持人执行完毕")]);
    let report = round(&d, &team3(), 0, &[task("leader-bot", 1)]).expect("leader 应可被派工");
    assert_eq!(report.delivered_count(), 1);
}

// ─────────────────────── 成员失败：拒绝 / 超时 / 断链 ───────────────────────

#[test]
fn rejection_timeout_and_link_drop_each_produce_a_distinct_failure() {
    // docs/07 §4.2.1 的三个网络失败模式必须**各自**被复现，
    // 且错误文案互不相同（否则排障时分不清根因）。
    let cases = [
        (FaultKind::Rejected, "member_rejected", false),
        (FaultKind::Timeout, "member_rejected", true),
        (FaultKind::LinkDropped, "member_rejected", true),
    ];
    let mut msgs = std::collections::BTreeSet::new();
    for (fault, want_code, want_retryable) in cases {
        let (_, d) = dispatcher(vec![Step::Fault(fault.clone())]);
        let report =
            round(&d, &team3(), 0, &[task("cost-analyst", 1)]).expect("派工应返回报告而非整体 Err");
        assert_eq!(report.failed_count(), 1, "{fault:?} 应产生 1 条失败");
        match &report.results[0] {
            MemberResult::Failed { member, error } => {
                assert_eq!(member.as_str(), "cost-analyst-1");
                assert_eq!(error.code(), want_code, "{fault:?} 的错误码不对");
                assert_eq!(
                    error.is_retryable(),
                    want_retryable,
                    "{fault:?} 的可重试标记不对（口径来自 AdapterError::is_retryable）"
                );
                assert!(
                    error.to_string().contains("cost-analyst-1"),
                    "{fault:?} 的错误须含成员标识：{error}"
                );
                msgs.insert(error.to_string());
            }
            other => panic!("{fault:?} 应产生 Failed，实际 {other:?}"),
        }
    }
    assert_eq!(
        msgs.len(),
        3,
        "已检查 3 个失败模式，3 条错误文案必须互不相同（否则分不清根因）"
    );
}

#[test]
fn failed_members_do_not_stop_the_others_in_the_same_round() {
    // 🔴 反向用例：一名成员失败就中断整轮 = 一次网络抖动废掉整个专家团。
    let (m, d) = dispatcher(vec![
        Step::Fault(FaultKind::Timeout),
        ok_step("growth-analyst-1", "增长来自自然流量"),
    ]);
    let report = round(
        &d,
        &team3(),
        0,
        &[task("cost-analyst", 1), task("growth-analyst", 1)],
    )
    .expect("派工应返回报告");

    assert_eq!(report.delivered_count(), 1, "第二个成员仍应交付");
    assert_eq!(report.failed_count(), 1);
    assert!(
        !report.is_total_failure(),
        "1 成 1 败**不是**全失败（这是 is_total_failure 存在的理由）"
    );
    assert_eq!(m.count_of("start"), 2, "两个成员都应被调用过");
}

#[test]
fn a_failed_dispatch_is_never_retried_automatically() {
    // 🔴 契约 docs/07 §4.3 + PHASE2 §七.4：派工失败**不自动重跑**。
    // 断言「执行器只被调用 1 次」—— 若有人加了重试循环，这条立刻红。
    let (m, d) = dispatcher(vec![Step::Fault(FaultKind::PeerUnreachable)]);
    let report = round(&d, &team3(), 0, &[task("cost-analyst", 1)]).expect("派工应返回报告");
    assert_eq!(report.failed_count(), 1);
    assert_eq!(
        m.count_of("start"),
        1,
        "已检查：失败后不得自动重跑（副作用不可逆）"
    );
}

#[test]
fn revoked_authorization_failure_carries_its_own_code() {
    let (_, d) = dispatcher(vec![Step::Fault(FaultKind::PeerRevoked)]);
    let report = round(&d, &team3(), 0, &[task("cost-analyst", 1)]).expect("派工应返回报告");
    match &report.results[0] {
        MemberResult::Failed { error, .. } => {
            assert_eq!(
                error.code(),
                "member_unauthorized",
                "鉴权失败必须与「对端拒收」区分（处置完全不同）"
            );
        }
        other => panic!("应为 Failed，实际 {other:?}"),
    }
}

#[test]
fn report_summary_carries_the_copyable_doctor_command_for_every_failure() {
    // 铁律七：汇总文本是最容易丢「下一步执行哪条命令」的地方。
    let (_, d) = dispatcher(vec![
        Step::Fault(FaultKind::Rejected),
        Step::Fault(FaultKind::LinkDropped),
    ]);
    let report = round(
        &d,
        &team3(),
        0,
        &[task("cost-analyst", 1), task("growth-analyst", 1)],
    )
    .expect("派工应返回报告");
    let sum = report.summary();
    assert!(sum.contains("quill doctor"), "汇总须指向 doctor：\n{sum}");
    assert!(sum.contains("member_rejected"), "汇总须含错误码：\n{sum}");
    // ⚠️ 两条失败 → 汇总里恰好两条命令。
    // 修复命令的唯一出口是 `AgentError::Display` 的尾部，
    // 汇总**不得**再打一遍（两处同形命令会被用户当成两个不同动作）。
    assert_eq!(
        sum.matches("quill doctor").count(),
        2,
        "每条失败恰好带一条修复命令（不得重复）：\n{sum}"
    );
    assert!(report.is_total_failure());
}

// ─────────────────────── 幂等（ux_dispatch_once） ───────────────────────

#[test]
fn replaying_the_same_round_never_calls_the_executor_twice() {
    // 🔴 幂等判据：返回值层面「执行器只被调用 1 次」，
    // 不是「报告看起来一样」。
    let (m, d) = dispatcher(vec![ok_step("cost-analyst-1", "成本集中在存储")]);
    let t = team3();
    let tasks = [task("cost-analyst", 1)];

    let first = round(&d, &t, 0, &tasks).expect("首次派工应成功");
    assert_eq!(first.delivered_count(), 1);
    assert_eq!(m.count_of("start"), 1);

    // 同一轮重放：脚本已耗尽，若编排层再调一次执行器就会拿到 Conflict。
    let second = round(&d, &t, 0, &tasks).expect("重放应幂等返回");
    assert_eq!(
        m.count_of("start"),
        1,
        "已检查：重放同一轮不得重复调用执行器"
    );
    assert!(second.results.is_empty(), "重放的结果应为空（没有新执行）");
    assert_eq!(
        second.skipped_as_duplicate,
        vec![MemberId::parse("cost-analyst-1").expect("应合法")],
        "重放的成员应记为幂等跳过"
    );
}

#[test]
fn a_different_round_is_a_different_dispatch_and_does_run() {
    // 反向用例：幂等键若漏了 round（或误用 member 而非 expert），
    // 第二轮会被错误跳过 —— 一次静默的任务丢失。
    let (m, d) = dispatcher(vec![
        ok_step("cost-analyst-1", "第 0 轮结论"),
        ok_step("cost-analyst-2", "第 1 轮结论"),
    ]);
    let t = team3();
    let tasks = [task("cost-analyst", 1)];
    round(&d, &t, 0, &tasks).expect("第 0 轮应成功");
    let second = round(&d, &t, 1, &tasks).expect("第 1 轮应成功");
    assert_eq!(second.delivered_count(), 1, "新轮次必须真的执行");
    assert_eq!(m.count_of("start"), 2, "已检查：两轮共 2 次调用");
}

#[test]
fn a_different_room_is_a_different_dispatch_and_does_run() {
    let (m, d) = dispatcher(vec![
        ok_step("cost-analyst-1", "A 房结论"),
        ok_step("cost-analyst-1", "B 房结论"),
    ]);
    let t = team3();
    let tasks = [task("cost-analyst", 1)];
    d.dispatch_round(&req(u(1), s(1), "room-a", &t, 0, &tasks, &[]))
        .expect("A 房应成功");
    let second = d
        .dispatch_round(&req(u(1), s(1), "room-b", &t, 0, &tasks, &[]))
        .expect("B 房应成功");
    assert_eq!(second.delivered_count(), 1, "新房间必须真的执行");
    assert_eq!(m.count_of("start"), 2);
}

#[test]
fn the_same_room_and_round_are_isolated_between_users() {
    // 🔴 跨用户隔离：键的第一列是 user_id。
    // 若漏掉，A 与 B 在同名房间的同一轮会互相命中幂等键 = 一方静默收不到结果。
    let (m, d) = dispatcher(vec![
        ok_step("cost-analyst-1", "A 的结论"),
        ok_step("cost-analyst-1", "B 的结论"),
    ]);
    let t = team3();
    let tasks = [task("cost-analyst", 1)];
    let a = round(&d, &t, 0, &tasks).expect("A 应成功");
    let b = d
        .dispatch_round(&req(u(2), s(2), ROOM, &t, 0, &tasks, &[]))
        .expect("B 应成功");
    assert_eq!(a.delivered_count(), 1);
    assert_eq!(b.delivered_count(), 1, "B 不得被 A 的幂等键挡掉");
    assert_eq!(m.count_of("start"), 2, "已检查：两名用户各执行 1 次");
    assert_eq!(d.ledger().len(), 2, "账本应有 2 条记录");
}

#[test]
fn two_dispatches_in_one_round_target_different_members_and_both_run() {
    let (m, d) = dispatcher(vec![
        ok_step("cost-analyst-1", "A 结论"),
        ok_step("growth-analyst-1", "B 结论"),
    ]);
    let report = round(
        &d,
        &team3(),
        0,
        &[task("cost-analyst", 1), task("growth-analyst", 1)],
    )
    .expect("派工应成功");
    assert_eq!(report.delivered_count(), 2, "同一轮两个成员都要跑");
    assert_eq!(m.count_of("start"), 2);
    assert_eq!(d.ledger().len(), 2, "账本按 (room, round, expert) 记 2 条");
}

// ─────────────────────── 崩溃恢复（ix_dispatch_inflight） ───────────────────────

#[test]
fn begin_distinguishes_created_from_existed() {
    // 🔴 幂等的**判别**载体：若 begin() 只返回记录而不说「是不是我插的」，
    // 编排层就无法区分「正常首派」与「崩溃恢复」，只能把两者混为一谈
    // —— 那会让每一轮正常派工都被报成「恢复重派」。
    let (_, d) = dispatcher(vec![]);
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    let rec = || {
        DispatchRecord::pending(
            key.clone(),
            MemberId::parse("cost-analyst-1").expect("应合法"),
        )
    };

    let first = d.ledger().begin(&rec()).expect("首次应成功");
    assert!(first.is_created(), "键不存在时必须判 Created");
    let second = d.ledger().begin(&rec()).expect("二次应幂等返回而非报错");
    assert!(!second.is_created(), "键已存在时必须判 Existed");
    assert_eq!(
        first.record().key(),
        second.record().key(),
        "两次返回的必须是同一条记录（幂等的核心）"
    );
    assert_eq!(d.ledger().len(), 1, "已检查：账本仍只有 1 条");
}

#[test]
fn a_fresh_dispatch_is_not_reported_as_a_crash_recovery() {
    // 🔴 反向用例：正常首派不得被标成「恢复重派」。
    // 若 begin() 的判别丢失，每一轮正常派工都会往 recovered_for_retry 里塞一条，
    // 上层据此提示用户「上次有派工没跑完」—— 一个每次都误报的健康告警。
    let (_, d) = dispatcher(vec![ok_step("cost-analyst-1", "首派成功")]);
    let report = round(&d, &team3(), 0, &[task("cost-analyst", 1)]).expect("首派应成功");
    assert_eq!(report.delivered_count(), 1);
    assert!(
        report.recovered_for_retry.is_empty(),
        "首派不是恢复重派：{:?}",
        report.recovered_for_retry
    );
}

#[test]
fn a_replayed_pending_dispatch_is_reported_as_a_recovery() {
    // 与上一条相反的方向：键已存在且仍是 PENDING 才算恢复重派。
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    let (_, d) = dispatcher(vec![ok_step("cost-analyst-1", "恢复后跑成功")]);
    d.ledger()
        .begin(&DispatchRecord::pending(
            key.clone(),
            MemberId::parse("cost-analyst-1").expect("应合法"),
        ))
        .expect("记账应成功");
    let report = round(&d, &team3(), 0, &[task("cost-analyst", 1)]).expect("PENDING 应可重派");
    assert_eq!(report.recovered_for_retry, vec![key]);
    assert_eq!(report.delivered_count(), 1);
}

#[test]
fn pending_dispatch_after_a_crash_is_safe_to_retry_and_does_rerun() {
    // PENDING = 已记账但**成员从未被调用** → 重派安全。
    let (m, d) = dispatcher(vec![ok_step("cost-analyst-1", "崩溃后补跑成功")]);
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    // 模拟「记账后进程被杀」：只有一条 PENDING 记录，执行器一次都没被调用。
    let begun = d
        .ledger()
        .begin(&DispatchRecord::pending(
            key.clone(),
            MemberId::parse("cost-analyst-1").expect("应合法"),
        ))
        .expect("记账应成功");
    assert!(begun.is_created(), "首次记账必须是 Created");
    assert_eq!(m.count_of("start"), 0, "已检查：崩溃前执行器未被调用");

    let rec = d.recover(&u(1)).expect("恢复判定应成功");
    assert_eq!(rec.checked, 1, "已检查：1 条在途派工");
    assert_eq!(
        rec.safe_to_retry,
        vec![key.clone()],
        "PENDING 应判为可安全重派"
    );
    assert!(rec.needs_confirmation.is_empty());
    assert!(!rec.is_clean(), "有在途派工就不算干净");

    let report = round(&d, &team3(), 0, &[task("cost-analyst", 1)]).expect("PENDING 应可重派");
    assert_eq!(report.delivered_count(), 1, "重派后应拿到结果");
    assert_eq!(
        report.recovered_for_retry,
        vec![key],
        "报告须标明这是恢复重派"
    );
    assert_eq!(m.count_of("start"), 1, "重派才第一次调用执行器");
}

#[test]
fn running_dispatch_after_a_crash_requires_human_confirmation_and_is_not_rerun() {
    // 🔴 RUNNING = 执行器**已被调用**，副作用可能已发生 → 不得自动重派。
    // 若这里重派，就是一次重复执行（成员可能已写过文件/库）。
    let (m, d) = dispatcher(vec![ok_step("cost-analyst-1", "不该被调用")]);
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    let mut rec = DispatchRecord::pending(
        key.clone(),
        MemberId::parse("cost-analyst-1").expect("应合法"),
    );
    rec.mark_running().expect("PENDING → RUNNING 应合法");
    d.ledger().put(&rec).expect("写入应成功");

    let recovery = d.recover(&u(1)).expect("恢复判定应成功");
    assert_eq!(recovery.checked, 1);
    assert!(recovery.safe_to_retry.is_empty(), "RUNNING 不得判为可重派");
    assert_eq!(
        recovery.needs_confirmation,
        vec![key],
        "RUNNING 必须要求人工确认"
    );
    assert!(
        recovery.summary().contains("不自动重跑"),
        "摘要必须说明为什么不自动重跑：{}",
        recovery.summary()
    );

    let report = round(&d, &team3(), 0, &[task("cost-analyst", 1)]).expect("派工应返回报告");
    assert!(report.delivered_count().eq(&0), "RUNNING 不得被自动重派");
    assert_eq!(m.count_of("start"), 0, "已检查：执行器一次都不该被调用");
    assert_eq!(report.skipped_as_duplicate.len(), 1, "应记为跳过");
}

#[test]
fn asking_dispatch_also_requires_human_confirmation() {
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    let mut rec = DispatchRecord::pending(
        key.clone(),
        MemberId::parse("cost-analyst-1").expect("应合法"),
    );
    rec.mark_asking(1).expect("PENDING → ASKING 应合法");
    let (_, d) = dispatcher(vec![]);
    d.ledger().put(&rec).expect("写入应成功");

    let recovery = d.recover(&u(1)).expect("恢复判定应成功");
    assert_eq!(
        recovery.needs_confirmation,
        vec![key],
        "ASKING 同样需人工确认"
    );
    assert!(recovery.safe_to_retry.is_empty());
}

#[test]
fn recovery_of_a_clean_ledger_reports_zero_checked_not_unknown() {
    // 🔴 铁律十六：「0 条在途」与「根本没查到」在屏幕上必须能区分。
    let (_, d) = dispatcher(vec![]);
    let rec = d.recover(&u(1)).expect("恢复判定应成功");
    assert_eq!(rec.checked, 0, "已检查：确实 0 条在途");
    assert!(rec.is_clean());
    assert!(
        rec.summary().contains("已检查 0 条"),
        "摘要必须显式写出检查了几个：{}",
        rec.summary()
    );
}

#[test]
fn recovery_only_sees_the_requesting_users_inflight_dispatches() {
    // 🔴 跨用户隔离：A 的悬挂派工不得出现在 B 的恢复报告里，
    // 否则 B 会看到别人的任务摘要。
    let (_, d) = dispatcher(vec![]);
    for owner in [u(1), u(2)] {
        let key = DispatchKey::new(owner, ROOM, 0, e("cost-analyst")).expect("键应合法");
        d.ledger()
            .begin(&DispatchRecord::pending(
                key,
                MemberId::parse("cost-analyst-1").expect("应合法"),
            ))
            .expect("记账应成功");
    }
    let a = d.recover(&u(1)).expect("A 的恢复应成功");
    assert_eq!(a.checked, 1, "已检查：A 只应看到自己那 1 条");
    assert_eq!(a.safe_to_retry.len(), 1);
    assert!(a.safe_to_retry[0].owner() == u(1), "键的属主必须是 A");
}

// ─────────────────────── 状态机 ───────────────────────

#[test]
fn a_settled_dispatch_cannot_be_settled_again() {
    // 反向用例：重复结算会让**先到的结果被后到的覆盖** = 静默数据丢失。
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    let mut rec = DispatchRecord::pending(key, MemberId::parse("cost-analyst-1").expect("应合法"));
    rec.mark_running().expect("应合法");
    let outcome = MemberOutcome::done(
        MemberId::parse("cost-analyst-1").expect("应合法"),
        "已完成",
        "结论",
    )
    .expect("应合法");
    rec.settle_done(outcome.clone()).expect("首次结算应成功");
    let err = rec.settle_done(outcome).expect_err("二次结算必须判红");
    assert!(
        matches!(err, AgentError::DispatchIllegalTransition { .. }),
        "应为非法跃迁：{err:?}"
    );
    assert!(err.to_string().contains("已结算"), "错误须说明原因：{err}");
}

#[test]
fn a_failed_outcome_cannot_be_settled_as_done() {
    // 🔴 反向用例：成员自报 failed 却记 DONE = 主持人汇总到空产出。
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    let mut rec = DispatchRecord::pending(key, MemberId::parse("cost-analyst-1").expect("应合法"));
    rec.mark_running().expect("应合法");
    let failed = MemberOutcome::failed(
        MemberId::parse("cost-analyst-1").expect("应合法"),
        "只完成了一半",
    )
    .expect("应合法");
    let err = rec.settle_done(failed).expect_err("失败结果不得记成 DONE");
    assert!(
        matches!(err, AgentError::DispatchIllegalTransition { .. }),
        "应为非法跃迁：{err:?}"
    );
}

#[test]
fn running_cannot_be_entered_from_a_settled_state() {
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    let mut rec = DispatchRecord::pending(key, MemberId::parse("cost-analyst-1").expect("应合法"));
    rec.mark_running().expect("应合法");
    rec.settle_cancelled().expect("结算应成功");
    assert!(rec.mark_running().is_err(), "终态不得回到 RUNNING");
}

#[test]
fn asking_requires_a_positive_depth() {
    // 反向用例：schema 有 CHECK ((state='ASKING') = (ask_depth > 0))，
    // 深度 0 进 ASKING 会让写库当场失败。
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    let mut rec = DispatchRecord::pending(key, MemberId::parse("cost-analyst-1").expect("应合法"));
    let err = rec.mark_asking(0).expect_err("深度 0 必须判红");
    assert!(
        err.to_string().contains("ask_depth"),
        "错误须点名该字段：{err}"
    );
    rec.mark_asking(2).expect("深度 2 应合法");
    assert_eq!(rec.ask_depth(), 2);
    assert_eq!(rec.state(), DispatchState::Asking);
}

#[test]
fn dispatch_state_wire_names_match_the_schema_check_list() {
    // 逐字对齐 `task_dispatches.state` 的 CHECK，6 个值不多不少。
    let all = [
        (DispatchState::Pending, "PENDING"),
        (DispatchState::Running, "RUNNING"),
        (DispatchState::Asking, "ASKING"),
        (DispatchState::Done, "DONE"),
        (DispatchState::Failed, "FAILED"),
        (DispatchState::Cancelled, "CANCELLED"),
    ];
    for (s, w) in all {
        assert_eq!(s.as_wire(), w);
        assert_eq!(DispatchState::from_wire(w), Some(s));
    }
    assert_eq!(all.len(), 6);
    assert_eq!(
        DispatchState::from_wire("done"),
        None,
        "大小写不符必须解析失败"
    );
    assert_eq!(
        DispatchState::from_wire("PENDINGX"),
        None,
        "未知值必须解析失败"
    );
}

#[test]
fn inflight_and_terminal_partition_the_six_states_without_gaps() {
    // 反照 ix_dispatch_inflight 的部分索引条件。
    let all = [
        DispatchState::Pending,
        DispatchState::Running,
        DispatchState::Asking,
        DispatchState::Done,
        DispatchState::Failed,
        DispatchState::Cancelled,
    ];
    let inflight: Vec<_> = all.iter().filter(|s| s.is_inflight()).collect();
    let terminal: Vec<_> = all.iter().filter(|s| s.is_terminal()).collect();
    assert_eq!(inflight.len(), 3, "在途：PENDING/RUNNING/ASKING");
    assert_eq!(terminal.len(), 3, "终态：DONE/FAILED/CANCELLED");
    assert_eq!(inflight.len() + terminal.len(), 6, "六态必须被完全划分");
    for s in all {
        assert_ne!(s.is_inflight(), s.is_terminal(), "{s} 必须恰属于一类");
    }
}

// ─────────────────────── 委派链环防护 ───────────────────────

#[test]
fn dispatching_an_expert_already_on_the_chain_is_rejected_before_the_executor_runs() {
    // A→B→A：B 派 A 必须被检出，且**在触达执行器之前**。
    let (m, d) = dispatcher(vec![]);
    let hops = chain(&[("node-a", "t-001"), ("cost-analyst", "t-002")]);
    let err = d
        .dispatch_round(&req(
            u(1),
            s(1),
            ROOM,
            &team3(),
            0,
            &[task("cost-analyst", 1)],
            &hops,
        ))
        .expect_err("成环必须判红");
    assert_eq!(err.code(), "chain_cycle");
    assert_eq!(m.count_of("start"), 0, "已检查：环检测在执行器之前");
    assert_eq!(d.ledger().len(), 0, "环检测失败不得留下记账");
}

#[test]
fn chain_deeper_than_the_limit_is_rejected() {
    let (m, d) = dispatcher(vec![]);
    // MAX_CHAIN_DEPTH = 4 → 已有 4 跳时再派第 5 跳必须判 TooDeep。
    let hops: Vec<ChainHop> = (0..4)
        .map(|i| ChainHop::new(format!("node-{i}"), format!("t-{i}")).expect("应合法"))
        .collect();
    let err = d
        .dispatch_round(&req(
            u(1),
            s(1),
            ROOM,
            &team3(),
            0,
            &[task("cost-analyst", 1)],
            &hops,
        ))
        .expect_err("超深必须判红");
    assert_eq!(err.code(), "chain_too_deep");
    assert!(err.to_string().contains('4'), "错误须含上限：{err}");
    assert_eq!(m.count_of("start"), 0);
}

#[test]
fn a_legal_chain_passes_through_and_is_forwarded_to_the_request() {
    let (m, d) = dispatcher(vec![ok_step("cost-analyst-1", "带链派工成功")]);
    let hops = chain(&[("node-a", "t-001")]);
    let report = d
        .dispatch_round(&req(
            u(1),
            s(1),
            ROOM,
            &team3(),
            0,
            &[task("cost-analyst", 1)],
            &hops,
        ))
        .expect("合法链应放行");
    assert_eq!(report.delivered_count(), 1);
    assert_eq!(m.count_of("start"), 1);
}

// ─────────────────────── steer / abort ───────────────────────

#[test]
fn steer_failure_is_reported_but_does_not_settle_the_dispatch() {
    // 🔴 docs/07 §0.1.1 保留 G2：隧道断开 ≠ 成员执行失败。
    let (_, d) = dispatcher(vec![Step::Fault(FaultKind::LinkDropped)]);
    let member = MemberId::parse("cost-analyst-1").expect("应合法");
    let err = d
        .steer(&member, "补充：请附上数据来源")
        .expect_err("应失败");
    assert_eq!(err.code(), "member_rejected");
    assert!(
        d.ledger()
            .get(&DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法"))
            .expect("查询应成功")
            .is_none(),
        "steer 失败不得凭空产生一条派工记录"
    );
}

#[test]
fn stop_round_does_not_halt_members_but_abort_room_does() {
    // 契约七.2：停主持人**不能**连带停成员。
    let (m, d) = dispatcher(vec![]);
    let a = MemberId::parse("cost-analyst-1").expect("应合法");
    let b = MemberId::parse("growth-analyst-1").expect("应合法");

    d.abort(&a, AbortScope::StopRound)
        .expect("StopRound 应成功");
    assert!(m.halted_members().is_empty(), "StopRound 不得停成员");

    d.abort(&b, AbortScope::AbortRoom)
        .expect("AbortRoom 应成功");
    assert_eq!(
        m.halted_members(),
        vec!["growth-analyst-1".to_string()],
        "AbortRoom 必须停成员"
    );
}

#[test]
fn steer_with_blank_text_is_rejected_before_reaching_the_executor() {
    // 反向用例：空 steer 若被放行，成员会收到一次「什么都没有」的唤醒，
    // 白烧一轮 token 且在时间线上留下不可解释的空洞。
    let (m, d) = dispatcher(vec![]);
    let member = MemberId::parse("cost-analyst-1").expect("应合法");
    let err = d.steer(&member, "   ").expect_err("空白消息必须判红");
    assert_eq!(err.code(), "dispatch_request_invalid");
    assert_eq!(m.count_of("steer"), 0, "已检查：执行器未被调用");
}

// ─────────────────────── 派工单校验 ───────────────────────

#[test]
fn a_task_with_a_member_from_another_expert_is_rejected() {
    // 反向用例：把 A 专家的任务派给 B 的成员实例 = 串号。
    let err = DispatchTask::new(
        e("cost-analyst"),
        MemberId::parse("growth-analyst-1").expect("应合法"),
        "标题",
        "正文",
    )
    .expect_err("成员与专家不匹配必须判红");
    assert_eq!(err.code(), "dispatch_request_invalid");
    assert!(
        err.to_string().contains("growth-analyst-1"),
        "须点名成员：{err}"
    );
}

#[test]
fn a_task_with_empty_title_or_instructions_is_rejected() {
    for (title, instr) in [("", "正文"), ("  ", "正文"), ("标题", ""), ("标题", " \n ")] {
        let err = DispatchTask::new(
            e("cost-analyst"),
            MemberId::parse("cost-analyst-1").expect("应合法"),
            title,
            instr,
        )
        .expect_err("空标题/正文必须判红");
        assert_eq!(
            err.code(),
            "dispatch_request_invalid",
            "标题 {title:?} / 正文 {instr:?} 应判红"
        );
    }
}

#[test]
fn a_blank_room_id_is_rejected() {
    // 反向用例：房间标识为空 = 崩溃恢复时找不到这条派工。
    let err = DispatchKey::new(u(1), "  ", 0, e("cost-analyst")).expect_err("空房间应判红");
    assert_eq!(err.code(), "dispatch_request_invalid");
    assert!(err.to_string().contains("房间"), "须点名房间：{err}");
}

#[test]
fn dispatch_key_display_is_readable_for_logs() {
    let key = DispatchKey::new(u(1), ROOM, 3, e("cost-analyst")).expect("键应合法");
    let s = key.to_string();
    assert!(s.contains(ROOM), "须含房间：{s}");
    assert!(s.contains("round-3"), "须含轮次：{s}");
    assert!(s.contains("cost-analyst"), "须含专家：{s}");
}

#[test]
fn list_round_returns_records_sorted_by_member_expert() {
    // 反照 `ix_dispatch_settle`：按 (user_id, room_id, state) 查，
    // 编排层要求结果可复现 → 必须排序。
    let (_, d) = dispatcher(vec![]);
    let prefix = RoundPrefix {
        owner: u(1),
        room_id: ROOM.to_string(),
        round: 0,
    };
    for name in ["growth-analyst", "cost-analyst"] {
        let key = DispatchKey::new(u(1), ROOM, 0, e(name)).expect("键应合法");
        let member = format!("{name}-1");
        d.ledger()
            .begin(&DispatchRecord::pending(
                key,
                MemberId::parse(&member).expect("应合法"),
            ))
            .expect("记账应成功");
    }
    let got = d.ledger().list_round(&prefix).expect("查询应成功");
    let names: Vec<&str> = got
        .iter()
        .map(|r| r.key().member_expert().as_str())
        .collect();
    assert_eq!(
        names,
        vec!["cost-analyst", "growth-analyst"],
        "必须按 expert 序"
    );
}

#[test]
fn list_round_of_another_users_room_is_empty_not_their_data() {
    // 🔴 跨用户隔离：按前缀查也必须带 owner。
    let (_, d) = dispatcher(vec![]);
    let key = DispatchKey::new(u(1), ROOM, 0, e("cost-analyst")).expect("键应合法");
    d.ledger()
        .begin(&DispatchRecord::pending(
            key,
            MemberId::parse("cost-analyst-1").expect("应合法"),
        ))
        .expect("记账应成功");
    let b_prefix = RoundPrefix {
        owner: u(2),
        room_id: ROOM.to_string(),
        round: 0,
    };
    assert!(
        d.ledger()
            .list_round(&b_prefix)
            .expect("查询应成功")
            .is_empty(),
        "B 不得看到 A 的派工"
    );
}
