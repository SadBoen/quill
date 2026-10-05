
use crate::canary::TestUser;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeakHit {

    pub location: String,

    pub marker: String,

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

#[derive(Debug, Default)]
pub struct LeakScan {
    hits: Vec<LeakHit>,
}

impl LeakScan {

    pub fn new() -> Self {
        Self { hits: Vec::new() }
    }

    pub fn has_leak(&self) -> bool {
        !self.hits.is_empty()
    }

    pub fn hits(&self) -> &[LeakHit] {
        &self.hits
    }

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

    pub fn assert_no_data_leak(&mut self, haystack: &str, victim: &TestUser) {
        self.assert_no_cross_user_leak(haystack, victim)
    }

    pub fn assert_resource_isolation(&mut self, haystack: &str, victim: &TestUser) {

        self.assert_no_cross_user_leak(haystack, victim)
    }

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

fn context_around(haystack: &str, idx: usize, marker: &str) -> String {
    const RADIUS: usize = 40;
    let start = idx.saturating_sub(RADIUS);
    let end = (idx + marker.len() + RADIUS).min(haystack.len());

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

        let b = test_user("u2");
        let mut s = LeakScan::new();
        s.assert_no_cross_user_leak(
            "会话 ZZQUILLTESTCANARY-u1-sess-x",
            &b,
        );
        assert!(!s.has_leak(), "A 上下文里出现的是 A 自己的标记，不算泄漏");
    }

    #[test]
    fn scanning_own_marks_always_hits() {

        let a = test_user("u1");
        let mut s = LeakScan::new();
        s.assert_no_cross_user_leak("ZZQUILLTESTCANARY-u1-sess-x", &a);
        assert!(s.has_leak(), "扫自己必然命中");
    }
    #[test]
    fn detects_leak_in_plain_string() {

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

        std::fs::write(dir.join("ZZQUILLTESTCANARY-u2-wiki-secret.md"), "干净内容").unwrap();

        let mut s = LeakScan::new();
        s.scan_dir(&dir, &victim).unwrap();
        assert!(s.has_leak(), "必须能通过文件名发现泄漏");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multibyte_boundary_does_not_panic() {

        let mut s = LeakScan::new();
        let victim = test_user("u2");
        let haystack = format!("中文中文中文{}后文", victim.canary());
        s.scan_str(&haystack, "x", &victim);

    }

    #[test]
    fn both_apis_detect_data_leak() {

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

        let b = test_user("u2");
        let mut s = LeakScan::new();

        s.assert_resource_isolation(r#"{"status":"queued","position":3}"#, &b);
        assert!(!s.has_leak(), "排队是合法状态，不该被判为泄漏");
    }

    #[test]
    fn queueing_must_not_be_asserted_as_isolation_failure() {

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
