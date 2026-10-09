use std::sync::Arc;

use quill_adapters::{
    check_chain, AbortScope, AdapterError, ChainCheck, ExpertId, MemberExecutor, MemberId,
    MemberOutcome, MemberStartRequest, MemberStatus, Message, SessionId, UserId,
};
use quill_testkit::mock_member::{chain, FaultKind, MockMemberExecutor, Step};

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
    panic!("future 在 10000 次轮询后仍未完成：mock 的 future 不应挂起");
}

fn u() -> UserId {
    UserId::from_bytes([1; 16])
}

fn s() -> SessionId {
    SessionId::from_bytes([2; 16])
}

fn expert(n: &str) -> ExpertId {
    ExpertId::parse(n).expect("测试用专家名应合法")
}

fn member(n: &str) -> MemberId {
    MemberId::parse(n).expect("测试用成员标识应合法")
}

fn req(expert_name: &str, member_name: &str) -> MemberStartRequest {
    MemberStartRequest::new(
        u(),
        s(),
        expert(expert_name),
        member(member_name),
        "分析成本",
        "请分析 Q3 成本结构并给出建议",
    )
    .expect("测试用请求应合法")
}

fn start(
    m: &Arc<MockMemberExecutor>,
    expert_name: &str,
    member_name: &str,
) -> Result<MemberOutcome, AdapterError> {
    block_on(m.start(req(expert_name, member_name)))
}

fn steer(m: &Arc<MockMemberExecutor>, member_name: &str, text: &str) -> Result<(), AdapterError> {
    let msg = Message::user(text).expect("测试用消息应合法");
    block_on(m.steer(&UserId::from_bytes([9; 16]), &member(member_name), msg))
}

fn abort(
    m: &Arc<MockMemberExecutor>,
    member_name: &str,
    scope: AbortScope,
) -> Result<(), AdapterError> {
    block_on(m.abort(&UserId::from_bytes([9; 16]), &member(member_name), scope))
}

#[test]
fn start_returns_scripted_outcome_and_records_call() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_ok(member("cost-analyst-1"), "结论：降 12%");

    let out = start(&m, "cost-analyst", "cost-analyst-1").expect("应成功");

    assert_eq!(out.status(), MemberStatus::Done, "状态须为 done");
    assert_eq!(out.output(), "结论：降 12%", "产出须逐字返回脚本内容");
    assert_eq!(
        out.member().as_str(),
        "cost-analyst-1",
        "成员标识须原样返回"
    );

    assert_eq!(m.call_count(), 1, "已检查：start 应记录 1 次调用");
    let calls = m.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method(), "start");
    assert_eq!(calls[0].seq(), 1, "首个调用序号应为 1");
    assert_eq!(calls[0].detail(), "cost-analyst", "start 记录专家名");
    assert_eq!(
        calls[0].member().map(|x| x.as_str()),
        Some("cost-analyst-1"),
        "start 记录须带成员标识"
    );
}

#[test]
fn start_consumes_script_in_order() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_ok(member("cost-analyst-1"), "第一份产出")
        .push_ok(member("cost-analyst-2"), "第二份产出");

    let a = start(&m, "cost-analyst", "cost-analyst-1").expect("第一次应成功");
    let b = start(&m, "cost-analyst", "cost-analyst-2").expect("第二次应成功");

    assert_eq!(a.output(), "第一份产出", "第一次取脚本第 1 步");
    assert_eq!(b.output(), "第二份产出", "第二次取脚本第 2 步");
    assert_eq!(m.count_of("start"), 2, "已检查：共 2 次 start");
}

#[test]
fn steer_is_recorded_after_start_so_orchestration_can_assert_order() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_ok(member("cost-analyst-1"), "产出")
        .push_fault(FaultKind::LinkDropped);

    start(&m, "cost-analyst", "cost-analyst-1").expect("start 应成功");
    let steered = steer(&m, "cost-analyst-1", "补充：请附上数据来源");

    assert!(steered.is_err(), "steer 撞上 LinkDropped 脚本步，应失败");
    assert!(
        m.called_in_order("start", "steer"),
        "steer 必须发生在 start 之后"
    );
    assert!(
        !m.called_in_order("steer", "start"),
        "反向：实际顺序是 start→steer，不存在 steer→start"
    );
}

