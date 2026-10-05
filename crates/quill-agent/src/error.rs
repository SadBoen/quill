//! 编排层领域错误：中文人话 + **可直接复制**的修复命令。
//!
//! 依据 `AGENTS.md` 铁律七「失败必须自诊断」：
//! - 不把 `SQLITE_CONSTRAINT` / 变体名抛给用户；
//! - 每条错误都带一条**完整、可直接复制**的命令；
//! - 用户出错时**唯一需要执行**的命令是 `quill doctor`。
//!
//! # 与 `quill-control` 的错误类型是什么关系
//!
//! **刻意不共用一个枚举**：控制面 22 个变体、编排层 18 个变体，两边各有
//! 自己的 `Storage` / `NotFound` 语义。强行合并会造出一个谁都不合适的
//! 巨枚举，而契约（`docs/PHASE2_CONTRACT.md` §三）要求的正是
//! **各 crate 定义自己的领域错误，在 adapters 边界转换**。
//! 两侧都提供 `From<…> for AdapterError` 与 `From<AdapterError> for …`。
//!
//! # 为什么手写 `Display` 而不用 `thiserror`
//!
//! 与 `quill-adapters` / `quill-control` 同一处理：`thiserror` 不在本 crate 的
//! 依赖表里，新增依赖须主理人裁决。少写 7 行 derive 不值得引入一条依赖边。

use std::fmt;

use quill_adapters::{AdapterError, ExpertId, MemberId};
use quill_domain::TeamError;

/// 全项目统一的诊断命令（`AGENTS.md` 铁律七：唯一需要执行的命令）。
pub const DOCTOR_CMD: &str = "quill doctor";

/// 编排层领域错误。
///
/// 变体按「用户能据此做什么」划分，每个变体都必须能回答「下一步执行哪条命令」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentError {
    /// 专家不存在，**或**对当前用户不可见。
    ///
    /// ⚠️ **刻意不区分「不存在」与「无权看」**：区分开就成了跨用户探测接口
    /// （A 用户能靠报错差异枚举 B 用户的私有专家）。与
    /// `quill-control` 的 `CredentialsRejected` 合并用户名/密码是同一个理由。
    ExpertNotFound {
        /// 专家标识。
        id: ExpertId,
    },
    /// 同一属主下专家标识已存在（复合主键 `(owner_user_id, id)` 冲突）。
    ExpertExists {
        /// 专家标识。
        id: ExpertId,
    },
    /// 专家显示名不合法。
    ExpertDisplayNameInvalid {
        /// 用户原始输入（原样回显）。
        raw: String,
        /// 中文原因。
        reason: &'static str,
    },
    /// 内置专家受保护：非系统属主不可改、不可删。
    ExpertBuiltinProtected {
        /// 专家标识。
        id: ExpertId,
    },
    /// 当前用户不是该专家的属主（仅创建者可改，`PHASE2_CONTRACT` §5.1）。
    ExpertNotModifiable {
        /// 专家标识。
        id: ExpertId,
    },
    /// 专家已被软删除。
    ExpertDeleted {
        /// 专家标识。
        id: ExpertId,
    },
    /// 委派链成环（`docs/07` §4.2.1 `chain_too_deep` 的环分支）。
    ChainCycle {
        /// 重复出现的节点名。
        node: String,
        /// 该节点在链中首次出现的位置（0 起）。
        first_at: usize,
    },
    /// 委派链超长。
    ChainTooDeep {
        /// 实际长度。
        depth: usize,
        /// 上限。
        max: usize,
    },
    /// 派工状态机非法跃迁（如对已结算的派工再结算一次）。
    DispatchIllegalTransition {
        /// 中文描述（含 from → to 与派工键）。
        detail: String,
    },
    /// 成员执行被拒绝 / 失败 / 中断（`AdapterError` 已在本层转成中文）。
    MemberRejected {
        /// 成员标识。
        member: MemberId,
        /// 失败类别（**错误码由它派生**，见 [`AgentError::code`]）。
        kind: MemberRejectKind,
        /// 中文描述。
        detail: String,
        /// 是否值得重试（口径来自 `AdapterError::is_retryable`）。
        ///
        /// ⚠️ 这只是**建议**：`docs/07` §4.3 规定派工失败**不自动重跑**
        /// （副作用不可逆），所以编排层只如实上报，绝不自行重试。
        retryable: bool,
    },
    /// 团队聚合的不变量被破坏（由 `quill-domain::TeamError` 转来）。
    TeamInvalid(TeamError),
    /// 派工参数非法（构造请求阶段判红，未触达执行器）。
    DispatchRequestInvalid {
        /// 中文原因。
        reason: String,
    },
    /// 存储层失败。
    Storage {
        /// 中文描述（**不得**含凭据原文）。
        detail: String,
    },
    /// 内部不变量被破坏（代码 bug，不是用户能修的）。
    InvariantBroken {
        /// 中文描述。
        detail: String,
    },
}

