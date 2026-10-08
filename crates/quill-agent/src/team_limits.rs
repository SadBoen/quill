//! 专家团的运行限制：`teams` 表的 `guidelines` / `max_dispatch` / `max_replan` /
//! `max_ask_depth` 四列在**生产路径**里的读取口径（队列 Q043）。
//!
//! 四列是 quill 0001 schema 自带的（`.octop-ref/octop` 与 `vendor/goose` 里
//! grep 不到同名概念，没有上游可抄），所以语义在这里定下来，并逐条写清
//! 「超限会怎样」—— 不允许出现「读了不用」的限制：
//!
//! - `max_dispatch`：**一轮派工**最多几个成员。超了**拒绝整轮**（不执行任何一个
//!   成员、不写台账），不做「只派前 N 个」—— 静默少派会让主持人以为全派出去了。
//! - `max_replan`：允许的**重新规划轮数**。`round 0` 是首派，`round 1..=max_replan`
//!   是重规划；`round > max_replan` 拒绝整轮。理由是每一轮都是一次新的模型开销，
//!   不设闸门时「再规划一轮」永远不会停。
//! - `max_ask_depth`：成员提问的最大深度。**目前尚未接线**（诚实记录，不假装已生效）：
//!   现在的成员执行器是「一次调用、没有追问通道」——`crates/quill-server/src/
//!   member_executor.rs` 的固定前缀明确要求成员「不要提问」，`DispatchRecord::mark_asking`
//!   在生产侧没有任何调用点（只有 `dispatch_ledger` 重建旧记录与测试会走到）。
//!   这一列**读得出、报得出、改得到**，但还没有生效点；等「提问」链路真的出现时，
//!   闸门应加在成员转 `ASKING` 的那一处（`Dispatcher::run_one` 或
//!   `SqlxDispatchLedger` 重建记录时），而不是在这里凭空造一个不会被调用的判断。
//! - `guidelines`：团队行为约束。派工时**逐字进每个成员的提示**（任务正文之前），
//!   所以它是全团队共享的行为约束，不是某一项任务的说明。
//!
//! 取值范围与 `crates/quill-store/migrations/0001_init.sql:308-310` 的 CHECK 同口径
//! （`max_dispatch` 2~8，`max_replan` 0~5，`max_ask_depth` 0~5）—— 两边必须一致，
//! 否则 API 放行的值会被数据库拒掉，或数据库放行的值让派工侧带病运行。

use crate::error::AgentError;

/// 与 `0001_init.sql:297` 的 `DEFAULT 4` 同值。
pub const DEFAULT_MAX_DISPATCH: u32 = 4;

/// 与 `0001_init.sql:298` 的 `DEFAULT 2` 同值。
pub const DEFAULT_MAX_REPLAN: u32 = 2;

/// 与 `0001_init.sql:299` 的 `DEFAULT 3` 同值。
pub const DEFAULT_MAX_ASK_DEPTH: u32 = 3;

/// 与 `0001_init.sql:308` 的 `CHECK (max_dispatch BETWEEN 2 AND 8)` 同口径。
pub const MIN_MAX_DISPATCH: u32 = 2;

/// 同 [`MIN_MAX_DISPATCH`]，另一头。
pub const MAX_MAX_DISPATCH: u32 = 8;

/// 与 `0001_init.sql:309` 的 `CHECK (max_replan BETWEEN 0 AND 5)` 同口径。
pub const MAX_MAX_REPLAN: u32 = 5;

/// 与 `0001_init.sql:310` 的 `CHECK (max_ask_depth BETWEEN 0 AND 5)` 同口径。
pub const MAX_MAX_ASK_DEPTH: u32 = 5;

/// 团队准则的长度上限（**按字符**，不是字节）。
///
/// 这个数不是拍的：准则会**逐字进每一个成员的提示**，一轮最多派
/// `MAX_MAX_DISPATCH` = 8 个成员，所以它是 8 倍开销。专家人格的上限是 20000
/// （`expert::MAX_INSTRUCTIONS_CHARS`），团队准则取它的十分之一（2000）——
/// 一句团队号令不该和整份人格一样长。超限报错，**不截断**：截断的准则只发出去
/// 一半，而模型会照着那半句行事，比直接拒绝难查得多。
pub const MAX_GUIDELINES_CHARS: usize = 2_000;