#[test]
fn abort_records_scope_and_room_scope_marks_member_halted() {
    let m = Arc::new(MockMemberExecutor::new());
    abort(&m, "cost-analyst-1", AbortScope::AbortRoom).expect("应成功");

    let calls = m.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method(), "abort");
    assert_eq!(calls[0].detail(), "AbortRoom", "abort 记录 scope");
    assert_eq!(
        m.halted_members(),
        vec!["cost-analyst-1".to_string()],
        "AbortRoom 必须让成员进入已中止状态"
    );
}

#[test]
fn stop_round_does_not_mark_member_as_halted() {
    let m = Arc::new(MockMemberExecutor::new());
    abort(&m, "cost-analyst-1", AbortScope::StopRound).expect("应成功");

    assert_eq!(m.call_count(), 1, "调用必须被记录（本次确实调了 abort）");
    assert!(
        m.halted_members().is_empty(),
        "StopRound 不得把成员标为已中止"
    );
    assert!(m.aborted().is_empty(), "StopRound 不得写入 aborted 表");
}

#[test]
fn sequence_numbers_are_contiguous_across_methods() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_ok(member("cost-analyst-1"), "产出")
        .push_fault(FaultKind::Timeout);
    start(&m, "cost-analyst", "cost-analyst-1").expect("start 应成功");
    let _ = steer(&m, "cost-analyst-1", "插一句");
    let _ = abort(&m, "cost-analyst-1", AbortScope::AbortRoom);

    let seqs: Vec<u64> = m.calls().iter().map(|c| c.seq()).collect();
    assert_eq!(seqs, vec![1, 2, 3], "序号必须跨方法连续且唯一");
    let methods: Vec<&str> = m.calls().iter().map(|c| c.method()).collect();
    assert_eq!(methods, vec!["start", "steer", "abort"], "调用顺序须可还原");
}

#[test]
fn each_network_fault_maps_to_a_distinct_error() {
    let m = Arc::new(MockMemberExecutor::new());
    let cases = [
        FaultKind::Timeout,
        FaultKind::LinkDropped,
        FaultKind::Rejected,
        FaultKind::PeerUnreachable,
        FaultKind::PeerRevoked,
        FaultKind::CycleDetected,
    ];
    assert_eq!(cases.len(), 6, "docs/07 §4.2.1 新增 6 个跨网络错误码");

    let mut seen = std::collections::BTreeSet::new();
    for fault in cases {
        m.push_fault(fault.clone());
        let err = start(&m, "cost-analyst", "cost-analyst-1")
            .err()
            .unwrap_or_else(|| panic!("{fault:?} 应导致 start 失败"));
        assert!(
            err.to_string().contains("cost-analyst-1"),
            "{fault:?} 的错误信息须含成员标识以便定位：{err}"
        );
        seen.insert(err.to_string());
    }
    assert_eq!(
        seen.len(),
        6,
        "已检查 6 个故障，6 个错误文案必须各不相同（否则排障时分不清根因）"
    );
}

#[test]
fn rejected_maps_to_forbidden_while_network_faults_map_to_provider() {
    let m = Arc::new(MockMemberExecutor::new());

    m.push_fault(FaultKind::Rejected);
    let rejected = start(&m, "cost-analyst", "cost-analyst-1").expect_err("应失败");
    assert!(
        matches!(rejected, AdapterError::Forbidden(_)),
        "拒收是策略拒绝（Forbidden），不是传输故障：{rejected}"
    );

    m.push_fault(FaultKind::PeerUnreachable);
    let unreachable = start(&m, "cost-analyst", "cost-analyst-1").expect_err("应失败");
    assert!(
        matches!(unreachable, AdapterError::Provider(_)),
        "不可达是传输故障（Provider）：{unreachable}"
    );

    m.push_fault(FaultKind::PeerRevoked);
    let revoked = start(&m, "cost-analyst", "cost-analyst-1").expect_err("应失败");
    assert!(
        matches!(revoked, AdapterError::Unauthorized(_)),
        "撤销我方 key 是鉴权失败（Unauthorized）：{revoked}"
    );

    m.push_fault(FaultKind::CycleDetected);
    let cycle = start(&m, "cost-analyst", "cost-analyst-1").expect_err("应失败");
    assert!(
        matches!(cycle, AdapterError::Conflict(_)),
        "环是状态冲突（Conflict）：{cycle}"
    );
}

