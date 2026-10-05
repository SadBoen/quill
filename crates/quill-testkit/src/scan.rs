//! 泄漏扫描器（三层断言的 L3 层）
//!
//! 依据 `08_测试与验收方案.md` §2.1.1
//!
//! **为什么扫描器本身也要被测**（FALS-05~07）：
//! 扫描器是 AI 写的。若它恒返回"无泄漏"，所有隔离测试都会**假绿** ——
//! 这比没有扫描器更危险，因为它提供虚假的安全感。
//! 所以 `FALS-05` 要求：手动塞一个 canary，扫描器**必须失败**。
//! 这个"扫描器的扫描器"由 `tests/falsify.rs` 实现（不在本 crate 内）。

use crate::canary::TestUser;
use std::path::Path;

/// 一次泄漏命中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeakHit {
    /// 命中位置的可读描述（文件路径 / "日志" / "响应录制" 等）。
    pub location: String,
    /// 命中的 canary 串。
    pub marker: String,
    /// 上下文片段（便于定位）。
    pub context: String,
}

impl std::fmt::Display for LeakHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "🔴 泄漏命中")?;
        writeln!(f, "  位置: {}", self.location)?;
        writeln!(f, "  标记: {}", self.marker)?;
        write!(f, "  上下文: {}", self.context)
    }
}

/// 泄漏扫描器。
///
/// **设计要点：宁可误报，不可漏报。**
/// 扫描不到 = 泄漏已发生且无人知道（最坏）；
/// 误报 = 多花几分钟查（可接受）。
#[derive(Debug, Default)]
pub struct LeakScan {
    hits: Vec<LeakHit>,
}

impl LeakScan {
    /// 新建扫描器。
    pub fn new() -> Self {
        Self { hits: Vec::new() }
    }

    /// 是否发现泄漏。
    pub fn has_leak(&self) -> bool {
        !self.hits.is_empty()
    }

    /// 取全部命中。
    pub fn hits(&self) -> &[LeakHit] {
        &self.hits
    }

    /// ⚠️ **断言无跨用户泄漏。这是隔离测试的 L3 层。**
    ///
    /// `actor` 是发起操作的用户，`victim` 是数据归属的用户。
    /// 扫描 `haystack` 中是否含有 `victim` 的任何 canary 标记。
    ///
    /// **为什么要按"数据归属方"扫，而不是按"发起方"扫**：
    /// 测试意图是"A 触碰 B 的数据" → **A 的上下文里绝不该出现 B 的标记**。
    /// 反过来（B 的上下文里有 A 的标记）是正常的。
    ///
    /// ⚠️ **常见误用**：把 `victim` 传成 `actor` 自己。
    /// 那样扫的是"A 的上下文里有没有 A 自己的标记"——**必然命中**，测试恒失败。
    /// 正确用法：`scan(A 的上下文, victim = B)`。
    pub fn assert_no_cross_user_leak(&mut self, haystack: &str, victim: &TestUser) {
        for mark in victim.all_marks() {
            if let Some(idx) = haystack.find(&mark) {
                self.hits.push(LeakHit {
                    location: "haystack".to_string(),
                    marker: mark.clone(),
                    context: context_around(haystack, idx, &mark),
                });
            }
        }
    }

