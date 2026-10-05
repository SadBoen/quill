use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultInjection {
    pub kind: FaultKind,

    pub times: Option<u32>,

    pub param: Option<u32>,
}

impl Default for FaultInjection {
    fn default() -> Self {
        Self {
            kind: FaultKind::Http5xx,
            times: None,
            param: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    Http5xx,

    Http429,

    StreamCut,

    Slow,

    Hang,

    MalformedSse,

    MalformedToolCall,

    UnknownEvent,

    Huge,
}

impl FaultKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "http_500" => Self::Http5xx,
            "http_429" => Self::Http429,
            "stream_cut" => Self::StreamCut,
            "slow" => Self::Slow,
            "hang" => Self::Hang,
            "malformed_sse" => Self::MalformedSse,
            "malformed_toolcall_json" => Self::MalformedToolCall,
            "unknown_event" => Self::UnknownEvent,
            "huge" => Self::Huge,
            _ => return None,
        })
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Hang)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MatchCond {
    pub role: Option<String>,

    pub contains: Option<String>,

    pub tool: Option<String>,
}

impl MatchCond {
    pub fn user_contains(s: &str) -> Self {
        Self {
            role: Some("user".into()),
            contains: Some(s.into()),
            ..Default::default()
        }
    }

    pub fn tool_named(tool: &str, contains: &str) -> Self {
        Self {
            role: Some("tool".into()),
            tool: Some(tool.into()),
            contains: Some(contains.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Step {
    pub match_cond: MatchCond,

    pub fault: Option<FaultInjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scenario {
    pub name: String,

    pub steps: Vec<Step>,
}

impl Scenario {
    pub fn from_yaml(yaml: &str) -> Result<Self, ScriptError> {
        let mut name = String::new();
        let mut steps = Vec::new();
        let mut cur: Option<Step> = None;

        for (i, raw) in yaml.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(n) = line.strip_prefix("name:") {
                name = n.trim().to_string();
            } else if line.starts_with("- match:") {
                if let Some(s) = cur.take() {
                    steps.push(s);
                }
                let cond = line
                    .strip_prefix("- match:")
                    .unwrap()
                    .trim()
                    .trim_start_matches('{')
                    .trim_end_matches('}');
                cur = Some(Step {
                    match_cond: parse_match(cond),
                    fault: None,
                });
            } else if let Some(f) = line.strip_prefix("inject:") {
                if let Some(s) = cur.as_mut() {
                    s.fault = Some(parse_fault(f.trim()));
                } else {
                    return Err(ScriptError::Syntax {
                        line: i + 1,
                        msg: "inject: 出现在 match: 之前".into(),
                    });
                }
            }
        }
        if let Some(s) = cur.take() {
            steps.push(s);
        }

        if name.is_empty() {
            return Err(ScriptError::Syntax {
                line: 0,
                msg: "缺少 name: 字段".into(),
            });
        }
        if steps.is_empty() {
            return Err(ScriptError::Syntax {
                line: 0,
                msg: "场景没有任何 steps".into(),
            });
        }

        Ok(Self { name, steps })
    }

    pub fn find_step(&self, contains: &str) -> Option<&Step> {
        self.steps.iter().find(|s| {
            s.match_cond
                .contains
                .as_deref()
                .map(|c| contains.contains(c))
                .unwrap_or(false)
        })
    }

    pub fn validate(&self) -> Result<(), ScriptError> {
        if self.steps.len() > 32 {
            return Err(ScriptError::Syntax {
                line: 0,
                msg: format!("步骤数 {} 过多（上限 32）", self.steps.len()),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    Syntax { line: usize, msg: String },
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::Syntax { line, msg } if *line > 0 => {
                write!(f, "脚本语法错误（第 {line} 行）: {msg}")
            }
            ScriptError::Syntax { msg, .. } => write!(f, "脚本语法错误: {msg}"),
        }
    }
}

impl std::error::Error for ScriptError {}

fn parse_match(s: &str) -> MatchCond {
    let mut cond = MatchCond::default();
    for part in s.split(',') {
        if let Some((k, v)) = part.split_once(':') {
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "role" => cond.role = Some(v),
                "contains" => cond.contains = Some(v),
                "tool" => cond.tool = Some(v),
                _ => {}
            }
        }
    }
    cond
}

fn parse_fault(s: &str) -> FaultInjection {
    let inner = s.trim().trim_start_matches('{').trim_end_matches('}');
    let map: BTreeMap<&str, &str> = inner
        .split(',')
        .filter_map(|p| p.split_once(':'))
        .map(|(k, v)| (k.trim(), v.trim().trim_matches('"')))
        .collect();
    let kind = map
        .get("kind")
        .and_then(|k| FaultKind::parse(k))
        .unwrap_or(FaultKind::Http5xx);
    FaultInjection {
        kind,
        times: map.get("times").and_then(|t| t.parse().ok()),
        param: map.get("param").and_then(|p| p.parse().ok()),
    }
}

pub const SAMPLE_TEAM_SCENARIO: &str = r#"
name: team_two_members
steps:
  - match: { role: user, contains: "分析这份合同的风险" }
    inject: { kind: http_500, times: 1 }
  - match: { role: user, contains: "__FINAL__" }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scenario_name_and_steps() {
        let s = Scenario::from_yaml(SAMPLE_TEAM_SCENARIO).unwrap();
        assert_eq!(s.name, "team_two_members");
        assert_eq!(s.steps.len(), 2);
    }

    #[test]
    fn parses_fault_injection() {
        let s = Scenario::from_yaml(SAMPLE_TEAM_SCENARIO).unwrap();
        let f = s.steps[0].fault.as_ref().unwrap();
        assert_eq!(f.kind, FaultKind::Http5xx);
        assert_eq!(f.times, Some(1));
    }

    #[test]
    fn missing_name_is_rejected() {
        let r = Scenario::from_yaml("steps:\n  - match: { role: user }");
        assert!(r.is_err(), "缺 name 必须报错");
    }

    #[test]
    fn no_match_returns_none_not_empty() {
        let s = Scenario::from_yaml(SAMPLE_TEAM_SCENARIO).unwrap();
        assert!(s.find_step("完全无关的内容").is_none());
    }

    #[test]
    fn match_is_by_content_not_order() {
        let s = Scenario::from_yaml(SAMPLE_TEAM_SCENARIO).unwrap();

        let st = s.find_step("现在请 __FINAL__ 汇总").unwrap();
        assert!(st
            .match_cond
            .contains
            .as_ref()
            .unwrap()
            .contains("__FINAL__"));
    }

    #[test]
    fn hang_is_marked_permanent() {
        assert!(FaultKind::Hang.is_permanent());
        assert!(!FaultKind::Http5xx.is_permanent());
    }

    #[test]
    fn all_fault_kinds_parse() {
        for (s, expect) in [
            ("http_500", FaultKind::Http5xx),
            ("http_429", FaultKind::Http429),
            ("stream_cut", FaultKind::StreamCut),
            ("slow", FaultKind::Slow),
            ("hang", FaultKind::Hang),
            ("malformed_sse", FaultKind::MalformedSse),
            ("malformed_toolcall_json", FaultKind::MalformedToolCall),
            ("unknown_event", FaultKind::UnknownEvent),
            ("huge", FaultKind::Huge),
        ] {
            assert_eq!(FaultKind::parse(s), Some(expect), "解析 {s} 失败");
        }
    }
}