#[test]
fn provider_faults_are_retryable_and_policy_faults_are_not() {
    let m = Arc::new(MockMemberExecutor::new());

    m.push_fault(FaultKind::Timeout);
    let timeout = start(&m, "cost-analyst", "cost-analyst-1").expect_err("应失败");
    assert!(timeout.is_retryable(), "超时应可重试：{timeout}");

    m.push_fault(FaultKind::LinkDropped);
    let dropped = start(&m, "cost-analyst", "cost-analyst-1").expect_err("应失败");
    assert!(dropped.is_retryable(), "断链在传输层可重试：{dropped}");

    m.push_fault(FaultKind::Rejected);
    let rejected = start(&m, "cost-analyst", "cost-analyst-1").expect_err("应失败");
    assert!(
        !rejected.is_retryable(),
        "对端拒收重试无意义（一次即失败，如实报告）：{rejected}"
    );
}

#[test]
fn steer_failure_does_not_change_member_state() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_fault(FaultKind::LinkDropped);

    let err = steer(&m, "cost-analyst-1", "插一句").expect_err("应失败");
    assert!(
        matches!(err, AdapterError::Provider(_)),
        "WS 断开是传输问题：{err}"
    );
    assert_eq!(m.count_of("steer"), 1, "steer 调用必须被记录");
    assert!(
        m.halted_members().is_empty(),
        "steer 失败不得把成员标为中止"
    );
    assert!(
        m.aborted().is_empty(),
        "steer 失败不得写入 aborted 表（成员仍在跑）"
    );
}

#[test]
fn custom_member_fault_keeps_its_message() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_fault(FaultKind::Member("模型上下文超长".into()));
    let err = start(&m, "cost-analyst", "cost-analyst-1").expect_err("应失败");
    assert_eq!(err.detail(), "模型上下文超长", "自定义文案须原样透出");
}

#[test]
fn start_with_empty_script_fails_instead_of_silently_succeeding() {
    let m = Arc::new(MockMemberExecutor::new());
    let err = start(&m, "cost-analyst", "cost-analyst-1").expect_err("必须失败");
    assert!(
        matches!(err, AdapterError::Conflict(_)),
        "脚本耗尽应是 Conflict：{err}"
    );
    assert!(
        err.detail().contains("脚本已耗尽"),
        "错误信息须点明是脚本耗尽而非真实 provider 故障：{err}"
    );
}

#[test]
fn steer_with_empty_script_fails_instead_of_silently_succeeding() {
    let m = Arc::new(MockMemberExecutor::new());
    let err = steer(&m, "cost-analyst-1", "插一句").expect_err("必须失败");
    assert!(
        matches!(err, AdapterError::Conflict(_)),
        "脚本耗尽应是 Conflict：{err}"
    );
    assert_eq!(m.count_of("steer"), 1, "失败的 steer 同样必须被记录");
}

#[test]
fn steer_hitting_a_start_success_step_reports_script_misalignment() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_ok(member("cost-analyst-1"), "产出");

    let err = steer(&m, "cost-analyst-1", "插一句").expect_err("必须失败");
    match err {
        AdapterError::Internal(detail) => {
            assert!(
                detail.contains("脚本错位"),
                "错误信息须点明脚本错位：{detail}"
            );
        }
        other => panic!("应为 Internal(脚本错位)，实际 {other:?}"),
    }
}

#[test]
fn exhausted_script_error_reports_how_many_calls_were_checked() {
    let m = Arc::new(MockMemberExecutor::new());
    let err = start(&m, "cost-analyst", "cost-analyst-1").expect_err("必须失败");
    assert!(
        err.detail().contains("1 次调用"),
        "错误须含已记录调用数（1 次调用）：{err}"
    );
}

