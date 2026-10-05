//! FALS 自检辅助：让"闸门必须会红"这件事可复用
//!
//! 依据 `AGENTS.md`（9 条铁律 + 四类失效）与 `08_测试与验收方案.md` §5.2.5
//!
//! **核心命题**：未自检的闸门不算闸门。
//! 一个从未在"已知坏输入"上验证过变红的校验，与没有校验**同等危险** ——
//! 它提供虚假的安全感。
//!
//! **本模块解决的问题**：这个自检模式在多个地方重复出现
//! （boundary 规则 7、canary 扫描器、schema 约束检查、各项阈值门槛），
//! 每处自己写一遍容易漏掉"不该红时不该红"那一半。

/// 规则定义：一条闸门规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRule {
    /// 规则 ID（如 `"rule7"`）。失败信息必须能精确指向它。
    pub id: String,
    /// 规则说明。
    pub desc: String,
    /// 该规则禁止出现的模式（源码或文本片段）。
    pub pattern: String,
}

impl GateRule {
    /// 构造一条规则。
    pub fn new(id: impl Into<String>, desc: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            desc: desc.into(),
            pattern: pattern.into(),
        }
    }

    /// 在文本中检出该规则。
    ///
    /// **返回精确的规则 ID**（而非笼统的"有违规"）——
    /// 否则测试可能因别的原因失败而假绿（这正是 BND-01~06 要防的）。
    pub fn detect(&self, text: &str) -> bool {
        text.contains(&self.pattern)
    }
}

/// 一条规则的自检样例。
#[derive(Debug, Clone)]
pub struct RuleSelfTest {
    /// 所属规则 ID。
    pub rule_id: String,
    /// 该样例**期望**的判定结果。
    pub should_detect: bool,
    /// 用于自检的输入文本。
    pub sample: String,
    /// 样例的构造理由（写入自检报告，便于后人理解为什么要有这条）。
    pub rationale: &'static str,
}

/// 规则集 + 自检。
#[derive(Debug, Clone, Default)]
pub struct GateRuleSet {
    rules: Vec<GateRule>,
    self_tests: Vec<RuleSelfTest>,
}

impl GateRuleSet {
    /// 新建规则集。
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加一条规则。
    pub fn add_rule(&mut self, rule: GateRule) {
        self.rules.push(rule);
    }

    /// 为某条规则添加自检样例。
    ///
    /// **每条规则必须至少有一对自检**：
    /// - 一条"含该违规"的样例 → 期望 `should_detect == true`（证明它会红）
    /// - 一条"不含该违规"的样例 → 期望 `should_detect == false`（证明它不误报）
    pub fn add_self_test(&mut self, self_test: RuleSelfTest) {
        self.self_tests.push(self_test);
    }

    /// 扫描文本，返回命中的规则 ID 列表。
    pub fn scan(&self, text: &str) -> Vec<String> {
        self.rules
            .iter()
            .filter(|r| r.detect(text))
            .map(|r| r.id.clone())
            .collect()
    }

    /// 扫描目录树的所有 `.rs` / `.sql` 文件。
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

    /// 🔴 运行全部自检。**自检本身失败即 assert 失败。**
    ///
    /// 调用位置应是**独立 CI job**（G07 的做法），而非普通测试主链路。
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

        // 🔴 关键检查：每条规则都必须有自检
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

/// 自检报告。
#[derive(Debug, Clone)]
pub struct SelfTestReport {
    /// 已执行的自检数。
    pub checked: usize,
    /// 失败详情（空 = 全部通过）。
    pub failures: Vec<String>,
}

impl SelfTestReport {
    /// 是否全部通过。
    pub fn is_ok(&self) -> bool {
        self.failures.is_empty()
    }

    /// 🔴 报告失败即 assert 失败。**错误信息必须含规则 ID。**
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

    /// ⚠️ **本模块自己的自检**。
    ///
    /// 这段代码存在的意义：如果 `GateRuleSet` 的自检机制本身是坏的
    /// （恒返回通过 / 恒返回失败），那所有用它写的闸门都是假闸门。
    /// 所以我必须先证明它能红。
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
        // 规则 1：会红的样例
        s.add_self_test(RuleSelfTest {
            rule_id: "rule7-global-config".into(),
            should_detect: true,
            sample: "fn f() { let c = goose::Config::global(); }".into(),
            rationale: "含违规调用，必须检出",
        });
        // 规则 1：不误报的样例
        s.add_self_test(RuleSelfTest {
            rule_id: "rule7-global-config".into(),
            should_detect: false,
            sample: "fn f(c: &Config) { let x = c.data_dir(); }".into(),
            rationale: "通过注入的 Config 参数访问，不是全局单例，不该误报",
        });
        // 规则 2：会红
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
        // 🔴 关键：规则缺自检必须被报出来
        let mut s = GateRuleSet::new();
        s.add_rule(GateRule::new("r1", "无自检的规则", "bad"));
        let rep = s.run_self_tests().unwrap();
        assert!(!rep.is_ok());
        assert!(rep.failures[0].contains("没有任何自检"));
    }

    #[test]
    fn detect_returns_precise_rule_id() {
        // 🔴 BND-01~06 的核心：失败信息必须精确指向规则编号
        let s = build_rules();
        let text = "let a = Config::global(); let b = Paths::config_dir();";
        let ids = s.scan(text);
        assert_eq!(ids.len(), 2, "应命中两条规则");
        assert!(ids.contains(&"rule7-global-config".to_string()));
        assert!(ids.contains(&"rule7-paths-config-dir".to_string()));
    }

    #[test]
    fn no_false_positive_on_injected_config_access() {
        // 🔴 不误报用例：走注入的 Config 参数不该被误判
        let s = build_rules();
        let text = "fn get(cfg: &Config) -> PathBuf { cfg.data_dir().to_owned() }";
        assert!(s.scan(text).is_empty(), "注入式访问被误报了");
    }

    #[test]
    fn broken_self_test_is_detected() {
        // 🔴 反向：把"该红"的样例标成"不该红"，自检必须失败
        //     这证明自检机制本身有鉴别力
        let mut s = GateRuleSet::new();
        s.add_rule(GateRule::new("r1", "测试规则", "Config::global()"));
        s.add_self_test(RuleSelfTest {
            rule_id: "r1".into(),
            should_detect: false, // ← 故意写错
            sample: "Config::global()".into(),
            rationale: "故意构造的自检错误",
        });
        let rep = s.run_self_tests().unwrap();
        assert!(!rep.is_ok(), "错误的自检期望值必须被发现");
    }
}
