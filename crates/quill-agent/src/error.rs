use std::fmt;

use quill_adapters::{AdapterError, ExpertId, MemberId};
use quill_domain::TeamError;

pub const DOCTOR_CMD: &str = "quill doctor";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentError {
    ExpertNotFound {
        id: ExpertId,
    },

    ExpertExists {
        id: ExpertId,
    },

    ExpertDisplayNameInvalid {
        raw: String,

        reason: &'static str,
    },

    /// 人格正文（对应 goose custom agent 的 markdown 正文）不合法。
    ExpertInstructionsInvalid {
        raw: String,

        reason: &'static str,
    },

    /// 偏好模型名不合法。`None` 合法（= 跟随实例默认模型），`Some(空串)` 不合法。
    ExpertModelInvalid {
        raw: String,

        reason: &'static str,
    },

    /// 来源模板 id 不合法。`None` 合法（= 不来自模板），
    /// `Some(不符合 ^[a-z0-9-]{1,64}$)` 不合法。
    ExpertSourceTemplateInvalid {
        raw: String,

        reason: &'static str,
    },

    ExpertBuiltinProtected {
        id: ExpertId,
    },

    ExpertNotModifiable {
        id: ExpertId,
    },

    ExpertDeleted {
        id: ExpertId,
    },

    ChainCycle {
        node: String,

        first_at: usize,
    },

    ChainTooDeep {
        depth: usize,

        max: usize,
    },

    DispatchIllegalTransition {
        detail: String,
    },

    MemberRejected {
        member: MemberId,

        kind: MemberRejectKind,

        detail: String,

        retryable: bool,
    },

    TeamInvalid(TeamError),

    /// 超出团队限制列（`teams.max_dispatch` / `max_replan` / `max_ask_depth`）。
    ///
    /// **不是内部故障**：调用方把本轮减量、或把该列调大就能过，所以 HTTP 层
    /// 应把它映射成 400 而不是 500（见 `crates/quill-server/src/api_dispatch.rs`）。
    TeamLimitsExceeded {
        /// 哪一条限制（中文说明，直接进文案）。
        limit: &'static str,

        /// 本次的数量（成员数 / 轮次 / 提问深度）。
        got: u32,

        /// 该列允许的上限。
        max: u32,

        /// 中文的下一步 —— 由提出限制的那一处给出（这里不猜）。
        next_step: String,
    },

    DispatchRequestInvalid {
        reason: String,
    },

    Storage {
        detail: String,
    },

    InvariantBroken {
        detail: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemberRejectKind {
    Rejected,

    Unauthorized,

    SelfReportedFailure,

    Cancelled,
}

impl MemberRejectKind {
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Rejected => "member_rejected",
            Self::Unauthorized => "member_unauthorized",
            Self::SelfReportedFailure => "member_reported_failure",
            Self::Cancelled => "member_cancelled",
        }
    }
}

impl AgentError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ExpertNotFound { .. } => "expert_not_found",
            Self::ExpertExists { .. } => "expert_exists",
            Self::ExpertDisplayNameInvalid { .. } => "expert_display_name_invalid",
            Self::ExpertInstructionsInvalid { .. } => "expert_instructions_invalid",
            Self::ExpertModelInvalid { .. } => "expert_model_invalid",
            Self::ExpertSourceTemplateInvalid { .. } => "expert_source_template_invalid",
            Self::ExpertBuiltinProtected { .. } => "expert_builtin_protected",
            Self::ExpertNotModifiable { .. } => "expert_not_modifiable",
            Self::ExpertDeleted { .. } => "expert_deleted",
            Self::ChainCycle { .. } => "chain_cycle",
            Self::ChainTooDeep { .. } => "chain_too_deep",
            Self::DispatchIllegalTransition { .. } => "dispatch_illegal_transition",

            Self::MemberRejected { kind, .. } => kind.as_wire(),
            Self::TeamInvalid(_) => "team_invalid",
            Self::TeamLimitsExceeded { .. } => "team_limits_exceeded",
            Self::DispatchRequestInvalid { .. } => "dispatch_request_invalid",
            Self::Storage { .. } => "storage_error",
            Self::InvariantBroken { .. } => "invariant_broken",
        }
    }

    pub fn fix_command(&self) -> String {
        match self {
            Self::ExpertNotFound { .. }
            | Self::ExpertExists { .. }
            | Self::ExpertDisplayNameInvalid { .. }
            | Self::ExpertInstructionsInvalid { .. }
            | Self::ExpertModelInvalid { .. }
            | Self::ExpertSourceTemplateInvalid { .. }
            | Self::ExpertBuiltinProtected { .. }
            | Self::ExpertNotModifiable { .. }
            | Self::ExpertDeleted { .. } => format!("{DOCTOR_CMD} --section=experts"),
            Self::ChainCycle { .. } | Self::ChainTooDeep { .. } => {
                format!("{DOCTOR_CMD} --section=delegation")
            }
            Self::DispatchIllegalTransition { .. }
            | Self::DispatchRequestInvalid { .. }
            | Self::MemberRejected { .. } => format!("{DOCTOR_CMD} --section=dispatch"),
            Self::TeamInvalid(_) | Self::TeamLimitsExceeded { .. } => {
                format!("{DOCTOR_CMD} --section=teams")
            }
            Self::Storage { .. } | Self::InvariantBroken { .. } => {
                format!("{DOCTOR_CMD} --section=db")
            }
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            Self::MemberRejected { retryable, .. } => *retryable,
            _ => false,
        }
    }
}