#[test]
fn called_in_order_returns_false_when_one_side_never_happened() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_ok(member("cost-analyst-1"), "产出");
    start(&m, "cost-analyst", "cost-analyst-1").expect("start 应成功");

    assert!(
        !m.called_in_order("start", "steer"),
        "steer 从未发生，不得判 true"
    );
    assert!(
        !m.called_in_order("steer", "abort"),
        "两端都未发生，不得判 true"
    );
    assert!(
        !m.called_in_order("abort", "start"),
        "只有 start，不得判 true"
    );
}

#[test]
fn clear_calls_resets_ledger_but_keeps_script() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_ok(member("cost-analyst-1"), "产出")
        .push_ok(member("cost-analyst-2"), "产出二");
    start(&m, "cost-analyst", "cost-analyst-1").expect("应成功");
    assert_eq!(m.call_count(), 1);

    m.clear_calls();
    assert_eq!(
        m.call_count(),
        0,
        "清空后调用数应为 0（这是「确实为 0」而非未检查）"
    );
    assert!(
        !m.called_in_order("start", "steer"),
        "清空后无历史，不得声称存在时序关系"
    );

    let out = start(&m, "cost-analyst", "cost-analyst-2").expect("应成功");
    assert_eq!(out.output(), "产出二", "脚本不应被 clear_calls 影响");
    assert_eq!(m.call_count(), 1, "清空后重新计数为 1");
}

#[test]
fn outcome_step_carries_partial_status() {
    let m = Arc::new(MockMemberExecutor::new());
    let partial = MemberOutcome::new(
        member("cost-analyst-1"),
        MemberStatus::Partial,
        "完成成本结构分析，未取到实时账单",
        "部分结论：成本集中在存储",
    )
    .expect("测试用结果应合法");
    m.push(Step::Outcome(partial));

    let out = start(&m, "cost-analyst", "cost-analyst-1").expect("应成功");
    assert_eq!(out.status(), MemberStatus::Partial);
    assert!(
        out.status().is_deliverable(),
        "partial 仍可交付给主持人汇总"
    );
    assert_eq!(out.completed_scope(), "完成成本结构分析，未取到实时账单");
    assert_eq!(out.output(), "部分结论：成本集中在存储");
}

#[test]
fn outcome_step_carries_failed_status_with_scope_only() {
    let m = Arc::new(MockMemberExecutor::new());
    let failed =
        MemberOutcome::failed(member("cost-analyst-1"), "只完成了成本结构分析").expect("应合法");
    m.push(Step::Outcome(failed));

    let out = start(&m, "cost-analyst", "cost-analyst-1").expect("应成功");
    assert_eq!(out.status(), MemberStatus::Failed);
    assert_eq!(out.output(), "", "失败结果不得携带产出正文");
    assert!(!out.status().is_deliverable(), "failed 不可交付");
}

#[test]
fn chain_helper_builds_usable_hops_for_cycle_checks() {
    let hops = chain(&[("node-a", "t-001"), ("node-b", "t-002")]);
    assert_eq!(hops.len(), 2, "已检查：应构造 2 跳");
    assert_eq!(hops[0].node(), "node-a");

    let r = req("cost-analyst", "cost-analyst-1").with_chain(hops[0].clone());
    assert_eq!(r.chain().len(), 1, "链应挂到请求上");
    assert_eq!(
        check_chain(r.chain(), "node-a"),
        ChainCheck::Cycle {
            node: "node-a".into(),
            first_at: 0
        },
        "把已在链上的节点再派一次必须检出环"
    );
}

#[test]
fn mock_is_usable_from_multiple_threads() {
    let m = Arc::new(MockMemberExecutor::new());
    for i in 0..8 {
        m.push_ok(member(&format!("cost-analyst-{i}")), "产出");
    }
    let mut handles = Vec::new();
    for i in 0..8 {
        let m = Arc::clone(&m);
        handles.push(std::thread::spawn(move || {
            block_on(m.start(req("cost-analyst", &format!("cost-analyst-{i}"))))
        }));
    }
    let mut ok = 0;
    for h in handles {
        if h.join().expect("线程不应 panic").is_ok() {
            ok += 1;
        }
    }
    assert_eq!(ok, 8, "已检查 8 个线程，8 个 start 都应成功");
    assert_eq!(m.call_count(), 8, "已记录 8 次调用（无丢失、无重复）");
}
