mod common;
mod dispatch_seed;

use std::collections::BTreeMap;
use std::sync::Arc;

use quill_adapters::{ExpertId, MemberId, MemberOutcome, MemberStatus, SessionId, UserId};
use quill_agent::{
    AgentError, BeginOutcome, DispatchKey, DispatchLedger, DispatchRecord, DispatchState,
    DispatchTask, MemberRejectKind, RoundPrefix,
};
use quill_server::db::DbBridge;
use quill_server::dispatch_ledger::{DispatchScope, SqlxDispatchLedger};

use common::{scalar_i64, text_of, TestDb};
use dispatch_seed::{member_session, seed};

fn u(n: u8) -> UserId {
    UserId::from_bytes([n; 16])
}

fn e(name: &str) -> ExpertId {
    ExpertId::parse(name).unwrap_or_else(|err| panic!("专家名 {name:?} 非法：{err}"))
}

fn member(expert: &str, seq: u32) -> MemberId {
    MemberId::for_expert(&e(expert), seq).unwrap_or_else(|err| panic!("成员标识非法：{err}"))
}

fn key(owner: UserId, room: &str, round: u32, expert: &str) -> DispatchKey {
    DispatchKey::new(owner, room, round, e(expert)).expect("派工键应合法")
}

fn fixture(
    label: &str,
    owner: u8,
    team_seed: u8,
    members: &[&str],
) -> (TestDb, SqlxDispatchLedger, String) {
    let t = TestDb::new(label);
    let f = seed(&t.bridge(), u(owner), team_seed, members);
    let mut map: BTreeMap<ExpertId, SessionId> = BTreeMap::new();
    for m in members {
        map.insert(e(m), member_session(team_seed, &e(m)));
    }
    let ledger = SqlxDispatchLedger::new(
        t.bridge(),
        DispatchScope::new(f.team_id, f.leader_session, map),
    );
    let room = f.room_id;
    (t, ledger, room)
}

#[test]
fn begin_twice_returns_created_then_existed() {
    let (t, ledger, room) = fixture("dispatch-idempotent", 1, 0x11, &["cost-analyst"]);
    let k = key(u(1), &room, 0, "cost-analyst");

    let first = ledger
        .begin(&DispatchRecord::pending(
            k.clone(),
            member("cost-analyst", 1),
        ))
        .expect("首次记账应成功");
    assert!(first.is_created(), "首次记账必须是 Created（键此前不存在）");
    assert_eq!(first.record().state(), DispatchState::Pending);

    let second = ledger
        .begin(&DispatchRecord::pending(
            k.clone(),
            member("cost-analyst", 1),
        ))
        .expect("二次记账必须成功返回（幂等的语义是「拿到同一个结果」，不是报错）");
    assert!(
        !second.is_created(),
        "🔴 二次记账必须判为 Existed —— 若判成 Created，说明判别逻辑是假的"
    );
    assert_eq!(
        second.record(),
        first.record(),
        "Existed 必须返回既有记录本身"
    );

    assert_eq!(
        scalar_i64(
            &t.bridge(),
            "SELECT COUNT(*) AS c FROM task_dispatches WHERE user_id = x'01010101010101010101010101010101'"
        ),
        1,
        "同一幂等键只能有一行"
    );
}

#[test]
fn begin_does_not_overwrite_an_existing_record() {
    let (_t, ledger, room) = fixture("dispatch-no-overwrite", 1, 0x12, &["cost-analyst"]);
    let k = key(u(1), &room, 0, "cost-analyst");

    ledger
        .begin(&DispatchRecord::pending(
            k.clone(),
            member("cost-analyst", 1),
        ))
        .expect("首次记账应成功");
    let mut running = DispatchRecord::pending(k.clone(), member("cost-analyst", 1));
    running.mark_running().expect("跃迁 RUNNING 应成功");
    ledger.put(&running).expect("写 RUNNING 应成功");

    let again = ledger
        .begin(&DispatchRecord::pending(
            k.clone(),
            member("cost-analyst", 1),
        ))
        .expect("二次记账应成功");
    assert!(!again.is_created());
    assert_eq!(
        again.record().state(),
        DispatchState::Running,
        "🔴 begin 不得把 RUNNING 覆盖回 PENDING"
    );
}

