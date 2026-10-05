use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct DecryptCounter {
    count: Arc<AtomicUsize>,

    touched: Arc<std::sync::Mutex<Vec<String>>>,
}

impl DecryptCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, provider_id: &str) {
        self.count.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut v) = self.touched.lock() {
            v.push(provider_id.to_string());
        }
    }

    pub fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

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

    pub fn assert_never_touched(&self, provider_id: &str) {
        let touched = self.touched.lock().map(|v| v.clone()).unwrap_or_default();
        assert!(
            !touched.iter().any(|p| p == provider_id),
            "❌ 触碰了不该触碰的 provider: {provider_id}（已触碰: {touched:?}）"
        );
    }

    pub fn touched(&self) -> Vec<String> {
        self.touched.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct PanicCounter {
    caught: Arc<AtomicUsize>,
    escaped: Arc<AtomicUsize>,
}

impl PanicCounter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_caught(&self) {
        self.caught.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_escaped(&self) {
        self.escaped.fetch_add(1, Ordering::SeqCst);
    }

    pub fn caught(&self) -> usize {
        self.caught.load(Ordering::SeqCst)
    }

    pub fn escaped(&self) -> usize {
        self.escaped.load(Ordering::SeqCst)
    }

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

#[derive(Debug, Clone)]
pub struct PermitLedger {
    issued: Arc<AtomicUsize>,

    released: Arc<AtomicUsize>,

    capacity: usize,
}

impl PermitLedger {
    pub fn new(capacity: usize) -> Self {
        Self {
            issued: Arc::new(AtomicUsize::new(0)),
            released: Arc::new(AtomicUsize::new(0)),
            capacity,
        }
    }

    pub fn record_issue(&self) {
        self.issued.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_release(&self) {
        self.released.fetch_add(1, Ordering::SeqCst);
    }

    pub fn outstanding(&self) -> usize {
        self.issued
            .load(Ordering::SeqCst)
            .saturating_sub(self.released.load(Ordering::SeqCst))
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

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
        let c = DecryptCounter::new();
        c.record("provider-B");
        assert_eq!(c.count(), 1);

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
        let ledger = PermitLedger::new(2);
        ledger.record_issue();
        ledger.record_issue();

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

        assert!(ledger.outstanding() > ledger.capacity());
    }
}
