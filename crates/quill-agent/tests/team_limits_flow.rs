//! `teams` 四个限制列在**派工真路径**上的行为（队列 Q043）。
//!
//! 这些用例全部走 `Dispatcher::dispatch_round` —— 也就是
//! `POST /api/teams/{id}/dispatch/run` 真正调用的那一条路。钉三件事：
//!
//! 1. 超过 `max_dispatch` / `max_replan` 的一轮**整轮被拒**：一个成员都不起、
//!    台账一行都不写（不留半派状态）；
//! 2. 没超限的一轮照常执行；
//! 3. `guidelines` 逐字进每个成员的任务正文（执行器拿到的 `instructions`）。
//!
//! `max_ask_depth` **不在这里测**：它没有生效点 —— quill 的成员执行器没有
//! 「追问」通道（见 `quill_agent::team_limits` 的文件头）。给一个没人调用的
//! 闸门写测试，只会得到一条永远绿的假证据。

use std::sync::{Arc, Mutex};

use quill_adapters::{
    AbortScope, AdapterError, ChainHop, ExpertId, MemberExecutor, MemberId, MemberOutcome,
    MemberStartRequest, Message, SessionId, UserId,
};
use quill_agent::{
    DispatchTask, Dispatcher, MemDispatchLedger, RoundRequest, SharedExecutor, TeamLimits,
};
use quill_domain::team::{roster, team_of};
use quill_domain::Team;
use quill_testkit::mock_member::MockMemberExecutor;

const ROOM: &str = "limits-room";

fn u(n: u8) -> UserId {
    UserId::from_bytes([n; 16])
}

fn s(n: u8) -> SessionId {
    SessionId::from_bytes([n; 16])
}

fn e(name: &str) -> ExpertId {
    ExpertId::parse(name).expect("测试用专家名应合法")
}

/// 一个够用的 3 人团：leader-bot + cost-analyst + growth-analyst。
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
        format!("请分析 {expert} 的指标"),
    )
    .expect("测试用派工单应合法")
}

fn round_with<E: MemberExecutor>(
    d: &Dispatcher<SharedExecutor<E>, MemDispatchLedger>,
    team: &Team,
    round_no: u32,
    tasks: &[DispatchTask],
    limits: &TeamLimits,
) -> Result<quill_agent::DispatchReport, quill_agent::AgentError> {
    d.dispatch_round(&RoundRequest {
        owner: u(1),
        session: s(1),
        team,
        room_id: ROOM,
        round: round_no,
        tasks,
        chain: &[],
        limits,
    })
}

#[test]
fn a_round_over_max_dispatch_is_rejected_before_any_member_runs() {
    let m = Arc::new(MockMemberExecutor::new());
    let d = Dispatcher::new(
        SharedExecutor::new(Arc::clone(&m)),
        MemDispatchLedger::new(),
    );
    // 上限 2，这一轮要派 3 个 —— 先把第 3 个成员补进团里。
    let mut team = team3();
    team.add_member(e("risk-reviewer"), &roster(&["risk-reviewer"]))
        .expect("加成员应成功");
    let limits = TeamLimits::new(2, 2, 3, "").expect("合法限制");

    let tasks = [
        task("cost-analyst", 1),
        task("growth-analyst", 1),
        task("risk-reviewer", 1),
    ];
    let err = round_with(&d, &team, 0, &tasks, &limits).expect_err("3 > max_dispatch=2 必须判红");

    assert_eq!(err.code(), "team_limits_exceeded");
    let msg = err.to_string();
    assert!(msg.contains("max_dispatch"), "要点名限制：{msg}");
    assert!(
        msg.contains('3') && msg.contains('2'),
        "要带实际值与上限：{msg}"
    );
    assert!(msg.contains("下一步"), "必须给下一步：{msg}");
    assert_eq!(m.count_of("start"), 0, "🔴 整轮被拒时一个成员都不许起");
    assert_eq!(d.ledger().len(), 0, "🔴 被拒的一轮不许在台账留下记录");
}