fn tail(f: &mut fmt::Formatter<'_>, e: &AgentError) -> fmt::Result {
    write!(
        f,
        "\n→ 下一步：复制执行 `{}`（它会打印本错误的完整诊断与修复动作）",
        e.fix_command()
    )
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpertNotFound { id } => {
                write!(f, "专家「{id}」不存在，或你没有查看它的权限")?;
                tail(f, self)
            }
            Self::ExpertExists { id } => {
                write!(f, "专家「{id}」已存在。换个标识，或先删除旧的")?;
                tail(f, self)
            }
            Self::ExpertDisplayNameInvalid { raw, reason } => {
                write!(f, "专家显示名不合法：{reason}。你填的是「{raw}」")?;
                tail(f, self)
            }
            Self::ExpertInstructionsInvalid { raw, reason } => {
                write!(
                    f,
                    "专家人格正文不合法：{reason}。你填了 {} 个字符",
                    raw.chars().count()
                )?;
                tail(f, self)
            }
            Self::ExpertModelInvalid { raw, reason } => {
                write!(f, "专家偏好模型名不合法：{reason}。你填的是「{raw}」")?;
                tail(f, self)
            }
            Self::ExpertSourceTemplateInvalid { raw, reason } => {
                write!(f, "专家来源模板不合法：{reason}。你填的是「{raw}」")?;
                tail(f, self)
            }
            Self::ExpertBuiltinProtected { id } => {
                write!(
                    f,
                    "专家「{id}」是内置系统专家，不能修改或删除。内置专家随版本发布，如需调整请提 issue"
                )?;
                tail(f, self)
            }
            Self::ExpertNotModifiable { id } => {
                write!(f, "你不是专家「{id}」的创建者，无法修改或删除它")?;
                tail(f, self)
            }
            Self::ExpertDeleted { id } => {
                write!(f, "专家「{id}」已被删除，无法继续操作")?;
                tail(f, self)
            }
            Self::ChainCycle { node, first_at } => {
                write!(
                    f,
                    "委派链成环：节点「{node}」在链上第 {first_at} 跳已出现过，再派一次会无限循环。请换一条委派路径"
                )?;
                tail(f, self)
            }
            Self::ChainTooDeep { depth, max } => {
                write!(
                    f,
                    "委派链太长：实际 {depth} 跳，上限 {max} 跳。请把任务拆成更小的子任务再派"
                )?;
                tail(f, self)
            }
            Self::DispatchIllegalTransition { detail } => {
                write!(
                    f,
                    "派工状态机被破坏：{detail}。这不是重试能解决的，请把本行连同 syslog 一起反馈"
                )?;
                tail(f, self)
            }
            Self::MemberRejected {
                member,
                detail,
                retryable,
                ..
            } => {
                write!(f, "成员「{member}」未能交付：{detail}")?;
                if *retryable {
                    write!(
                        f,
                        "。这是传输层故障，理论上可重试，但系统**不会自动重跑**（执行副作用不可逆），请手动重开一轮"
                    )?;
                }
                tail(f, self)
            }
            Self::TeamInvalid(e) => {
                write!(f, "专家团配置不合法：{e}")?;
                tail(f, self)
            }
            Self::TeamLimitsExceeded {
                limit,
                got,
                max,
                next_step,
            } => {
                write!(
                    f,
                    "超出团队限制「{limit}」：本次 {got}，上限 {max}。{next_step}"
                )?;
                tail(f, self)
            }
            Self::DispatchRequestInvalid { reason } => {
                write!(f, "派工参数不合法：{reason}")?;
                tail(f, self)
            }
            Self::Storage { detail } => {
                write!(f, "数据库操作失败：{detail}")?;
                tail(f, self)
            }
            Self::InvariantBroken { detail } => {
                write!(
                    f,
                    "内部不变量被破坏：{detail}。这不是你能靠重试解决的，请把本行连同 syslog 一起反馈"
                )?;
                tail(f, self)
            }
        }
    }
}