#[test]
fn same_key_of_another_user_is_independent() {
    let t = TestDb::new("dispatch-isolation");

    seed(&t.bridge(), u(1), 0x13, &["cost-analyst"]);
    seed(&t.bridge(), u(2), 0x14, &["cost-analyst"]);

    let mk = |team: u8| {
        let mut map = BTreeMap::new();
        map.insert(e("cost-analyst"), member_session(team, &e("cost-analyst")));
        SqlxDispatchLedger::new(
            t.bridge(),
            DispatchScope::new([team; 16], SessionId::from_bytes([team; 16]), map),
        )
    };
    let la = mk(0x13);
    let lb = mk(0x14);

    let ka = key(u(1), "shared-room", 3, "cost-analyst");
    let kb = key(u(2), "shared-room", 3, "cost-analyst");

    assert!(la
        .begin(&DispatchRecord::pending(
            ka.clone(),
            member("cost-analyst", 1)
        ))
        .expect("A 记账应成功")
        .is_created());
    let b_out = lb
        .begin(&DispatchRecord::pending(
            kb.clone(),
            member("cost-analyst", 1),
        ))
        .expect("B 记账应成功");
    assert!(
        b_out.is_created(),
        "🔴 B 的同名同房间同轮必须是 Created —— 命中 A 的记录即隔离失效"
    );
    assert_eq!(b_out.record().key().owner(), u(2));

    assert_eq!(la.inflight(&u(1)).expect("A 列在途应成功").len(), 1);
    assert_eq!(lb.inflight(&u(2)).expect("B 列在途应成功").len(), 1);

    let via_a = la
        .get(&kb)
        .expect("按 B 的键查应成功")
        .expect("B 的行必须存在");
    assert_eq!(via_a.key().owner(), u(2), "查到的必须是 B 自己的行");

    let a_sess = text_of(
        &t.bridge(),
        "SELECT hex(member_session_id) AS v FROM task_dispatches WHERE user_id = x'01010101010101010101010101010101'",
    );
    let b_sess = text_of(
        &t.bridge(),
        "SELECT hex(member_session_id) AS v FROM task_dispatches WHERE user_id = x'02020202020202020202020202020202'",
    );
    assert_ne!(a_sess, b_sess, "🔴 两用户的成员会话不得互相覆盖");
    assert!(
        a_sess.to_uppercase().starts_with("13"),
        "A 的行必须指向 A 的成员会话（以 13 开头）：{a_sess}"
    );
    assert!(
        b_sess.to_uppercase().starts_with("14"),
        "B 的行必须指向 B 的成员会话（以 14 开头）：{b_sess}"
    );
    assert_eq!(
        scalar_i64(&t.bridge(), "SELECT COUNT(*) AS c FROM task_dispatches"),
        2,
        "两个用户各一行"
    );
}

#[test]
fn state_transitions_and_settlement_survive_a_real_roundtrip() {
    let (t, ledger, room) = fixture(
        "dispatch-transitions",
        1,
        0x15,
        &["cost-analyst", "risk-checker"],
    );
    let k = key(u(1), &room, 2, "cost-analyst");

    ledger
        .begin(&DispatchRecord::pending(
            k.clone(),
            member("cost-analyst", 1),
        ))
        .expect("记账应成功");

    let mut r = DispatchRecord::pending(k.clone(), member("cost-analyst", 1));
    r.mark_running().expect("RUNNING 应成功");
    ledger.put(&r).expect("写 RUNNING 应成功");
    let got = ledger.get(&k).expect("读应成功").expect("行应存在");
    assert_eq!(got.state(), DispatchState::Running);
    assert_eq!(
        got.member().as_str(),
        "cost-analyst-1",
        "成员名必须往返保住"
    );

    let mut asking = DispatchRecord::pending(k.clone(), member("cost-analyst", 1));
    asking.mark_running().expect("RUNNING 应成功");
    asking.mark_asking(2).expect("ASKING 应成功");
    ledger.put(&asking).expect("写 ASKING 应成功");
    let got = ledger.get(&k).expect("读应成功").expect("行应存在");
    assert_eq!(got.state(), DispatchState::Asking);
    assert_eq!(
        got.ask_depth(),
        2,
        "ask_depth 必须往返保住（表上有对应 CHECK）"
    );

    let outcome = MemberOutcome::new(
        member("cost-analyst", 1),
        MemberStatus::Partial,
        "已完成数据采集",
        "采集到 3 条成本记录",
    )
    .expect("结果应合法");
    let mut done = DispatchRecord::pending(k.clone(), member("cost-analyst", 1));
    done.settle_done(outcome.clone()).expect("结算 DONE 应成功");
    ledger.put(&done).expect("写 DONE 应成功");
    let got = ledger.get(&k).expect("读应成功").expect("行应存在");
    assert_eq!(got.state(), DispatchState::Done);
    let o = got.outcome().expect("DONE 必须带回结果");
    assert_eq!(o.status(), MemberStatus::Partial);
    assert_eq!(o.completed_scope(), "已完成数据采集");
    assert_eq!(o.output(), "采集到 3 条成本记录");
    assert_eq!(*o, outcome, "结算结果必须逐字往返（不丢正文）");
    assert_eq!(
        text_of(
            &t.bridge(),
            "SELECT state AS v FROM task_dispatches WHERE member_expert_id = 'cost-analyst'"
        ),
        "DONE",
        "🔴 状态必须真的落库，而不是只在内存里转"
    );

    let k2 = key(u(1), &room, 2, "risk-checker");
    let mut failed = DispatchRecord::pending(k2.clone(), member("risk-checker", 1));
    let err = AgentError::MemberRejected {
        member: member("risk-checker", 1),
        kind: MemberRejectKind::SelfReportedFailure,
        detail: "成员自报做不到风险评估".to_string(),
        retryable: true,
    };
    failed.settle_failed(err).expect("结算 FAILED 应成功");
    ledger.put(&failed).expect("写 FAILED 应成功");
    let got = ledger.get(&k2).expect("读应成功").expect("行应存在");
    assert_eq!(got.state(), DispatchState::Failed);
    let e2 = got.error().expect("FAILED 必须带回错误");
    assert_eq!(e2.code(), "member_reported_failure", "🔴 错误码必须往返");
    assert!(e2.is_retryable(), "retryable 标志必须往返");
    assert!(e2.to_string().contains("做不到风险评估"));
}

