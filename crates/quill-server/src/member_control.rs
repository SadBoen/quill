//! 运行中成员的控制面：**追加指令（steer）** 与 **中途取消（abort）**。
//!
//! 形**抄自 goose**：上游把「追加指令」放进一个 **FIFO 队列**，由状态机在
//! **轮与轮之间**取走、注入对话 —— `vendor/goose/crates/goose/src/agents/agent.rs:562-600`
//! （`Agent::steer` / `steer_queue` / `drain_pending_steers`）与
//! `vendor/goose/crates/goose/src/agents/state_machine/ops_steer.rs:1-70`
//! （`SteerOperation`，头注写着「Adds queued user guidance when the agent is
//! between model and tool turns」）。
//!
//! **键不一样（如实记）**：goose 按 **session** 排队；quill 一轮派工里所有成员共用
//! leader 的 session（`RoundRequest.session`），拿 session 当键会把同一轮的不同成员
//! 串在一起。所以这里按 **(发起人, 成员)** 排队 —— 成员标识在一轮里唯一，加上发起人
//! 是为了让两个用户各自的 `cost-analyst-1` 不可能互相串台。
//!
//! **能力的边界，不猜**：steer 只作用在**运行中**的成员上。没有在跑的成员就如实报错，
//! 不假装送达、也不排队等下一次 —— 「排队等下次」意味着一次调用可能永远不生效，
//! 而调用方以为自己成功送达了。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use quill_adapters::{AbortScope, AdapterError, MemberId, Message, UserId};

/// 一次成员运行的句柄。注册表里一份、执行器手里一份。
pub struct MemberRun {
    steers: Mutex<Vec<Message>>,
    cancelled: AtomicBool,
    wake: tokio::sync::Notify,
}

impl MemberRun {
    fn new() -> Self {
        Self {
            steers: Mutex::new(Vec::new()),
            cancelled: AtomicBool::new(false),
            wake: tokio::sync::Notify::new(),
        }
    }

    fn push_steer(&self, m: Message) {
        self.steers.lock().expect("steer 队列锁不应毒化").push(m);
    }

    /// 取走这一轮攒下的追加指令。成员在**两轮模型调用之间**调它。
    pub fn drain_steers(&self) -> Vec<Message> {
        std::mem::take(&mut *self.steers.lock().expect("steer 队列锁不应毒化"))
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// 等到被取消。已经取消了就立刻返回。
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.wake.notified().await;
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        // `notify_one` 而不是 `notify_waiters`：没人在等时它会**存一张通行证**，
        // 于是「先 abort、成员才轮到 select」这种顺序也不会漏掉取消。
        self.wake.notify_one();
    }
}

/// 注册表的守卫：成员跑完（正常结束、报错、panic）时把注册的那一条摘掉。
///
/// 摘的时候要 `Arc::ptr_eq` 认自己的那一份 —— 同键又注册了一次（同一成员被并发跑）
/// 时不许把新那份误摘了。
pub struct MemberRunGuard {
    control: Arc<MemberControl>,
    key: (UserId, MemberId),
    run: Arc<MemberRun>,
}

impl Drop for MemberRunGuard {
    fn drop(&mut self) {
        let mut g = self.control.runs.lock().expect("成员注册表锁不应毒化");
        if let Some(cur) = g.get(&self.key) {
            if Arc::ptr_eq(cur, &self.run) {
                g.remove(&self.key);
            }
        }
    }
}

/// 运行中成员的注册表。进程里一份，由 `AppState` 持有（每个测试实例一份，
/// 与 `login_limiter` 同一条理由：全局 `OnceLock` 会让用例互相污染）。
#[derive(Default)]
pub struct MemberControl {
    runs: Mutex<HashMap<(UserId, MemberId), Arc<MemberRun>>>,
}

impl MemberControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一次成员运行。返回（这次运行的句柄，守卫）—— 守卫活到成员跑完。
    pub fn register(
        self: &Arc<Self>,
        owner: UserId,
        member: MemberId,
    ) -> (Arc<MemberRun>, MemberRunGuard) {
        let run = Arc::new(MemberRun::new());
        let key = (owner, member);
        self.runs
            .lock()
            .expect("成员注册表锁不应毒化")
            .insert(key.clone(), Arc::clone(&run));
        let guard = MemberRunGuard {
            control: Arc::clone(self),
            key,
            run: Arc::clone(&run),
        };
        (run, guard)
    }

    /// 在跑的成员（这位发起人名下的），给诊断与用例用。
    pub fn running(&self, owner: &UserId) -> Vec<MemberId> {
        self.runs
            .lock()
            .expect("成员注册表锁不应毒化")
            .keys()
            .filter(|(o, _)| o == owner)
            .map(|(_, m)| m.clone())
            .collect()
    }

    /// 给**运行中**的成员追加一条指令。
    pub fn steer(&self, owner: &UserId, member: &MemberId, m: Message) -> Result<(), AdapterError> {
        let run = self.lookup(owner, member)?;
        if run.is_cancelled() {
            return Err(AdapterError::Conflict(format!(
                "成员 {member} 已经被取消，这条追加指令没有送到。\
                 下一步：这一轮结束后重派（同一派工键会幂等跳过已结算的成员）。"
            )));
        }
        run.push_steer(m);
        Ok(())
    }

    /// 中途取消。`StopRound` 停这一位成员；`AbortRoom`（上游语义 = 停一整间房）
    /// 落成「停这位发起人名下**所有**运行中的成员」—— 执行器一次只服务一个房间，
    /// 所以「这个人名下在跑的」就是这一轮的那些。
    pub fn abort(
        &self,
        owner: &UserId,
        member: &MemberId,
        scope: AbortScope,
    ) -> Result<(), AdapterError> {
        match scope {
            AbortScope::StopRound => {
                self.lookup(owner, member)?.cancel();
                Ok(())
            }
            AbortScope::AbortRoom => {
                let mine: Vec<Arc<MemberRun>> = {
                    let g = self.runs.lock().expect("成员注册表锁不应毒化");
                    g.iter()
                        .filter(|((o, _), _)| o == owner)
                        .map(|(_, r)| Arc::clone(r))
                        .collect()
                };
                if mine.is_empty() {
                    return Err(no_running(member));
                }
                for run in mine {
                    run.cancel();
                }
                Ok(())
            }
        }
    }

    fn lookup(&self, owner: &UserId, member: &MemberId) -> Result<Arc<MemberRun>, AdapterError> {
        self.runs
            .lock()
            .expect("成员注册表锁不应毒化")
            .get(&(*owner, member.clone()))
            .map(Arc::clone)
            .ok_or_else(|| no_running(member))
    }
}