/// 成员失败的类别。
///
/// # 为什么用枚举而不是在 `MemberRejected` 里存一个 `code: &'static str`
///
/// 存字符串会造出**两份真相源**：变体自己的 `code` 字段一份，
/// `AgentError::code()` 一份。两者一旦漂移，`quill-server` 的 HTTP 映射
/// 与 UI 的 i18n key 就会指向不同根因 —— 而**测试很难发现**，
/// 因为两个函数各自都能通过单测。
/// 枚举让「类别」成为唯一可写的地方，`code()` 只做**派生**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemberRejectKind {
    /// 对端拒收 / 不可达 / 中断链等传输或策略类失败。
    Rejected,
    /// 对端撤销了我方 key（鉴权失败）。
    Unauthorized,
    /// 成员自报 `FAILED`（它自己知道做不到哪一步）。
    SelfReportedFailure,
    /// 成员自报 `CANCELLED`（被 `AbortRoom` 取消）。
    Cancelled,
}

impl MemberRejectKind {
    /// 稳定错误码（供 UI / 日志聚合，**不能**去匹配中文文案）。
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
    /// 稳定错误码：机器可读，供 `quill-server` 映射 HTTP 状态码与前端 i18n key。
    ///
    /// ⚠️ 判红项：**码不能重复**。两个变体共用一个码，
    /// 前端就无法区分根因（`tests::error_codes_are_unique` 固化为断言）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::ExpertNotFound { .. } => "expert_not_found",
            Self::ExpertExists { .. } => "expert_exists",
            Self::ExpertDisplayNameInvalid { .. } => "expert_display_name_invalid",
            Self::ExpertBuiltinProtected { .. } => "expert_builtin_protected",
            Self::ExpertNotModifiable { .. } => "expert_not_modifiable",
            Self::ExpertDeleted { .. } => "expert_deleted",
            Self::ChainCycle { .. } => "chain_cycle",
            Self::ChainTooDeep { .. } => "chain_too_deep",
            Self::DispatchIllegalTransition { .. } => "dispatch_illegal_transition",
            // ⚠️ 派生于 `kind`，不另存一份字符串（见 MemberRejectKind 的说明）。
            Self::MemberRejected { kind, .. } => kind.as_wire(),
            Self::TeamInvalid(_) => "team_invalid",
            Self::DispatchRequestInvalid { .. } => "dispatch_request_invalid",
            Self::Storage { .. } => "storage_error",
            Self::InvariantBroken { .. } => "invariant_broken",
        }
    }

    /// 该错误**唯一**需要的用户动作命令（可直接整行复制）。
    ///
    /// 判据（`tests::every_error_carries_a_copyable_command` 固化为断言）：
    /// ① 单行；② 以 `quill ` 开头；③ 含 `doctor`；④ 无尾随空格。
    pub fn fix_command(&self) -> String {
        match self {
            Self::ExpertNotFound { .. }
            | Self::ExpertExists { .. }
            | Self::ExpertDisplayNameInvalid { .. }
            | Self::ExpertBuiltinProtected { .. }
            | Self::ExpertNotModifiable { .. }
            | Self::ExpertDeleted { .. } => format!("{DOCTOR_CMD} --section=experts"),
            Self::ChainCycle { .. } | Self::ChainTooDeep { .. } => {
                format!("{DOCTOR_CMD} --section=delegation")
            }
            Self::DispatchIllegalTransition { .. }
            | Self::DispatchRequestInvalid { .. }
            | Self::MemberRejected { .. } => format!("{DOCTOR_CMD} --section=dispatch"),
            Self::TeamInvalid(_) => format!("{DOCTOR_CMD} --section=teams"),
            Self::Storage { .. } | Self::InvariantBroken { .. } => {
                format!("{DOCTOR_CMD} --section=db")
            }
        }
    }

    /// 是否属于**可重试**的失败。
    ///
    /// ⚠️ 这是**建议**而不是动作：`docs/07` §4.3 与 `PHASE2_CONTRACT` §七.4
    /// 都规定「派工失败不自动重跑」（副作用不可逆）。编排层只把它暴露给
    /// 上层决策与 UI 提示，绝不据此自行重试。
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::MemberRejected { retryable, .. } => *retryable,
            _ => false,
        }
    }
}