#[test]
fn inflight_lists_only_inflight_and_list_round_is_sorted() {
    let (_t, ledger, room) = fixture("dispatch-listing", 1, 0x16, &["zeta", "alpha", "mid"]);
    for (expert, round) in [("zeta", 0u32), ("alpha", 0), ("mid", 0), ("alpha", 1)] {
        let k = key(u(1), &room, round, expert);
        ledger
            .begin(&DispatchRecord::pending(k, member(expert, 1)))
            .unwrap_or_else(|err| panic!("{expert}/round-{round} 记账失败：{err}"));
    }

    let k = key(u(1), &room, 0, "alpha");
    let mut done = DispatchRecord::pending(k.clone(), member("alpha", 1));
    done.settle_done(
        MemberOutcome::new(member("alpha", 1), MemberStatus::Done, "已完成", "结论")
            .expect("结果应合法"),
    )
    .expect("结算应成功");
    ledger.put(&done).expect("写 DONE 应成功");

    let inflight = ledger.inflight(&u(1)).expect("列在途应成功");
    assert_eq!(inflight.len(), 3, "已结算的不该在途：{inflight:?}");
    assert!(inflight.iter().all(|r| r.state().is_inflight()));

    let round0 = ledger
        .list_round(&RoundPrefix {
            owner: u(1),
            room_id: room.clone(),
            round: 0,
        })
        .expect("列某轮应成功");
    let ids: Vec<String> = round0
        .iter()
        .map(|r| r.key().member_expert().as_str().to_string())
        .collect();
    assert_eq!(
        ids,
        vec!["alpha", "mid", "zeta"],
        "必须按 member_expert 升序"
    );
    assert!(
        round0.iter().any(|r| r.state() == DispatchState::Done),
        "已结算的记录也属于「某轮派工」"
    );
}

#[test]
fn missing_member_session_is_reported_and_writes_nothing() {
    let t = TestDb::new("dispatch-missing-session");
    seed(&t.bridge(), u(1), 0x17, &["cost-analyst"]);

    let ledger = SqlxDispatchLedger::new(
        t.bridge(),
        DispatchScope::new(
            [0x17; 16],
            SessionId::from_bytes([0x17; 16]),
            BTreeMap::new(),
        ),
    );
    let k = key(u(1), "room-21", 0, "cost-analyst");
    let err = ledger
        .begin(&DispatchRecord::pending(k, member("cost-analyst", 1)))
        .expect_err("缺成员会话必须判红");
    assert_eq!(
        err.code(),
        "dispatch_request_invalid",
        "应是「请求前提不合法」而不是「存储错误」：{err}"
    );
    assert!(
        err.to_string().contains("cost-analyst"),
        "错误必须点名缺的是哪个专家：{err}"
    );
    assert_eq!(
        scalar_i64(&t.bridge(), "SELECT COUNT(*) AS c FROM task_dispatches"),
        0,
        "🔴 失败时绝不能留下半行"
    );
}