#[test]
fn a_round_at_max_dispatch_really_runs() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_ok(
        MemberId::parse("cost-analyst-1").expect("应合法"),
        "成本结论",
    );
    m.push_ok(
        MemberId::parse("growth-analyst-1").expect("应合法"),
        "增长结论",
    );
    let d = Dispatcher::new(
        SharedExecutor::new(Arc::clone(&m)),
        MemDispatchLedger::new(),
    );
    let limits = TeamLimits::new(2, 2, 3, "").expect("合法限制");

    let tasks = [task("cost-analyst", 1), task("growth-analyst", 1)];
    let report = round_with(&d, &team3(), 0, &tasks, &limits).expect("正好到上限必须放行");
    assert_eq!(report.delivered_count(), 2);
    assert_eq!(m.count_of("start"), 2, "到上限的这一轮要真的把成员跑起来");
}

#[test]
fn a_round_beyond_max_replan_is_rejected_while_the_first_round_passes() {
    let m = Arc::new(MockMemberExecutor::new());
    m.push_ok(
        MemberId::parse("cost-analyst-1").expect("应合法"),
        "首派结论",
    );
    let d = Dispatcher::new(
        SharedExecutor::new(Arc::clone(&m)),
        MemDispatchLedger::new(),
    );
    // 不允许重规划：只有 round 0 能派。
    let limits = TeamLimits::new(4, 0, 3, "").expect("合法限制");
    let tasks = [task("cost-analyst", 1)];

    let first = round_with(&d, &team3(), 0, &tasks, &limits).expect("round 0 是首派，必须放行");
    assert_eq!(first.delivered_count(), 1);

    let err = round_with(&d, &team3(), 1, &tasks, &limits)
        .expect_err("round 1 是重规划，max_replan=0 必须判红");
    assert_eq!(err.code(), "team_limits_exceeded");
    assert!(err.to_string().contains("max_replan"), "{err}");
    assert_eq!(m.count_of("start"), 1, "🔴 被拒的第 1 轮不许再跑成员");
    assert_eq!(
        d.ledger().len(),
        1,
        "🔴 被拒的一轮不许在台账里多出记录（只该有首派那一条）"
    );
}

/// 记录每次 `start` 拿到的 `(title, instructions)` 的执行器。
///
/// `MockMemberExecutor` 的 `CallRecord` 只记专家名，看不到提示正文，
/// 所以这里自己收一份 —— 要证明的正是「准则真的到了提示里」。
#[derive(Debug, Default)]
struct PromptRecorder {
    seen: Mutex<Vec<(String, String)>>,
}

impl PromptRecorder {
    fn seen(&self) -> Vec<(String, String)> {
        self.seen.lock().expect("测试锁不该毒化").clone()
    }
}

impl MemberExecutor for PromptRecorder {
    fn start(
        &self,
        req: MemberStartRequest,
    ) -> impl std::future::Future<Output = Result<MemberOutcome, AdapterError>> + Send {
        let member = req.member().clone();
        self.seen
            .lock()
            .expect("测试锁不该毒化")
            .push((req.title().to_string(), req.instructions().to_string()));
        async move {
            MemberOutcome::done(member, "已完成", "产出")
                .map_err(|err| AdapterError::Internal(format!("构造成员产出失败：{err}")))
        }
    }

    // 这里保留 `-> impl Future` 的写法以示「不碰 trait 的形状」；
    // 换成 `async fn` 只是语法糖，与本用例要测的东西无关。
    #[allow(clippy::manual_async_fn)]
    fn steer(
        &self,
        _owner: &UserId,
        _member: &MemberId,
        _m: Message,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        async move { Ok(()) }
    }