impl std::error::Error for AgentError {}

impl AgentError {
    pub fn from_member_error(member: &MemberId, e: AdapterError) -> Self {
        let retryable = e.is_retryable();
        let detail = e.detail().to_string();
        match e {
            AdapterError::Storage(d) => Self::Storage { detail: d },
            AdapterError::Internal(d) => Self::InvariantBroken { detail: d },

            AdapterError::Unauthorized(_) => Self::MemberRejected {
                member: member.clone(),
                kind: MemberRejectKind::Unauthorized,
                detail,
                retryable,
            },
            AdapterError::Forbidden(_)
            | AdapterError::NotFound(_)
            | AdapterError::Conflict(_)
            | AdapterError::Provider(_) => Self::MemberRejected {
                member: member.clone(),
                kind: MemberRejectKind::Rejected,
                detail,
                retryable,
            },
        }
    }
}

pub fn chain_check_error(check: quill_adapters::ChainCheck) -> Option<AgentError> {
    match check {
        quill_adapters::ChainCheck::Ok { .. } => None,
        quill_adapters::ChainCheck::Cycle { node, first_at } => {
            Some(AgentError::ChainCycle { node, first_at })
        }
        quill_adapters::ChainCheck::TooDeep { depth, max } => {
            Some(AgentError::ChainTooDeep { depth, max })
        }
    }
}