/// 一个团队的运行限制。构造即校验（范围与长度），所以拿到手的值一定可用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamLimits {
    max_dispatch: u32,
    max_replan: u32,
    max_ask_depth: u32,
    guidelines: String,
}

impl Default for TeamLimits {
    fn default() -> Self {
        Self {
            max_dispatch: DEFAULT_MAX_DISPATCH,
            max_replan: DEFAULT_MAX_REPLAN,
            max_ask_depth: DEFAULT_MAX_ASK_DEPTH,
            guidelines: String::new(),
        }
    }
}

/// 限制列本身不合法（越过 API 直写、或 API 校验漏了）。
///
/// 与 [`AgentError::TeamLimitsExceeded`] 是两件事：那是「这次派工超了」，
/// 这是「这个团的限制值本身就存坏了」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamLimitsError {
    /// 超出 schema 允许的范围。
    OutOfRange {
        field: &'static str,
        got: i64,
        min: u32,
        max: u32,
    },

    /// 团队准则超过长度上限。
    GuidelinesTooLong { chars: usize, max: usize },
}

impl TeamLimitsError {
    /// 中文的「下一步」。与 `AgentError` 的 `fix_command` 同一用意：错误文案
    /// 必须给出能照着做的一步，否则用户只能猜。
    pub fn next_step(&self) -> String {
        match self {
            Self::OutOfRange {
                field, min, max, ..
            } => {
                format!("把 {field} 改成 {min}~{max} 之间的整数（与数据库 CHECK 同口径），再重试。")
            }
            Self::GuidelinesTooLong { max, .. } => format!(
                "把 guidelines 压到 {max} 个字符以内（团队准则会逐字进每个成员的提示，\
                 8 个成员就是 8 倍开销），再重试。"
            ),
        }
    }
}

impl std::fmt::Display for TeamLimitsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange {
                field,
                got,
                min,
                max,
            } => write!(f, "团队限制 {field} 的值 {got} 超出允许范围（{min}~{max}）"),
            Self::GuidelinesTooLong { chars, max } => {
                write!(f, "团队准则太长（{chars} 个字符，上限 {max}）")
            }
        }
    }
}

impl std::error::Error for TeamLimitsError {}

impl TeamLimits {
    /// 构造 + 校验。范围与 `0001_init.sql` 的 CHECK 同口径，准则长度见
    /// [`MAX_GUIDELINES_CHARS`]。
    pub fn new(
        max_dispatch: u32,
        max_replan: u32,
        max_ask_depth: u32,
        guidelines: impl Into<String>,
    ) -> Result<Self, TeamLimitsError> {
        let guidelines = guidelines.into();
        if !(MIN_MAX_DISPATCH..=MAX_MAX_DISPATCH).contains(&max_dispatch) {
            return Err(TeamLimitsError::OutOfRange {
                field: "max_dispatch",
                got: i64::from(max_dispatch),
                min: MIN_MAX_DISPATCH,
                max: MAX_MAX_DISPATCH,
            });
        }
        if max_replan > MAX_MAX_REPLAN {
            return Err(TeamLimitsError::OutOfRange {
                field: "max_replan",
                got: i64::from(max_replan),
                min: 0,
                max: MAX_MAX_REPLAN,
            });
        }
        if max_ask_depth > MAX_MAX_ASK_DEPTH {
            return Err(TeamLimitsError::OutOfRange {
                field: "max_ask_depth",
                got: i64::from(max_ask_depth),
                min: 0,
                max: MAX_MAX_ASK_DEPTH,
            });
        }
        let chars = guidelines.chars().count();
        if chars > MAX_GUIDELINES_CHARS {
            return Err(TeamLimitsError::GuidelinesTooLong {
                chars,
                max: MAX_GUIDELINES_CHARS,
            });
        }
        Ok(Self {
            max_dispatch,
            max_replan,
            max_ask_depth,
            guidelines,
        })
    }