    #[allow(clippy::manual_async_fn)]
    fn abort(
        &self,
        _owner: &UserId,
        _member: &MemberId,
        _scope: AbortScope,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send {
        async move { Ok(()) }
    }
}

#[test]
fn guidelines_reach_every_member_prompt_before_the_task_text() {
    let recorder = Arc::new(PromptRecorder::default());
    let d = Dispatcher::new(
        SharedExecutor::new(Arc::clone(&recorder)),
        MemDispatchLedger::new(),
    );
    let limits =
        TeamLimits::new(4, 2, 3, "结论必须给出数据来源；不确定的地方要标注假设").expect("合法限制");

    let tasks = [task("cost-analyst", 1), task("growth-analyst", 1)];
    let report = round_with(&d, &team3(), 0, &tasks, &limits).expect("应派成功");
    assert_eq!(report.delivered_count(), 2);

    let seen = recorder.seen();
    assert_eq!(seen.len(), 2, "两个成员各拿一份提示");
    for (title, instructions) in &seen {
        let g_at = instructions
            .find("结论必须给出数据来源")
            .unwrap_or_else(|| panic!("团队准则没进提示（{title}）：{instructions}"));
        let t_at = instructions
            .find("请分析")
            .unwrap_or_else(|| panic!("任务正文不见了（{title}）：{instructions}"));
        assert!(g_at < t_at, "准则必须排在任务正文之前：{instructions}");
    }
    // 两个成员各自的标题不同（证明不是把同一条记录复制了两遍）。
    assert_ne!(seen[0].0, seen[1].0);
}

#[test]
fn without_guidelines_the_member_prompt_is_the_task_text_unchanged() {
    let recorder = Arc::new(PromptRecorder::default());
    let d = Dispatcher::new(
        SharedExecutor::new(Arc::clone(&recorder)),
        MemDispatchLedger::new(),
    );
    let limits = TeamLimits::default();
    assert!(!limits.has_guidelines());

    round_with(&d, &team3(), 0, &[task("cost-analyst", 1)], &limits).expect("应派成功");
    let seen = recorder.seen();
    assert_eq!(
        seen[0].1, "请分析 cost-analyst 的指标",
        "没有准则时必须逐字原样，不许拼空标题"
    );
}

#[test]
fn an_over_long_guidelines_is_rejected_at_construction_so_it_never_reaches_a_prompt() {
    // 这是「准则进提示」的另一半：唯一能进提示的入口就是 TeamLimits，
    // 所以长度闸门设在构造处 —— 提示不可能被一段超长准则撑爆。
    let too_long = "字".repeat(quill_agent::team_limits::MAX_GUIDELINES_CHARS + 1);
    let err = TeamLimits::new(4, 2, 3, too_long).expect_err("超长准则不许构造出来");
    assert!(err.to_string().contains("团队准则太长"), "{err}");
    assert!(
        err.next_step().contains("下一步") || err.next_step().contains("压到"),
        "{err}"
    );
}

#[test]
fn the_chain_gate_still_runs_after_the_team_limit_gate() {
    // 顺序钉住：限制闸门在前、委派链校验在后 —— 两者都不许被对方架空。
    let m = Arc::new(MockMemberExecutor::new());
    let d = Dispatcher::new(
        SharedExecutor::new(Arc::clone(&m)),
        MemDispatchLedger::new(),
    );
    let limits = TeamLimits::new(2, 2, 3, "").expect("合法限制");
    let hops: Vec<ChainHop> = (0..4)
        .map(|i| ChainHop::new(format!("node-{i}"), format!("t-{i}")).expect("应合法"))
        .collect();

    let err = d
        .dispatch_round(&RoundRequest {
            owner: u(1),
            session: s(1),
            team: &team3(),
            room_id: ROOM,
            round: 0,
            tasks: &[task("cost-analyst", 1)],
            chain: &hops,
            limits: &limits,
        })
        .expect_err("超深委派链仍须判红");
    assert_eq!(err.code(), "chain_too_deep");
}
