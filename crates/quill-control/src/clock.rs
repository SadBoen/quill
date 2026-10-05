use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync + 'static {
    fn now_millis(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),

            Err(_) => i64::MIN,
        }
    }
}

#[derive(Debug)]
pub struct ManualClock {
    now_ms: AtomicI64,
}

impl ManualClock {
    pub fn new(start_ms: i64) -> Self {
        Self {
            now_ms: AtomicI64::new(start_ms),
        }
    }

    pub fn advance(&self, delta_ms: i64) -> i64 {
        self.now_ms.fetch_add(delta_ms, Ordering::SeqCst) + delta_ms
    }

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

        assert_eq!(c.advance(0), 1_500);
        assert_eq!(c.advance(-200), 1_300);
    }

    #[test]
    fn manual_clock_is_shared_across_threads() {
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
        let c = ManualClock::new(1_000);
        c.set(500);
    }

    #[test]
    fn system_clock_is_after_2020_and_matches_reasonable_magnitude() {
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