    pub fn max_dispatch(&self) -> u32 {
        self.max_dispatch
    }

    pub fn max_replan(&self) -> u32 {
        self.max_replan
    }

    pub fn max_ask_depth(&self) -> u32 {
        self.max_ask_depth
    }

    pub fn guidelines(&self) -> &str {
        &self.guidelines
    }

    /// 准则的有效字符数（去掉首尾空白后计），用于如实回报「这份准则
    /// 到底有没有进提示」。
    pub fn guidelines_chars(&self) -> usize {
        self.guidelines.trim().chars().count()
    }

    pub fn has_guidelines(&self) -> bool {
        !self.guidelines.trim().is_empty()
    }

    /// 一轮派工的成员数闸门。**必须在写台账与起成员之前调用** ——
    /// 被拒的一轮不留任何记录，调用方才能「改小成员数再重发」。
    pub fn check_dispatch(&self, planned: usize) -> Result<(), AgentError> {
        // usize → u32 只可能在上界被截断；截断后的值只会更大，
        // 不会把一个超限的请求放过去。
        let planned = u32::try_from(planned).unwrap_or(u32::MAX);
        if planned > self.max_dispatch {
            return Err(AgentError::TeamLimitsExceeded {
                limit: "max_dispatch（一轮最多派几个成员）",
                got: planned,
                max: self.max_dispatch,
                next_step: format!(
                    "把这一轮的 members 减到 {} 个以内，或 PATCH /api/teams/{{id}} \
                     调大 max_dispatch（上限 {MAX_MAX_DISPATCH}）。",
                    self.max_dispatch
                ),
            });
        }
        Ok(())
    }

    /// 重新规划闸门：`round 0` 是首派，`round 1..=max_replan` 是重规划。
    pub fn check_replan(&self, round: u32) -> Result<(), AgentError> {
        if round > self.max_replan {
            return Err(AgentError::TeamLimitsExceeded {
                limit: "max_replan（最多重新规划几轮）",
                got: round,
                max: self.max_replan,
                next_step: format!(
                    "round 0 是首派，1~{} 才是重新规划：把这一轮并回已用过的轮次，\
                     或 PATCH /api/teams/{{id}} 调大 max_replan（上限 {MAX_MAX_REPLAN}）。",
                    self.max_replan
                ),
            });
        }
        Ok(())
    }

