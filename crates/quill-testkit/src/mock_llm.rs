//! Mock LLM provider 的场景脚本模型
//!
//! 依据 `08_测试与验收方案.md` §3.3
//!
//! **本模块只提供脚本模型与故障注入原语的定义与校验**，
//! 不含 HTTP/SSE 服务端（那部分需 workspace 与 axum 就位后实现）。
//!
//! **为什么 mock 走 HTTP 而不是 mock 掉 trait**：
//! 架构 B-1 已定 Provider 走 OpenAI 兼容 HTTP。mock 也走 HTTP → 测的是**真实代码路径**
//! （含 HTTP 解析、SSE 解析、tool_call 解析），而不是把整层替换掉。

use std::collections::BTreeMap;

// ─────────────────────────── 故障注入原语 ───────────────────────────

/// 故障注入类型。
///
/// **这是 mock LLM 的核心价值**：没有它，专家团状态机的全部异常路径测不了。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultInjection {
    /// 注入类型。
    pub kind: FaultKind,
    /// 生效次数（`None` = 永久，如 `hang`）。
    pub times: Option<u32>,
    /// 参数（如 `stream_cut` 的 `at_chunk`、`slow` 的 `ms`）。
    pub param: Option<u32>,
}

impl Default for FaultInjection {
    /// 默认构造用 `Http5xx`（最常见的可重试故障）。
    ///
    /// **不用 `#[derive(Default)]`**：那会给 `FaultKind` 强加一个
    /// "默认故障类型"的语义，而枚举的默认值应该显式选择而非隐式落第一个变体。
    fn default() -> Self {
        Self {
            kind: FaultKind::Http5xx,
            times: None,
            param: None,
        }
    }
}

/// 故障类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// HTTP 500 —— 测重试逻辑。
    Http5xx,
    /// HTTP 429 限流 —— 测退避。
    Http429,
    /// 流中途断开 —— 测半截回复处理。
    StreamCut,
    /// 慢响应 —— 测超时（配合 `tokio::time::pause`）。
    Slow,
    /// 永久挂起 —— 测超时**必须**生效。
    Hang,
    /// 畸形 SSE —— 测 SSE 解析容错。
    MalformedSse,
    /// 畸形 tool_call JSON —— 测 JSON 解析容错。
    MalformedToolCall,
    /// 未知事件类型 —— 测前向兼容。
    UnknownEvent,
    /// 超长响应 —— 测响应体上限。
    Huge,
}

impl FaultKind {
    /// 从 YAML 字符串解析。
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

    /// 是否为"永久"故障（需被超时机制终止，而非重试恢复）。
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Hang)
    }
}

// ─────────────────────────── 场景脚本模型 ───────────────────────────

/// 步骤匹配条件。
///
/// **关键设计：按内容匹配，不按顺序。**
/// 并行派工时步骤顺序不确定，若按序号匹配则测试不可靠。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MatchCond {
    /// 消息角色（如 `user` / `tool`）。
    pub role: Option<String>,
    /// 消息内容须包含的子串。
    pub contains: Option<String>,
    /// 工具名（当 `role == "tool"`）。
    pub tool: Option<String>,
}

impl MatchCond {
    /// 构造：按用户消息内容匹配。
    pub fn user_contains(s: &str) -> Self {
        Self {
            role: Some("user".into()),
            contains: Some(s.into()),
            ..Default::default()
        }
    }

    /// 构造：按工具调用匹配。
    pub fn tool_named(tool: &str, contains: &str) -> Self {
        Self {
            role: Some("tool".into()),
            tool: Some(tool.into()),
            contains: Some(contains.into()),
        }
    }
}

/// 单个脚本步骤。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Step {
    /// 匹配条件。
    pub match_cond: MatchCond,
    /// 该步注入的故障（可选）。
    pub fault: Option<FaultInjection>,
}

/// 一个完整的场景脚本。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scenario {
    /// 场景名（用于测试报告与失败信息）。
    pub name: String,
    /// 步骤序列。
    pub steps: Vec<Step>,
}

impl Scenario {
    /// 从 YAML 解析。
    ///
    /// **必须在启动时解析并校验**：脚本语法错误要**立即失败并打印行号**，
    /// 不能等到运行时静默走空（MOCK-02）。
    pub fn from_yaml(yaml: &str) -> Result<Self, ScriptError> {
        // 极简解析：只支持 name / steps[].match / steps[].emit / steps[].inject
        // 完整 YAML 解析需 serde_yaml，等 workspace 就位后替换
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

    /// 找到匹配 `contains` 的第一个步骤。
    ///
    /// **MOCK-03：全不命中时必须返回 `None` 而非空回复** ——
    /// 静默返回空会让"派工失败"看起来像"正常返回但内容为空"。
    pub fn find_step(&self, contains: &str) -> Option<&Step> {
        self.steps.iter().find(|s| {
            s.match_cond
                .contains
                .as_deref()
                .map(|c| contains.contains(c))
                .unwrap_or(false)
        })
    }

    /// 校验脚本合理性（启动时调用）。
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

/// 脚本错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    /// 语法错误（含行号）。
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
    // 形如 {kind: http_500, times: 2, param: 3}
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

// ─────────────────────────── 脚本样例 ───────────────────────────

/// 团队派工场景样例（YAML 文本）。
///
/// **注意**：本常量只用于测试本模块的解析能力，**不是**给具体用例用的。
/// 具体场景脚本应放到 `tests/fixtures/scenarios/` 下。
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
        // 🔴 MOCK-02：语法错误必须**启动即失败**，不能运行时静默走空
        let r = Scenario::from_yaml("steps:\n  - match: { role: user }");
        assert!(r.is_err(), "缺 name 必须报错");
    }

    #[test]
    fn no_match_returns_none_not_empty() {
        // 🔴 MOCK-03：全不命中时返回 None，绝不返回"空回复"
        let s = Scenario::from_yaml(SAMPLE_TEAM_SCENARIO).unwrap();
        assert!(s.find_step("完全无关的内容").is_none());
    }

    #[test]
    fn match_is_by_content_not_order() {
        // 关键设计：并行派工时顺序不确定，必须按内容匹配
        let s = Scenario::from_yaml(SAMPLE_TEAM_SCENARIO).unwrap();
        // 第二个条件是 "__FINAL__"，但它排在最后 —— 仍应能匹配到
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
        // 永久故障必须靠超时机制终止，不能靠重试
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
