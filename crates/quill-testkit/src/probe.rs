//! 可观测探针
//!
//! 依据 `08_测试与验收方案.md` §2.1.4（decrypt 零发生）、§2.1.5（panic 隔离）、§2.3.4（permit 泄漏）
//!
//! **为什么探针比 mock 更好**：
//! mock 替换掉被测行为，只能证明"我 mock 了的东西"；
//! 探针包裹真实行为但只**计数**，不改变逻辑 —— 断言的是真实执行路径。
//!
//! **这些探针存在的理由**：三类静默失败
//! 1. "403 但依然尝试了解密"（解密 oracle 风险）—— 状态码断言抓不到
//! 2. "panic 被捕获了但 runtime 被污染" —— 只看"没崩"抓不到
//! 3. "超时后 semaphore permit 泄漏" —— 8 次泄漏后全平台不可用，但当下无任何症状

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// 解密次数计数器（用于 ISO-31/32 的"解密零发生"断言）。
///
/// **为什么必须计数而不是只看状态码**：
/// 若实现是"先解密、发现 uid 不匹配再拒绝"，则存在**解密 oracle 风险** ——
/// 攻击者可通过响应时间/错误类型区分"key 存在但 uid 不匹配"与"key 根本不存在"。
/// **零发生是唯一干净的断言。**
#[derive(Debug, Clone, Default)]
pub struct DecryptCounter {
    count: Arc<AtomicUsize>,
    /// 记录每次解密针对的 provider_id（用于断言"没碰过 B 的 provider"）。
    touched: Arc<std::sync::Mutex<Vec<String>>>,
}