    /// 把团队准则拼进给成员的任务正文。
    ///
    /// 空准则**逐字返回原文** —— 不硬塞一个空标题，否则每个成员的提示里都会多出
    /// 一段没有内容的「准则」，纯属噪声。
    pub fn apply_guidelines(&self, instructions: &str) -> String {
        let g = self.guidelines.trim();
        if g.is_empty() {
            return instructions.to_string();
        }
        format!("【团队准则（本团所有成员共同遵守）】\n{g}\n\n【本次任务】\n{instructions}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_dispatch: u32, max_replan: u32) -> TeamLimits {
        TeamLimits::new(max_dispatch, max_replan, 3, "结论必须给出数据来源")
            .expect("测试用的限制值应合法")
    }

    #[test]
    fn a_round_over_the_member_cap_is_rejected_with_the_numbers_and_a_next_step() {
        let l = limits(4, 2);
        let err = l
            .check_dispatch(5)
            .expect_err("5 个成员超过上限 4，必须判红");
        assert_eq!(err.code(), "team_limits_exceeded");
        let msg = err.to_string();
        assert!(msg.contains('5'), "要带本次数量：{msg}");
        assert!(msg.contains('4'), "要带上限：{msg}");
        assert!(msg.contains("max_dispatch"), "要点名是哪条限制：{msg}");
        assert!(msg.contains("下一步"), "必须给下一步：{msg}");
        assert!(
            err.fix_command().contains("doctor"),
            "错误要指向可诊断的命令：{}",
            err.fix_command()
        );
    }

    #[test]
    fn a_round_at_the_member_cap_passes() {
        let l = limits(4, 2);
        assert!(l.check_dispatch(4).is_ok(), "上限本身必须放行");
        assert!(l.check_dispatch(1).is_ok());
    }

    #[test]
    fn a_round_beyond_the_replan_cap_is_rejected_and_the_first_round_always_passes() {
        let l = limits(4, 2);
        assert!(l.check_replan(0).is_ok(), "round 0 是首派，永远放行");
        assert!(l.check_replan(2).is_ok(), "上限本身必须放行");
        let err = l
            .check_replan(3)
            .expect_err("round 3 超过 max_replan=2 必须判红");
        let msg = err.to_string();
        assert!(msg.contains('3') && msg.contains('2'), "{msg}");
        assert!(msg.contains("max_replan"), "{msg}");
        assert!(msg.contains("下一步"), "{msg}");
    }

    #[test]
    fn guidelines_are_prepended_to_the_member_instructions_in_order() {
        let l = TeamLimits::new(4, 2, 3, "  风险评级必须写清依据  ").expect("合法");
        let out = l.apply_guidelines("分析 Q3 成本");
        let g_at = out.find("风险评级必须写清依据").expect("准则要在");
        let t_at = out.find("分析 Q3 成本").expect("任务要在");
        assert!(g_at < t_at, "准则必须排在任务正文之前：{out}");
        assert!(l.has_guidelines());
        assert_eq!(l.guidelines_chars(), 10);
    }

    #[test]
    fn empty_guidelines_leave_the_instructions_untouched() {
        let l = TeamLimits::default();
        assert!(!l.has_guidelines());
        assert_eq!(l.apply_guidelines("分析 Q3 成本"), "分析 Q3 成本");
        // 只有空白也不算准则，不许拼一个空标题。
        let blank = TeamLimits::new(4, 2, 3, "  \n ").expect("合法");
        assert!(!blank.has_guidelines());
        assert_eq!(blank.apply_guidelines("原样"), "原样");
    }

    #[test]
    fn the_schema_ranges_are_enforced_at_construction_time() {
        // 与 0001_init.sql:308-310 的 CHECK 同口径。
        for bad in [1u32, 9u32] {
            let err = TeamLimits::new(bad, 2, 3, "").expect_err("max_dispatch 超界必须判红");
            assert!(err.to_string().contains("max_dispatch"), "{err}");
            assert!(err.next_step().contains("2~8"), "{err}");
        }
        assert!(TeamLimits::new(4, 6, 3, "").is_err(), "max_replan 上限是 5");
        assert!(
            TeamLimits::new(4, 2, 6, "").is_err(),
            "max_ask_depth 上限是 5"
        );
        assert!(TeamLimits::new(2, 0, 0, "").is_ok(), "边界值本身合法");
        assert!(TeamLimits::new(8, 5, 5, "").is_ok(), "边界值本身合法");
    }

    #[test]
    fn an_over_long_guidelines_is_rejected_instead_of_truncated() {
        let long = "字".repeat(MAX_GUIDELINES_CHARS + 1);
        let err = TeamLimits::new(4, 2, 3, long).expect_err("超长准则必须判红");
        assert!(err.to_string().contains("团队准则太长"), "{err}");
        assert!(err.next_step().contains("2000"), "要说清上限：{err}");
        // 按**字符**计：2000 个汉字的字节数远超 2000，但正好是上限，必须放行。
        let at_cap = "字".repeat(MAX_GUIDELINES_CHARS);
        assert!(TeamLimits::new(4, 2, 3, at_cap).is_ok());
    }

    #[test]
    fn defaults_match_the_schema_defaults() {
        let d = TeamLimits::default();
        assert_eq!(d.max_dispatch(), 4);
        assert_eq!(d.max_replan(), 2);
        assert_eq!(d.max_ask_depth(), 3);
        assert_eq!(d.guidelines(), "");
    }
}