#[test]
fn dispatcher_agrees_with_the_ledger_on_replay() {
    use std::sync::Mutex;

    #[derive(Debug, Clone, Default)]
    struct CountingExecutor {
        started: Arc<Mutex<Vec<MemberId>>>,
    }
    impl quill_adapters::MemberExecutor for CountingExecutor {
        fn start(
            &self,
            req: quill_adapters::MemberStartRequest,
        ) -> impl std::future::Future<Output = Result<MemberOutcome, quill_adapters::AdapterError>> + Send
        {
            let m = req.member().clone();
            let log = Arc::clone(&self.started);
            async move {
                log.lock().expect("计数锁不应被毒化").push(m);
                MemberOutcome::done(req.member().clone(), "已完成", "模拟执行产出")
                    .map_err(|e| quill_adapters::AdapterError::Internal(e.to_string()))
            }
        }
        async fn steer(
            &self,
            _member: &MemberId,
            _msg: quill_adapters::Message,
        ) -> Result<(), quill_adapters::AdapterError> {
            Ok(())
        }
        async fn abort(
            &self,
            _member: &MemberId,
            _scope: quill_adapters::AbortScope,
        ) -> Result<(), quill_adapters::AdapterError> {
            Ok(())
        }
    }

    let (_t, ledger, room) = fixture("dispatch-replay", 1, 0x18, &["cost-analyst"]);
    let started = Arc::new(Mutex::new(Vec::new()));
    let exec = CountingExecutor {
        started: Arc::clone(&started),
    };
    let d = quill_agent::Dispatcher::new(exec, ledger);

    let task = DispatchTask::with_seq(e("cost-analyst"), 1, "算成本", "把本月成本算清楚")
        .expect("派工单应合法");

    let team = quill_domain::Team::new(
        quill_domain::TeamId::parse("team-24").expect("团队名应合法"),
        "成本核算团",
        e("cost-analyst"),
    )
    .expect("团队应可建");
    // 默认限制（max_dispatch=4 / max_replan=2）：这条用例只关心幂等重放。
    let limits = quill_agent::TeamLimits::default();
    let req = quill_agent::RoundRequest {
        owner: u(1),
        session: SessionId::from_bytes([0x18; 16]),
        team: &team,
        room_id: &room,
        round: 0,
        tasks: std::slice::from_ref(&task),
        chain: &[],
        limits: &limits,
    };

    let r1 = d.dispatch_round(&req).expect("首轮派工应成功");
    assert_eq!(r1.delivered_count(), 1, "首轮应交付 1 个成员");
    assert_eq!(started.lock().expect("锁").len(), 1);

    let r2 = d.dispatch_round(&req).expect("重放同轮应成功（幂等）");
    assert_eq!(
        r2.delivered_count(),
        0,
        "🔴 重放不得再交付（已结算的键必须被跳过）"
    );
    assert_eq!(
        r2.skipped_as_duplicate.len(),
        1,
        "重放必须记为幂等跳过：{}",
        r2.summary()
    );
    assert_eq!(
        started.lock().expect("锁").len(),
        1,
        "🔴 重放不得再次调用执行器（否则就是重复副作用）"
    );
}

#[test]
fn a_fresh_bridge_sees_the_dispatches_written_by_the_previous_one() {
    let t = TestDb::new("dispatch-restart");
    let path = t.path();
    {
        let f = seed(&t.bridge(), u(1), 0x19, &["cost-analyst"]);
        let mut map = BTreeMap::new();
        map.insert(e("cost-analyst"), member_session(0x19, &e("cost-analyst")));
        let ledger = SqlxDispatchLedger::new(
            t.bridge(),
            DispatchScope::new(f.team_id, f.leader_session, map),
        );
        let k = key(u(1), "room-23", 0, "cost-analyst");
        ledger
            .begin(&DispatchRecord::pending(k, member("cost-analyst", 1)))
            .expect("记账应成功");
    }
    let fresh = Arc::new(DbBridge::open(&path, 2).expect("重开库应成功"));

    let ledger = SqlxDispatchLedger::new(
        fresh,
        DispatchScope::new([0u8; 16], SessionId::from_bytes([0u8; 16]), BTreeMap::new()),
    );
    let got = ledger
        .get(&key(u(1), "room-23", 0, "cost-analyst"))
        .expect("读应成功")
        .expect("🔴 重启后派工记录必须还在（否则崩溃恢复无从谈起）");
    assert_eq!(got.state(), DispatchState::Pending);
    assert_eq!(got.member().as_str(), "cost-analyst-1");
}

