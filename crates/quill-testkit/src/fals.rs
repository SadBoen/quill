
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRule {

    pub id: String,

    pub desc: String,

    pub pattern: String,
}

impl GateRule {

    pub fn new(id: impl Into<String>, desc: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            desc: desc.into(),
            pattern: pattern.into(),
        }
    }

    pub fn detect(&self, text: &str) -> bool {
        text.contains(&self.pattern)
    }
}

#[derive(Debug, Clone)]
pub struct RuleSelfTest {

    pub rule_id: String,

    pub should_detect: bool,

    pub sample: String,

    pub rationale: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct GateRuleSet {
    rules: Vec<GateRule>,
    self_tests: Vec<RuleSelfTest>,
}

impl GateRuleSet {

    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_rule(&mut self, rule: GateRule) {
        self.rules.push(rule);
    }

    pub fn add_self_test(&mut self, self_test: RuleSelfTest) {
        self.self_tests.push(self_test);
    }

    pub fn scan(&self, text: &str) -> Vec<String> {
        self.rules
            .iter()
            .filter(|r| r.detect(text))
            .map(|r| r.id.clone())
            .collect()
    }

    pub fn scan_dir(&self, root: &std::path::Path) -> std::io::Result<Vec<(String, String)>> {
        let mut hits = Vec::new();
        if !root.exists() {
            return Ok(hits);
        }
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                hits.extend(self.scan_dir(&path)?);
                continue;
            }
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e == "rs" || e == "sql")
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                for id in self.scan(&content) {
                    hits.push((id, path.display().to_string()));
                }
            }
        }
        Ok(hits)
    }

    pub fn run_self_tests(&self) -> std::io::Result<SelfTestReport> {
        let mut failures = Vec::new();
        let mut checked = 0;

        for st in &self.self_tests {
            checked += 1;
            let rule = match self.rules.iter().find(|r| r.id == st.rule_id) {
                Some(r) => r,
                None => {
                    failures.push(format!("[{}] 自检引用了不存在的规则", st.rule_id));
                    continue;
                }
            };
            let detected = rule.detect(&st.sample);
            if detected != st.should_detect {
                failures.push(format!(
                    "[{}] 自检失败（{}）：期望 detect={}，实际={}\n  样例：{}\n  理由：{}",
                    st.rule_id,
                    rule.desc,
                    st.should_detect,
                    detected,
                    truncate(&st.sample, 80),
                    st.rationale
                ));
            }
        }

        for r in &self.rules {
            if !self.self_tests.iter().any(|s| s.rule_id == r.id) {
                failures.push(format!(
                    "[{}] 规则**没有任何自检** —— 未自检的闸门不算闸门",
                    r.id
                ));
            }
        }

        Ok(SelfTestReport { checked, failures })
    }
}

#[derive(Debug, Clone)]
pub struct SelfTestReport {

    pub checked: usize,

    pub failures: Vec<String>,
}

impl SelfTestReport {

    pub fn is_ok(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn assert_ok(&self) {
        assert!(
            self.is_ok(),
            "🚨 闸门自检失败（{} 项失败 / 已检查 {} 项）：\n{}\n\
             ⚠️ 未自检或自检失败的闸门是**假闸门** —— 它提供虚假的安全感。",
            self.failures.len(),
            self.checked,
            self.failures.join("\n")
        );
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let t: String = s.chars().take(n).collect();
    format!("{t}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_rules() -> GateRuleSet {
        let mut s = GateRuleSet::new();
        s.add_rule(GateRule::new(
            "rule7-global-config",
            "禁止 crates/ 出现 Config::global()",
            "Config::global()",
        ));
        s.add_rule(GateRule::new(
            "rule7-paths-config-dir",
            "禁止 crates/ 出现 Paths::config_dir()",
            "Paths::config_dir()",
        ));

        s.add_self_test(RuleSelfTest {
            rule_id: "rule7-global-config".into(),
            should_detect: true,
            sample: "fn f() { let c = goose::Config::global(); }".into(),
            rationale: "含违规调用，必须检出",
        });

        s.add_self_test(RuleSelfTest {
            rule_id: "rule7-global-config".into(),
            should_detect: false,
            sample: "fn f(c: &Config) { let x = c.data_dir(); }".into(),
            rationale: "通过注入的 Config 参数访问，不是全局单例，不该误报",
        });

        s.add_self_test(RuleSelfTest {
            rule_id: "rule7-paths-config-dir".into(),
            should_detect: true,
            sample: "let d = goose::Paths::config_dir();".into(),
            rationale: "含违规调用，必须检出",
        });
        s
    }

    #[test]
    fn self_tests_pass_for_well_built_ruleset() {
        let rep = build_rules().run_self_tests().unwrap();
        rep.assert_ok();
    }

    #[test]
    fn rule_without_self_test_is_reported() {

        let mut s = GateRuleSet::new();
        s.add_rule(GateRule::new("r1", "无自检的规则", "bad"));
        let rep = s.run_self_tests().unwrap();
        assert!(!rep.is_ok());
        assert!(rep.failures[0].contains("没有任何自检"));
    }

    #[test]
    fn detect_returns_precise_rule_id() {

        let s = build_rules();
        let text = "let a = Config::global(); let b = Paths::config_dir();";
        let ids = s.scan(text);
        assert_eq!(ids.len(), 2, "应命中两条规则");
        assert!(ids.contains(&"rule7-global-config".to_string()));
        assert!(ids.contains(&"rule7-paths-config-dir".to_string()));
    }

    #[test]
    fn no_false_positive_on_injected_config_access() {

        let s = build_rules();
        let text = "fn get(cfg: &Config) -> PathBuf { cfg.data_dir().to_owned() }";
        assert!(s.scan(text).is_empty(), "注入式访问被误报了");
    }

    #[test]
    fn broken_self_test_is_detected() {

        let mut s = GateRuleSet::new();
        s.add_rule(GateRule::new("r1", "测试规则", "Config::global()"));
        s.add_self_test(RuleSelfTest {
            rule_id: "r1".into(),
            should_detect: false,
            sample: "Config::global()".into(),
            rationale: "故意构造的自检错误",
        });
        let rep = s.run_self_tests().unwrap();
        assert!(!rep.is_ok(), "错误的自检期望值必须被发现");
    }
}
