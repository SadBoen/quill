//! 时间来源：注入式 [`Clock`]，让「过期」这类逻辑**可测**。
//!
//! # 为什么不用 `SystemTime::now()` 直接写死
//!
//! 会话过期、邀请码过期、登录锁定都是**时间相关**的不变量。若代码里直接调
//! `SystemTime::now()`，测试就只能靠 `sleep` —— 而 `sleep` 造出来的测试有两个致命问题：
//! ① 慢（CI 上每次几十秒）；② ** flaky **（机器一慢，断言边界就飘）。
//! 于是「过期」这条最关键的安全逻辑，反而是最少被测到的。
//!
//! 注入 `Clock` 后，测试用 [`ManualClock`] 精确推进，不 sleep、不 flaky。
//!
//! # 边界规则 7 面 B：禁 static 承载用户态
//!
//! 本 crate **没有** `static NOW` / `OnceLock<Clock>`。时钟随
//! [`ControlPlane`](crate::ControlPlane) 实例走 —— 不同实例可以带不同时钟，
//! 也就没有「一个全局时钟被测试偷偷改掉」的可能。

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 时间来源（Unix **毫秒**）。
///
/// 选毫秒而不是 `SystemTime`：SQLite 里所有时间列都是 `INTEGER` 毫秒
/// （见 `crates/quill-store/migrations/0001_init.sql`），
/// 用同一单位才不会出现「秒当毫秒存」这类差 1000 倍的静默错误。
pub trait Clock: Send + Sync + 'static {
    /// 当前时刻（Unix 毫秒）。
    fn now_millis(&self) -> i64;
}

/// 生产用时钟：读系统时间。
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
            // ⚠️ 时钟早于 1970：这不是「没时间」，而是系统时间被设错了。
            // 铁律七要求「环境变量配错不得 panic，服务仍能起」，
            // 所以这里钳到下界而不是 panic —— 后果是所有会话立刻判过期，
            // 是**响亮**的失败（用户一登录就看到过期），而不是静默算错。
            Err(_) => i64::MIN,
        }
    }
}

/// 手动推进的时钟（**仅供测试**）。
///
/// 存在的理由见模块文档。它是公开的，因为集成测试（`tests/`）要用它 ——
/// 放在 `#[cfg(test)]` 里则集成测试根本够不着。
#[derive(Debug)]
pub struct ManualClock {
    now_ms: AtomicI64,
}

impl ManualClock {
    /// 以给定时刻构造。
    pub fn new(start_ms: i64) -> Self {
        Self {
            now_ms: AtomicI64::new(start_ms),
        }
    }

    /// 推进 `delta_ms` 毫秒，返回推进后的时刻。
    pub fn advance(&self, delta_ms: i64) -> i64 {
        self.now_ms.fetch_add(delta_ms, Ordering::SeqCst) + delta_ms
    }

    /// 直接跳到 `target_ms`（只允许前进，避免测试意外造出「时间倒流」的不可能状态）。
    ///
    /// # Panics
    ///
    /// `target_ms` 早于当前时刻时 panic —— 那是**测试写错了**，
    /// 而不是一个应当被容忍的运行时情形。
    pub fn set(&self, target_ms: i64) {
        let prev = self.now_ms.swap(target_ms, Ordering::SeqCst);
        assert!(
            target_ms >= prev,
            "ManualClock 只能前进：试图从 {prev} 回到 {target_ms}"
        );
    }
}

impl Clock for ManualClock {
    fn now_millis(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

impl Default for ManualClock {
    /// 默认起点：2024-01-01T00:00:00Z。
    ///
    /// 取一个**固定**值而不是 0：断言里的时间戳因此可读，
    /// 且不会因为「从 1970 起算」让人误以为在测边界。
    fn default() -> Self {
        Self::new(1_704_067_200_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_advances_deterministically() {
        let c = ManualClock::new(1_000);
        assert_eq!(c.now_millis(), 1_000);
        assert_eq!(c.advance(500), 1_500);
        assert_eq!(c.now_millis(), 1_500);
        // 再推进 0 也不变 —— 证明 advance 是加法而不是「设为 delta」
        assert_eq!(c.advance(0), 1_500);
        assert_eq!(c.advance(-200), 1_300);
    }

    #[test]
    fn manual_clock_is_shared_across_threads() {
        // Clock 要求 Send+Sync：控制面会被 axum 的多线程运行时并发调用。
        // 这里用真线程验证，而不是靠「它编译过了」推断。
        let c = std::sync::Arc::new(ManualClock::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let c = std::sync::Arc::clone(&c);
            handles.push(std::thread::spawn(move || {
                c.advance(10);
            }));
        }
        for h in handles {
            h.join().expect("线程不应 panic");
        }
        assert_eq!(c.now_millis(), 40, "4 个线程各推进 10ms，应累加为 40ms");
    }

    #[test]
    #[should_panic(expected = "ManualClock 只能前进")]
    fn manual_clock_refuses_to_go_backwards() {
        // 装置可信性：「时间倒流」必须是响亮的失败。
        // 若这条测试因为 assert 被删而消失，set() 的语义就悄悄变了。
        let c = ManualClock::new(1_000);
        c.set(500);
    }

    #[test]
    fn system_clock_is_after_2020_and_matches_reasonable_magnitude() {
        // 不用固定值断言（会 flaky），只断言**量级与单调性**。
        let c = SystemClock;
        let t = c.now_millis();
        assert!(
            t > 1_577_836_800_000,
            "系统时钟早于 2020-01-01，疑似系统时间被设错：{t}"
        );
        assert!(t < 4_102_444_800_000, "系统时钟超过 2100 年，疑似溢出：{t}");
        let t2 = c.now_millis();
        assert!(t2 >= t, "系统时钟不应倒退：{t} → {t2}");
    }
}