impl From<AgentError> for AdapterError {
    fn from(e: AgentError) -> Self {
        let code = e.code();
        match e {
            AgentError::ExpertNotFound { .. } | AgentError::ExpertDeleted { .. } => {
                AdapterError::NotFound(code.to_string())
            }
            AgentError::ExpertExists { .. }
            | AgentError::ExpertDisplayNameInvalid { .. }
            | AgentError::ExpertInstructionsInvalid { .. }
            | AgentError::ExpertModelInvalid { .. }
            | AgentError::ExpertSourceTemplateInvalid { .. }
            | AgentError::ChainCycle { .. }
            | AgentError::ChainTooDeep { .. }
            | AgentError::DispatchIllegalTransition { .. }
            | AgentError::DispatchRequestInvalid { .. }
            | AgentError::TeamInvalid(_)
            | AgentError::TeamLimitsExceeded { .. } => AdapterError::Conflict(code.to_string()),
            AgentError::ExpertBuiltinProtected { .. } | AgentError::ExpertNotModifiable { .. } => {
                AdapterError::Forbidden(code.to_string())
            }

            AgentError::MemberRejected { .. } => AdapterError::Provider(code.to_string()),
            AgentError::Storage { .. } => AdapterError::Storage(code.to_string()),
            AgentError::InvariantBroken { .. } => AdapterError::Internal(code.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_adapters::ChainCheck;

    fn expert(name: &str) -> ExpertId {
        ExpertId::parse(name).expect("测试用专家名应合法")
    }

    fn member(name: &str) -> MemberId {
        MemberId::parse(name).expect("测试用成员标识应合法")
    }

    fn one_of_each() -> Vec<AgentError> {
        vec![
            AgentError::ExpertNotFound {
                id: expert("cost-analyst"),
            },
            AgentError::ExpertExists {
                id: expert("cost-analyst"),
            },
            AgentError::ExpertDisplayNameInvalid {
                raw: String::new(),
                reason: "不能为空",
            },
            AgentError::ExpertInstructionsInvalid {
                raw: "x".repeat(20_001),
                reason: "超过 20000 个字符",
            },
            AgentError::ExpertModelInvalid {
                raw: "   ".into(),
                reason: "trim 后为空",
            },
            AgentError::ExpertSourceTemplateInvalid {
                raw: "AI Coding".into(),
                reason: "含非法字符",
            },
            AgentError::ExpertBuiltinProtected {
                id: expert("builtin-helper"),
            },
            AgentError::ExpertNotModifiable {
                id: expert("cost-analyst"),
            },
            AgentError::ExpertDeleted {
                id: expert("cost-analyst"),
            },
            AgentError::ChainCycle {
                node: "node-a".into(),
                first_at: 0,
            },
            AgentError::ChainTooDeep { depth: 5, max: 4 },
            AgentError::DispatchIllegalTransition {
                detail: "DONE → RUNNING".into(),
            },
            AgentError::MemberRejected {
                member: member("cost-analyst-1"),
                kind: MemberRejectKind::Rejected,
                detail: "对端拒绝接收".into(),
                retryable: false,
            },
            AgentError::TeamInvalid(TeamError::EmptyName),
            AgentError::TeamLimitsExceeded {
                limit: "max_dispatch（一轮最多派几个成员）",
                got: 5,
                max: 4,
                next_step: "把成员减到 4 个以内再重试。".into(),
            },
            AgentError::DispatchRequestInvalid {
                reason: "标题为空".into(),
            },
            AgentError::Storage {
                detail: "唯一约束冲突 ux_dispatch_once".into(),
            },
            AgentError::InvariantBroken {
                detail: "派工行缺 user_id".into(),
            },
        ]
    }

    #[test]
    fn every_error_carries_a_copyable_command() {
        let all = one_of_each();
        assert_eq!(all.len(), 18, "变体数变了，请同步本测试的样本清单");
        for e in &all {
            let cmd = e.fix_command();
            assert!(!cmd.contains('\n'), "[{}] 命令不是单行：{cmd}", e.code());
            assert!(
                cmd.starts_with("quill "),
                "[{}] 命令不以 quill 开头：{cmd}",
                e.code()
            );
            assert!(
                cmd.contains("doctor"),
                "[{}] 命令未指向 doctor：{cmd}",
                e.code()
            );
            assert!(
                !cmd.ends_with(' '),
                "[{}] 命令尾部有多余空格：{cmd}",
                e.code()
            );
        }
    }

    #[test]
    fn every_error_display_points_at_the_same_command() {
        for e in one_of_each() {
            let msg = e.to_string();
            assert!(
                msg.contains(&e.fix_command()),
                "[{}] 文案没给出 fix_command() 的那条命令：\n{msg}",
                e.code()
            );
        }
    }

    #[test]
    fn error_codes_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for e in one_of_each() {
            assert!(seen.insert(e.code()), "错误码重复：{}", e.code());
        }
        assert_eq!(seen.len(), 18, "已检查 18 个变体，18 个码必须互不相同");
    }

    #[test]
    fn display_is_chinese_and_carries_the_offending_identity() {
        let msg = AgentError::ChainCycle {
            node: "node-a".into(),
            first_at: 2,
        }
        .to_string();
        assert!(msg.contains("node-a"), "须含节点名：{msg}");
        assert!(msg.contains('2'), "须含环出现位置：{msg}");

        let msg = AgentError::MemberRejected {
            member: member("cost-analyst-1"),
            kind: MemberRejectKind::Rejected,
            detail: "对端拒绝接收".into(),
            retryable: true,
        }
        .to_string();
        assert!(msg.contains("cost-analyst-1"), "须含成员标识：{msg}");
        assert!(
            msg.contains("不会自动重跑"),
            "可重试错误必须说明「不会自动重跑」，否则用户以为系统会自己恢复：{msg}"
        );
    }

    #[test]
    fn only_member_rejected_carries_retryable_and_it_comes_from_the_adapter() {
        let m = member("cost-analyst-1");
        let timeout = AgentError::from_member_error(&m, AdapterError::Provider("成员超时".into()));
        assert!(timeout.is_retryable(), "Provider 类应可重试：{timeout}");

        let rejected =
            AgentError::from_member_error(&m, AdapterError::Forbidden("对端拒收".into()));
        assert!(!rejected.is_retryable(), "对端拒收重试无意义：{rejected}");

        for e in one_of_each() {
            if !matches!(
                e,
                AgentError::MemberRejected {
                    retryable: true,
                    ..
                }
            ) {
                assert!(!e.is_retryable(), "[{}] 不该被标成可重试", e.code());
            }
        }
    }

    #[test]
    fn member_error_keeps_the_caller_supplied_member_not_a_placeholder() {
        let m = member("risk-reviewer-7");
        for e in [
            AdapterError::Provider("x".into()),
            AdapterError::Forbidden("x".into()),
            AdapterError::Unauthorized("x".into()),
            AdapterError::NotFound("x".into()),
            AdapterError::Conflict("x".into()),
        ] {
            let got = AgentError::from_member_error(&m, e);
            match got {
                AgentError::MemberRejected { member, .. } => assert_eq!(
                    member.as_str(),
                    "risk-reviewer-7",
                    "必须保留调用方给的真实成员标识"
                ),
                other => panic!("应为 MemberRejected，实际 {other:?}"),
            }
        }
    }

    #[test]
    fn adapter_storage_and_internal_do_not_become_member_rejection() {
        let m = member("cost-analyst-1");
        let e = AgentError::from_member_error(
            &m,
            AdapterError::Storage("ux_dispatch_once 冲突".into()),
        );
        assert!(
            matches!(e, AgentError::Storage { .. }),
            "存储错误必须留在存储类：{e:?}"
        );

        let e = AgentError::from_member_error(&m, AdapterError::Internal("不变量被破坏".into()));
        assert!(
            matches!(e, AgentError::InvariantBroken { .. }),
            "内部错误必须留在内部类：{e:?}"
        );
    }

    #[test]
    fn adapter_unauthorized_gets_its_own_code() {
        let m = member("cost-analyst-1");
        let e = AgentError::from_member_error(&m, AdapterError::Unauthorized("key 被撤销".into()));
        match e {
            AgentError::MemberRejected { kind, .. } => {
                assert_eq!(kind, MemberRejectKind::Unauthorized);
                assert_eq!(kind.as_wire(), "member_unauthorized");
            }
            other => panic!("应为 MemberRejected，实际 {other:?}"),
        }
    }

    #[test]
    fn member_reject_kind_wire_names_are_distinct() {
        let all = [
            MemberRejectKind::Rejected,
            MemberRejectKind::Unauthorized,
            MemberRejectKind::SelfReportedFailure,
            MemberRejectKind::Cancelled,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for k in all {
            assert!(seen.insert(k.as_wire()), "线名重复：{}", k.as_wire());
        }
        assert_eq!(seen.len(), 4, "已检查 4 个 kind，4 个线名必须互不相同");
    }

    #[test]
    fn chain_check_conversion_returns_none_for_ok_and_maps_the_other_two() {
        assert_eq!(chain_check_error(ChainCheck::Ok { depth: 1 }), None);
        assert_eq!(chain_check_error(ChainCheck::Ok { depth: 4 }), None);

        let e = chain_check_error(ChainCheck::Cycle {
            node: "node-a".into(),
            first_at: 1,
        })
        .expect("环必须转成错误");
        assert_eq!(e.code(), "chain_cycle");

        let e =
            chain_check_error(ChainCheck::TooDeep { depth: 5, max: 4 }).expect("超深必须转成错误");
        assert_eq!(e.code(), "chain_too_deep");
    }

    #[test]
    fn agent_error_to_adapter_error_keeps_code_and_drops_chinese() {
        for e in one_of_each() {
            let a = AdapterError::from(e.clone());
            let payload = a.detail().to_string();
            assert_eq!(
                payload,
                e.code(),
                "[{}] 转换后载荷必须是错误码本身",
                e.code()
            );
            assert!(
                !payload.contains("下一步"),
                "[{}] 载荷里混进了人看的文案",
                e.code()
            );
        }
    }

    #[test]
    fn team_error_conversion_keeps_the_domain_message() {
        let e = AgentError::TeamInvalid(TeamError::DuplicateMember(expert("cost-analyst")));
        let msg = e.to_string();
        assert!(msg.contains("cost-analyst"), "须含专家名：{msg}");
        assert_eq!(e.code(), "team_invalid");
    }
}