    /// 扫描目录树（文件内容 + 文件名）是否含泄漏标记。
    pub fn scan_dir(&mut self, root: &Path, victim: &TestUser) -> std::io::Result<()> {
        if !root.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                self.scan_dir(&path, victim)?;
                continue;
            }
            // 文件名也要扫 —— 泄漏可能通过文件名体现
            let name = path.file_name().map(|s| s.to_string_lossy().into_owned());
            if let Some(name) = &name {
                for mark in victim.all_marks() {
                    if name.contains(&mark) {
                        self.hits.push(LeakHit {
                            location: format!("文件名 {}", path.display()),
                            marker: mark.clone(),
                            context: name.clone(),
                        });
                    }
                }
            }
            // 文件内容
            if let Ok(content) = std::fs::read_to_string(&path) {
                for mark in victim.all_marks() {
                    if let Some(idx) = content.find(&mark) {
                        self.hits.push(LeakHit {
                            location: format!("文件 {}", path.display()),
                            marker: mark.clone(),
                            context: context_around(&content, idx, &mark),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// 🔴 **断言无「数据」泄漏**（严格版）：B 的上下文中不得含 A 的任何 canary 标记。
    ///
    /// ## 与 `assert_resource_isolation` 的区别（务必读）
    ///
    /// 主理人裁决 A-9：`global: Semaphore(8)` 是**平台级共享**（合法）。
    /// 这导致两种现象**形态相同但性质相反**：
    ///
    /// | 现象 | 平台级 semaphore（合法） | RoomRegistry 单例（泄漏） |
    /// |---|---|---|
    /// | A 的操作能让 B **排队** | ✅ 合法（资源竞争） | ❌ 泄漏 |
    /// | A 的操作能让 B **看到 A 的数据** | ❌ 不可能（semaphore 不含数据） | ❌ 泄漏 |
    ///
    /// **不区分会导致两条死路**：
    /// 1. 测试红 → 有人把 semaphore 改成 per-user → **引入 OOM 风险**（违背 A-9 裁决）
    /// 2. 或放宽断言 → **真泄漏被放过**
    ///
    /// 所以必须用两个独立 API：
    /// - 本方法：**只管数据**，不管排队
    /// - `assert_resource_isolation`：允许排队，但同样不允许看到数据
    pub fn assert_no_data_leak(&mut self, haystack: &str, victim: &TestUser) {
        self.assert_no_cross_user_leak(haystack, victim)
    }

    /// 🔴 **断言资源隔离**：A 的负载**可以**让 B 排队（合法），但 B 读不到 A 的数据。
    ///
    /// **注意本方法不检查排队** —— 排队是 A-9 裁决下的**预期行为**，
    /// 若把它断言成"不得排队"，会诱导实现者把 semaphore 改成 per-user，从而引入 OOM 风险。
    ///
    /// 排队是否正常，由 [`crate::probe::PermitLedger`] 负责断言（它管的是"槽位是否泄漏"）。
    pub fn assert_resource_isolation(&mut self, haystack: &str, victim: &TestUser) {
        // 当前实现与严格版相同 —— 因为 semaphore 本身不含数据。
        // 保留独立 API 是为了**语义清晰**：将来若加入"排队可观测"字段，
        // 应加到 assert_resource_isolation 而非 assert_no_data_leak。
        self.assert_no_cross_user_leak(haystack, victim)
    }

    /// 扫描单个字符串（日志、响应体等），location 用于错误信息定位。
    pub fn scan_str(&mut self, haystack: &str, location: &str, victim: &TestUser) {
        for mark in victim.all_marks() {
            if let Some(idx) = haystack.find(&mark) {
                self.hits.push(LeakHit {
                    location: location.to_string(),
                    marker: mark.clone(),
                    context: context_around(haystack, idx, &mark),
                });
            }
        }
    }

    /// 失败时打印全部命中并 panic。
    ///
    /// **不能只 panic 第一条** —— 泄漏往往成片出现，只报第一条会误导排查方向。
    pub fn fail_if_leak(&self) {
        assert!(
            !self.has_leak(),
            "发现 {} 处跨用户泄漏：\n{}",
            self.hits.len(),
            self.hits
                .iter()
                .map(|h| h.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// 取命中位置前后各若干字符，便于定位。
fn context_around(haystack: &str, idx: usize, marker: &str) -> String {
    const RADIUS: usize = 40;
    let start = idx.saturating_sub(RADIUS);
    let end = (idx + marker.len() + RADIUS).min(haystack.len());
    // 避免切在多字节字符中间导致 panic
    let mut s = start;
    let mut e = end;
    while s < haystack.len() && !haystack.is_char_boundary(s) {
        s += 1;
    }
    while e < haystack.len() && !haystack.is_char_boundary(e) {
        e += 1;
    }
    let prefix = if s > 0 { "…" } else { "" };
    let suffix = if e < haystack.len() { "…" } else { "" };
    format!("{prefix}{}{suffix}", &haystack[s..e])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canary::test_user;

    #[test]
    fn clean_haystack_passes() {
        // 🔴 这条测试同时防"扫描器过宽"与"调用方传错参数"：
        // 场景是 A 的上下文里只有 A 自己的标记，扫描 victim=B → 必须干净
        let b = test_user("u2");
        let mut s = LeakScan::new();
        s.assert_no_cross_user_leak(
            "会话 ZZQUILLTESTCANARY-u1-sess-x",
            &b, // victim 是 B，不是 A
        );
        assert!(!s.has_leak(), "A 上下文里出现的是 A 自己的标记，不算泄漏");
    }

    #[test]
    fn scanning_own_marks_always_hits() {
        // 反向断言：把 A 当 victim 扫 A 自己的数据，**必然命中**。
        // 这条存在的意义是提醒调用方 —— 参数传错时测试会恒失败（不是恒绿）。
        let a = test_user("u1");
        let mut s = LeakScan::new();
        s.assert_no_cross_user_leak("ZZQUILLTESTCANARY-u1-sess-x", &a);
        assert!(s.has_leak(), "扫自己必然命中");
    }
    #[test]
    fn detects_leak_in_plain_string() {
        // 🔴 这就是 FALS-05 的核心：手动塞 canary，扫描器必须失败
        let victim = test_user("u2");
        let mut s = LeakScan::new();
        s.scan_str(
            "数据库返回: ZZQUILLTESTCANARY-u2-key-sk-abc",
            "响应录制",
            &victim,
        );
        assert!(s.has_leak(), "扫描器必须发现手工植入的 canary");
        assert_eq!(s.hits()[0].marker, "ZZQUILLTESTCANARY-u2-key");
    }

    #[test]
    fn detects_all_nine_resource_kinds() {
        // 🔴 FALS-07：覆盖全部 9 种资源类型，不能只扫一处
        let victim = test_user("u9");
        for kind in [
            "sess", "msg", "soul", "wiki", "raw", "key", "mcp", "team", "mem",
        ] {
            let mut s = LeakScan::new();
            let marker = format!("ZZQUILLTESTCANARY-u9-{kind}");
            s.scan_str(&format!("前缀 {marker} 后缀"), "x", &victim);
            assert!(s.has_leak(), "未检测出 {kind} 类泄漏");
        }
    }

    #[test]
    fn scans_filename_as_well_as_content() {
        let victim = test_user("u2");
        let dir = std::env::temp_dir().join(format!("quill-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 文件名含 canary，内容干净
        std::fs::write(dir.join("ZZQUILLTESTCANARY-u2-wiki-secret.md"), "干净内容").unwrap();

        let mut s = LeakScan::new();
        s.scan_dir(&dir, &victim).unwrap();
        assert!(s.has_leak(), "必须能通过文件名发现泄漏");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multibyte_boundary_does_not_panic() {
        // 上下文截取不能切坏 UTF-8 字符，否则自身会 panic
        let mut s = LeakScan::new();
        let victim = test_user("u2");
        let haystack = format!("中文中文中文{}后文", victim.canary());
        s.scan_str(&haystack, "x", &victim);
        // 能跑完不 panic 即可
    }

    #[test]
    fn both_apis_detect_data_leak() {
        // 🔴 两个 API 都必须能抓到数据泄漏 —— 若其中一个恒绿，它就是假闸门
        let victim = test_user("u2");
        let hay = "泄漏: ZZQUILLTESTCANARY-u2-key-abc";

        let mut s1 = LeakScan::new();
        s1.assert_no_data_leak(hay, &victim);
        assert!(s1.has_leak(), "assert_no_data_leak 必须抓到");

        let mut s2 = LeakScan::new();
        s2.assert_resource_isolation(hay, &victim);
        assert!(s2.has_leak(), "assert_resource_isolation 也必须抓到");
    }

    #[test]
    fn resource_isolation_tolerates_queueing() {
        // 🔴 A-9 裁决：平台级 semaphore 下"排队"是**合法**的。
        // 若这个用例变红，说明有人把 semaphore 改成 per-user → OOM 风险。
        // 排队是预期行为，绝不能被断言成违规。
        let b = test_user("u2");
        let mut s = LeakScan::new();
        // 模拟：B 的响应里只有"排队中"的状态，没有任何 A 的数据
        s.assert_resource_isolation(r#"{"status":"queued","position":3}"#, &b);
        assert!(!s.has_leak(), "排队是合法状态，不该被判为泄漏");
    }

    #[test]
    fn queueing_must_not_be_asserted_as_isolation_failure() {
        // 这条测试锁定上面的语义：排队 ≠ 隔离失效
        // 若将来有人给 assert_resource_isolation 加上"排队即失败"的逻辑，此测试会红
        let b = test_user("u2");
        let mut s = LeakScan::new();
        for status in ["queued", "running", "done"] {
            s.assert_resource_isolation(&format!("{{\"s\":\"{status}\"}}"), &b);
        }
        assert!(!s.has_leak());
    }

    #[test]
    fn empty_haystack_passes() {
        let victim = test_user("u2");
        let mut s = LeakScan::new();
        s.scan_str("", "x", &victim);
        assert!(!s.has_leak());
    }
}