/// 同一个派工键**并发** begin：只能有一个 `Created`，其余全是 `Existed`，且一个都不许报错。
///
/// 为什么要单独测（queue Q082）：`begin` 走的是
/// `INSERT … ON CONFLICT DO NOTHING RETURNING …` —— 撞上冲突时**返回零行**，
/// 实现必须把那零行解释成「别人先建了」，而不是「插入失败」。
/// 顺序调用已经被 `begin_twice_returns_created_then_existed` 覆盖，但那是**同一个线程
/// 一前一后**；并发下 N 次调用挤进同一条存储队列、顺序不定，只有真并发才看得见
/// 「零行分支」被同时走到会不会出问题（以及有没有哪一次读到半截记录）。
#[test]
fn concurrent_begins_for_the_same_key_produce_exactly_one_created() {
    let (_t, ledger, room) = fixture("dispatch-concurrent-begin", 1, 0x22, &["cost-analyst"]);
    let ledger = Arc::new(ledger);
    let record = DispatchRecord::pending(
        key(u(1), &room, 0, "cost-analyst"),
        member("cost-analyst", 1),
    );

    const N: usize = 8;
    let gate = Arc::new(std::sync::Barrier::new(N));
    let handles: Vec<_> = (0..N)
        .map(|_| {
            let ledger = Arc::clone(&ledger);
            let gate = Arc::clone(&gate);
            let record = record.clone();
            std::thread::spawn(move || {
                // 一起冲：尽量让 N 次 begin 同时落在存储队列里。
                gate.wait();
                ledger.begin(&record).expect("并发 begin 不许报错")
            })
        })
        .collect();
    let outcomes: Vec<BeginOutcome> = handles
        .into_iter()
        .map(|h| h.join().expect("线程不许 panic"))
        .collect();

    let created = outcomes.iter().filter(|o| o.is_created()).count();
    assert_eq!(
        created, 1,
        "同一个键只能被创建一次，实际 Created={created}；全部结果：{outcomes:?}"
    );
    // 没有哪一次拿到一条被写坏或写岔的记录：N 个结果必须逐字段相同。
    let first = outcomes[0].record().clone();
    assert!(
        outcomes.iter().all(|o| o.record() == &first),
        "并发 begin 拿到的记录不一致（有人读到了半截或别的行）：{outcomes:?}"
    );
    assert_eq!(first.state(), DispatchState::Pending);
    assert_eq!(first.key(), &key(u(1), &room, 0, "cost-analyst"));
}

/// 四个**不同**的键并发 begin：必须各建一次 —— 一个都不许被「冲突」吞掉。
///
/// 这是上一条的补集，抓的是相反方向的错：`ON CONFLICT DO NOTHING` 一旦**管得太宽**
/// （比如派生行 id 少算了一个成员，四个键撞成同一行），表现就是「静默少记账」——
/// 派工发出去了、台账里却没有，而顺序用例一条都发现不了（顺序下每次只有一个键）。
#[test]
fn concurrent_begins_for_distinct_keys_never_lose_a_dispatch() {
    const MEMBERS: [&str; 4] = ["cost-analyst", "researcher", "writer", "reviewer"];
    let (_t, ledger, room) = fixture("dispatch-concurrent-distinct", 1, 0x23, &MEMBERS);
    let ledger = Arc::new(ledger);

    let gate = Arc::new(std::sync::Barrier::new(MEMBERS.len()));
    let handles: Vec<_> = MEMBERS
        .iter()
        .map(|name| {
            let name: &str = name;
            let ledger = Arc::clone(&ledger);
            let gate = Arc::clone(&gate);
            let record = DispatchRecord::pending(key(u(1), &room, 0, name), member(name, 1));
            std::thread::spawn(move || {
                gate.wait();
                ledger.begin(&record).expect("并发 begin 不许报错")
            })
        })
        .collect();
    let outcomes: Vec<BeginOutcome> = handles
        .into_iter()
        .map(|h| h.join().expect("线程不许 panic"))
        .collect();

    let created = outcomes.iter().filter(|o| o.is_created()).count();
    assert_eq!(
        created,
        MEMBERS.len(),
        "四个不同的键必须各建一次；被当成冲突吞掉就等于丢了派工：{outcomes:?}"
    );
    assert_eq!(
        ledger.inflight(&u(1)).expect("列在途").len(),
        MEMBERS.len(),
        "在途清单必须四个都在（少一个就是静默丢账）"
    );
}