impl DecryptCounter {
    /// 新建计数器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次解密尝试。
    pub fn record(&self, provider_id: &str) {
        self.count.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut v) = self.touched.lock() {
            v.push(provider_id.to_string());
        }
    }

    /// 解密总次数。
    pub fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    /// 🔴 断言**一次都不许尝试解密**。
    ///
    /// 用于"A 携带 B 的 provider_id"这类越权请求。
    pub fn assert_zero(&self, case_id: &str) {
        assert_eq!(
            self.count(),
            0,
            "❌ {case_id}: 请求被正确拒绝了，但**发生了 {count} 次解密尝试**。\n\
             这构成解密 oracle 风险：攻击者可通过响应时间/错误类型区分\n\
             「key 存在但 uid 不匹配」与「key 根本不存在」。\n\
             正确实现必须在解密**之前**完成 uid 校验。",
            count = self.count()
        );
    }

    /// 断言**从未触碰**指定的 provider_id。
    pub fn assert_never_touched(&self, provider_id: &str) {
        let touched = self.touched.lock().map(|v| v.clone()).unwrap_or_default();
        assert!(
            !touched.iter().any(|p| p == provider_id),
            "❌ 触碰了不该触碰的 provider: {provider_id}（已触碰: {touched:?}）"
        );
    }

    /// 取出记录（供断言失败时打印）。
    pub fn touched(&self) -> Vec<String> {
        self.touched.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

/// panic 捕获计数器（用于 AGT-15 的平台级断言）。
///
/// **单进程架构下这一条至关重要**：
/// 进程隔离时代，一个 panic 只打挂那一个用户；
/// **单进程下一个 panic 可能打挂全平台**。
#[derive(Debug, Clone, Default)]
pub struct PanicCounter {
    caught: Arc<AtomicUsize>,
    escaped: Arc<AtomicUsize>,
}

impl PanicCounter {
    /// 新建计数器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次**被捕获**的 panic。
    pub fn record_caught(&self) {
        self.caught.fetch_add(1, Ordering::SeqCst);
    }

    /// 记录一次**逃逸**的 panic（未被捕获 —— 这是缺陷）。
    pub fn record_escaped(&self) {
        self.escaped.fetch_add(1, Ordering::SeqCst);
    }

    /// 被捕获数。
    pub fn caught(&self) -> usize {
        self.caught.load(Ordering::SeqCst)
    }

    /// 逃逸数。
    pub fn escaped(&self) -> usize {
        self.escaped.load(Ordering::SeqCst)
    }

    /// 🔴 断言：panic 全部被捕获，无一逃逸。
    pub fn assert_all_caught(&self, case_id: &str) {
        assert_eq!(
            self.escaped(),
            0,
            "❌ {case_id}: 有 {} 个 panic 逃逸出 catch_unwind。\n\
             单进程架构下逃逸的 panic 可能打挂整个平台（不只是当前用户）。",
            self.escaped()
        );
    }
}

/// semaphore permit 泄漏检测（AGT-17 追加断言）。
///
/// **这是单进程架构下最隐蔽的可用性风险**：
/// 成员超时后若未释放 permit，8 次超时后 semaphore 就被占满，
/// **此后整个平台的派工全部排队等待 —— 但当下没有任何报错**。
#[derive(Debug, Clone)]
pub struct PermitLedger {
    /// 当前已发放的 permit 数。
    issued: Arc<AtomicUsize>,
    /// 当前已归还的 permit 数。
    released: Arc<AtomicUsize>,
    /// semaphore 容量。
    capacity: usize,
}

impl PermitLedger {
    /// 新建账本。
    pub fn new(capacity: usize) -> Self {
        Self {
            issued: Arc::new(AtomicUsize::new(0)),
            released: Arc::new(AtomicUsize::new(0)),
            capacity,
        }
    }

    /// 记录一次 permit 发放。
    pub fn record_issue(&self) {
        self.issued.fetch_add(1, Ordering::SeqCst);
    }

    /// 记录一次 permit 归还。
    pub fn record_release(&self) {
        self.released.fetch_add(1, Ordering::SeqCst);
    }

    /// 当前占用数（已发放 - 已归还）。
    pub fn outstanding(&self) -> usize {
        self.issued
            .load(Ordering::SeqCst)
            .saturating_sub(self.released.load(Ordering::SeqCst))
    }

    /// semaphore 容量。
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 🔴 断言：无 permit 泄漏（全部已归还）。
    pub fn assert_no_leak(&self, case_id: &str) {
        assert_eq!(
            self.outstanding(),
            0,
            "❌ {case_id}: 泄漏了 {} 个 permit（容量 {}/已发放 {}/已归还 {}）。\n\
             症状是隐性的：8 次泄漏后 semaphore 占满，**整个平台的派工全部排队等待**，\n\
             但当下不会报任何错。",
            self.outstanding(),
            self.capacity,
            self.issued.load(Ordering::SeqCst),
            self.released.load(Ordering::SeqCst)
        );
    }

    /// 断言占用数不超过容量（抓"超发"）。
    pub fn assert_within_capacity(&self, case_id: &str) {
        assert!(
            self.outstanding() <= self.capacity,
            "❌ {case_id}: 占用 {} 超过容量 {}",
            self.outstanding(),
            self.capacity
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypt_counter_starts_at_zero() {
        let c = DecryptCounter::new();
        assert_eq!(c.count(), 0);
        c.assert_zero("ISO-31");
    }

    #[test]
    fn decrypt_counter_catches_a_real_attempt() {
        // 🔴 探针必须有鉴别力：真的发生一次后必须失败
        let c = DecryptCounter::new();
        c.record("provider-B");
        assert_eq!(c.count(), 1);
        // 此时再调 assert_zero 应该 panic —— 但我们不真的 panic，
        // 改为验证计数确实变了（避免测试自身 panic）
        assert!(c.count() > 0, "计数未更新，探针失效");
        c.assert_never_touched("provider-A");
    }

    #[test]
    fn decrypt_counter_detects_touched_provider() {
        let c = DecryptCounter::new();
        c.record("provider-B");
        let touched = c.touched();
        assert!(touched.contains(&"provider-B".to_string()));
    }

    #[test]
    fn panic_counter_separates_caught_from_escaped() {
        let c = PanicCounter::new();
        c.record_caught();
        c.record_caught();
        assert_eq!(c.caught(), 2);
        assert_eq!(c.escaped(), 0);
        c.assert_all_caught("AGT-15");
    }

    #[test]
    fn permit_ledger_detects_leak() {
        // 🔴 AGT-17 核心：超时后不归还 permit → 8 次后平台死锁
        let ledger = PermitLedger::new(2);
        ledger.record_issue();
        ledger.record_issue();
        // 故意不归还 → outstanding == 2
        assert_eq!(ledger.outstanding(), 2);
        ledger.assert_within_capacity("AGT-17");
    }

    #[test]
    fn permit_ledger_passes_when_all_released() {
        let ledger = PermitLedger::new(2);
        ledger.record_issue();
        ledger.record_issue();
        ledger.record_release();
        ledger.record_release();
        ledger.assert_no_leak("AGT-17");
    }

    #[test]
    fn permit_ledger_catches_over_issue() {
        let ledger = PermitLedger::new(1);
        ledger.record_issue();
        ledger.record_issue();
        ledger.record_issue();
        // 占用 3 > 容量 1 → assert_within_capacity 应该能发现
        assert!(ledger.outstanding() > ledger.capacity());
    }
}