/// 统一的尾部提示：告诉用户「复制这一行就行」。
///
/// ⚠️ **必须带上 `e.fix_command()` 的完整形态（含 `--section=`）**，
/// 而不只是裸的 `quill doctor`：
/// 两处形态不一致 = 用户照抄文案里的命令与程序建议的命令不同，
/// 那是两份真相源（`tests::every_error_display_points_at_the_same_command` 判红）。
fn tail(f: &mut fmt::Formatter<'_>, e: &AgentError) -> fmt::Result {
    write!(
        f,
        "\n→ 下一步：复制执行 `{}`（它会打印本错误的完整诊断与修复动作）",
        e.fix_command()
    )
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // ⚠️ 先借出 `self` 供 `tail()` 取 `fix_command()`：
        // 匹配臂里用 `Self::…` 解构不会移动（`self` 是 `&Self`），
        // 所以 `self` 在整个 match 期间仍然可用。
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

/// `AdapterError` → `AgentError`，**由调用方补上成员标识**。
///
/// # 为什么不是 `impl From<AdapterError>`
///
/// `AdapterError` 的载荷里**没有成员标识**（契约层 7 个变体都不带），
/// 而 `MemberRejected` 必须能定位到「是哪个成员」。
/// 若用 `From` 就得编一个 `"unknown-member"` 占位值 —— 那会让排障时的
/// 错误信息指向一个不存在的成员，比没有成员名更糟。
/// 因此这里要求调用方（唯一知道成员的派工层）显式传进来。
///
/// ⚠️ **映射口径按语义分，不按变体一对一**：`AdapterError` 是契约层的 7 个变体，
/// 编排层需要区分「成员被拒」与「存储坏了」——两者在
/// `AdapterError` 里分别是 `Forbidden` 与 `Storage`，但在本层必须走不同变体，
/// 否则用户会拿到「成员被拒」这种驴唇不对马嘴的提示。
impl AgentError {
    /// 把成员执行错误转成本层错误。
    pub fn from_member_error(member: &MemberId, e: AdapterError) -> Self {
        let retryable = e.is_retryable();
        let detail = e.detail().to_string();
        match e {
            AdapterError::Storage(d) => Self::Storage { detail: d },
            AdapterError::Internal(d) => Self::InvariantBroken { detail: d },
            // ⚠️ 鉴权失败单独给码：它与「对端拒收」的处置完全不同
            //（前者要去查凭据，后者重试无意义）。
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

/// 委派链检查结果 → 编排层错误。
///
/// `ChainCheck::Ok` **不是错误**，故返回 `Option`：
/// 返回 `None` 表示「无环且未超深，可继续派工」。
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

/// `AgentError` → `AdapterError`（契约层边界转换，`docs/PHASE2_CONTRACT.md` §三）。
///
/// ⚠️ 只带 `code()` 过去，**不带中文文案**：中文属于人看的层，
/// 塞进机器枚举的载荷里会让日志无法按根因聚合。
impl From<AgentError> for AdapterError {
    fn from(e: AgentError) -> Self {
        // ⚠️ 匹配臂上的 `AgentError::` 必须**写全**：这里是
        // `From<AgentError> for AdapterError`，`Self` == `AdapterError`，
        // 裸 `Self::ExpertNotFound` 会被解析到契约层的 7 个变体上（不存在）→ 编译红。
        let code = e.code();
        match e {
            AgentError::ExpertNotFound { .. } | AgentError::ExpertDeleted { .. } => {
                AdapterError::NotFound(code.to_string())
            }
            AgentError::ExpertExists { .. }
            | AgentError::ExpertDisplayNameInvalid { .. }
            | AgentError::ChainCycle { .. }
            | AgentError::ChainTooDeep { .. }
            | AgentError::DispatchIllegalTransition { .. }
            | AgentError::DispatchRequestInvalid { .. }
            | AgentError::TeamInvalid(_) => AdapterError::Conflict(code.to_string()),
            AgentError::ExpertBuiltinProtected { .. } | AgentError::ExpertNotModifiable { .. } => {
                AdapterError::Forbidden(code.to_string())
            }
            // ⚠️ 成员执行失败是**对端/传输**问题，映射成 Provider：
            // 它是本层唯一「重试与否由降级链决定」的一类（docs/07 §4.3）。
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

    /// 构造**每一个**变体的一个样本。
    ///
    /// 新增变体时忘记写 `code()` / 中文文案 / 命令，本测试立刻红。
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
        assert_eq!(all.len(), 14, "变体数变了，请同步本测试的样本清单");
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
        // 铁律七的「唯一命令」口径：文案里给出的命令必须与 fix_command() 一致，
        // 否则用户照抄文案里的命令会与程序建议的不一致（两处真相源）。
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
        // ⚠️ 码重复 = 前端分不清根因。这条判红是可执行的，不靠人记得。
        let mut seen = std::collections::BTreeSet::new();
        for e in one_of_each() {
            assert!(seen.insert(e.code()), "错误码重复：{}", e.code());
        }
        assert_eq!(seen.len(), 14, "已检查 14 个变体，14 个码必须互不相同");
    }

    #[test]
    fn display_is_chinese_and_carries_the_offending_identity() {
        // 铁律七：错误必须能定位到「是谁」，否则用户无法自诊断。
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
        // 判据来自 `AdapterError::is_retryable`（契约层口径），不是编排层自己拍的。
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
        // 🔴 反向用例：若这里编一个占位成员名，排障时会指向不存在的成员。
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
        // 🔴 反向用例：把存储错误说成「成员被拒」，用户会去查错方向。
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
        // 鉴权失败与「对端拒收」处置完全不同：前者查凭据，后者重试无意义。
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
        // 判据：`code()` 派生于 `kind`，所以**每个 kind 必须有唯一线名**，
        // 否则两个不同根因会共用一个码。
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
        // 🔴 环防护三态必须各自可表达；Ok 返回 None 而不是 Err
        // ——「无环」不是错误。
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
        // 契约层载荷只带码：中文会毁掉日志按根因聚合的能力。
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