/// 「没有在跑的成员」这句错误只有一份 —— 两个入口（steer / abort）与两种传输
/// 都得说同一句话，各写一遍迟早漂。
fn no_running(member: &MemberId) -> AdapterError {
    AdapterError::NotFound(format!(
        "没有正在运行的成员 {member}：steer / abort 只作用于**运行中**的成员，不排队等下一次。\
         下一步：确认这一轮真的在跑（`GET /api/dispatch/inflight` 能看到在途派工），再发一次。"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u8) -> UserId {
        UserId::from_bytes([n; 16])
    }

    fn m(name: &str) -> MemberId {
        MemberId::parse(name).expect("成员标识合法")
    }

    fn control() -> Arc<MemberControl> {
        Arc::new(MemberControl::new())
    }

    #[test]
    fn steer_and_abort_only_reach_a_running_member() {
        // 没有在跑的成员：两个入口都必须**如实报错**，不假装送达。
        let c = control();
        let msg = Message::user("补充一句").expect("消息合法");
        let err = c
            .steer(&u(1), &m("cost-analyst-1"), msg)
            .expect_err("没人跑就该报错");
        assert!(format!("{err}").contains("没有正在运行的成员"), "{err}");
        assert!(format!("{err}").contains("下一步"), "{err}");
        let err = c
            .abort(&u(1), &m("cost-analyst-1"), AbortScope::StopRound)
            .expect_err("没人跑就该报错");
        assert!(format!("{err}").contains("没有正在运行的成员"), "{err}");
    }

    #[tokio::test]
    async fn a_registered_run_receives_steers_and_the_guard_removes_it() {
        let c = control();
        let (run, guard) = c.register(u(1), m("cost-analyst-1"));
        assert_eq!(c.running(&u(1)), vec![m("cost-analyst-1")]);

        c.steer(&u(1), &m("cost-analyst-1"), Message::user("一").unwrap())
            .expect("运行中就能送");
        c.steer(&u(1), &m("cost-analyst-1"), Message::user("二").unwrap())
            .expect("运行中就能送");
        let drained: Vec<String> = run
            .drain_steers()
            .iter()
            .map(|x| x.text().to_string())
            .collect();
        assert_eq!(drained, vec!["一", "二"], "FIFO，且 drain 之后要清空");
        assert!(run.drain_steers().is_empty());

        // 别人的同名成员：碰不到。
        let err = c
            .steer(&u(2), &m("cost-analyst-1"), Message::user("蹭").unwrap())
            .expect_err("跨用户必须碰不到");
        assert!(format!("{err}").contains("没有正在运行的成员"), "{err}");

        drop(guard);
        assert!(c.running(&u(1)).is_empty(), "守卫 Drop 后注册表必须干净");
    }

    #[tokio::test]
    async fn abort_wakes_a_waiter_and_room_scope_hits_every_run_of_that_owner() {
        let c = control();
        let (a, _ga) = c.register(u(1), m("cost-analyst-1"));
        let (b, _gb) = c.register(u(1), m("growth-analyst-1"));
        let (_other, _go) = c.register(u(2), m("cost-analyst-1"));

        c.abort(&u(1), &m("cost-analyst-1"), AbortScope::StopRound)
            .expect("StopRound 应成功");
        a.cancelled().await; // 已经取消，立刻返回；没实现的话这里会挂住
        assert!(a.is_cancelled());
        assert!(!b.is_cancelled(), "StopRound 只停点名的那个");

        c.abort(&u(1), &m("growth-analyst-1"), AbortScope::AbortRoom)
            .expect("AbortRoom 应成功");
        assert!(b.is_cancelled(), "AbortRoom 停这位发起人名下所有成员");

        // 已经取消的成员：再送追加指令必须报错（送了也不会被读走）。
        let err = c
            .steer(&u(1), &m("cost-analyst-1"), Message::user("晚了").unwrap())
            .expect_err("取消之后不许假装送达");
        assert!(format!("{err}").contains("已经被取消"), "{err}");
    }

    #[tokio::test]
    async fn a_cancel_that_arrives_before_the_wait_still_wins() {
        // 先 abort、后 await：`notify_one` 存的那张通行证要让它立刻返回。
        let c = control();
        let (run, _guard) = c.register(u(1), m("cost-analyst-1"));
        c.abort(&u(1), &m("cost-analyst-1"), AbortScope::StopRound)
            .expect("应成功");
        run.cancelled().await;
        assert!(run.is_cancelled());
    }
}
