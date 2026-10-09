use crate::ids::{ExpertId, MemberId, SessionId, UserId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    Unauthorized(String),

    Forbidden(String),

    NotFound(String),

    Conflict(String),

    Provider(String),

    Storage(String),

    Internal(String),
}

impl AdapterError {
    pub fn detail(&self) -> &str {
        match self {
            Self::Unauthorized(s)
            | Self::Forbidden(s)
            | Self::NotFound(s)
            | Self::Conflict(s)
            | Self::Provider(s)
            | Self::Storage(s)
            | Self::Internal(s) => s,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Provider(_))
    }
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = match self {
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not found",
            Self::Conflict(_) => "conflict",
            Self::Provider(_) => "provider error",
            Self::Storage(_) => "storage error",
            Self::Internal(_) => "internal",
        };
        write!(f, "{prefix}: {}", self.detail())
    }
}

impl std::error::Error for AdapterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbortScope {
    StopRound,

    AbortRoom,
}

impl AbortScope {
    pub fn halts_members(&self) -> bool {
        matches!(self, Self::AbortRoom)
    }
}

impl std::fmt::Display for AbortScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StopRound => f.write_str("StopRound"),
            Self::AbortRoom => f.write_str("AbortRoom"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    role: MessageRole,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageRole {
    User,

    Assistant,

    Tool,
}

impl MessageRole {
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}

impl Message {
    pub fn new(role: MessageRole, text: impl Into<String>) -> Result<Self, InvalidMessage> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(InvalidMessage::EmptyText);
        }
        Ok(Self { role, text })
    }

    pub fn user(text: impl Into<String>) -> Result<Self, InvalidMessage> {
        Self::new(MessageRole::User, text)
    }

    pub fn role(&self) -> MessageRole {
        self.role
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidMessage {
    EmptyText,
}

impl std::fmt::Display for InvalidMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyText => f.write_str("消息正文为空或全空白"),
        }
    }
}

impl std::error::Error for InvalidMessage {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainHop {
    node: String,
    task: String,
}

impl ChainHop {
    pub fn new(node: impl Into<String>, task: impl Into<String>) -> Result<Self, InvalidChainHop> {
        let node = node.into();
        let task = task.into();
        if node.trim().is_empty() {
            return Err(InvalidChainHop::EmptyNode);
        }
        if task.trim().is_empty() {
            return Err(InvalidChainHop::EmptyTask);
        }
        Ok(Self { node, task })
    }

    pub fn node(&self) -> &str {
        &self.node
    }

    pub fn task(&self) -> &str {
        &self.task
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidChainHop {
    EmptyNode,

    EmptyTask,
}

impl std::fmt::Display for InvalidChainHop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyNode => f.write_str("委派链节点名为空"),
            Self::EmptyTask => f.write_str("委派链任务号为空"),
        }
    }
}

impl std::error::Error for InvalidChainHop {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStartRequest {
    owner: UserId,
    session: SessionId,
    expert: ExpertId,
    member: MemberId,
    title: String,
    instructions: String,
    chain: Vec<ChainHop>,
    required_capabilities: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidStartRequest {
    EmptyTitle,

    EmptyInstructions,

    MemberExpertMismatch { expert: String, member: String },
}

impl std::fmt::Display for InvalidStartRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyTitle => f.write_str("任务标题为空"),
            Self::EmptyInstructions => f.write_str("任务 instructions 为空：委派任务必须自包含"),
            Self::MemberExpertMismatch { expert, member } => write!(
                f,
                "成员标识 {member:?} 与专家 {expert:?} 不匹配（成员标识须以专家名开头）"
            ),
        }
    }
}

impl std::error::Error for InvalidStartRequest {}

impl MemberStartRequest {
    pub fn new(
        owner: UserId,
        session: SessionId,
        expert: ExpertId,
        member: MemberId,
        title: impl Into<String>,
        instructions: impl Into<String>,
    ) -> Result<Self, InvalidStartRequest> {
        let title = title.into();
        let instructions = instructions.into();
        if title.trim().is_empty() {
            return Err(InvalidStartRequest::EmptyTitle);
        }
        if instructions.trim().is_empty() {
            return Err(InvalidStartRequest::EmptyInstructions);
        }
        if !member
            .as_str()
            .starts_with(&format!("{}-", expert.as_str()))
        {
            return Err(InvalidStartRequest::MemberExpertMismatch {
                expert: expert.as_str().to_string(),
                member: member.as_str().to_string(),
            });
        }
        Ok(Self {
            owner,
            session,
            expert,
            member,
            title,
            instructions,
            chain: Vec::new(),
            required_capabilities: None,
        })
    }

    pub fn with_chain(mut self, hop: ChainHop) -> Self {
        self.chain.push(hop);
        self
    }

    pub fn with_required_capabilities(mut self, caps: Option<Vec<String>>) -> Self {
        self.required_capabilities = caps;
        self
    }

    pub fn owner(&self) -> UserId {
        self.owner
    }

    pub fn session(&self) -> SessionId {
        self.session
    }

    pub fn expert(&self) -> &ExpertId {
        &self.expert
    }

    pub fn member(&self) -> &MemberId {
        &self.member
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn chain(&self) -> &[ChainHop] {
        &self.chain
    }

    pub fn required_capabilities(&self) -> Option<&[String]> {
        self.required_capabilities.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberStatus {
    Done,

    Partial,

    Failed,

    Cancelled,
}

impl MemberStatus {
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_deliverable(&self) -> bool {
        matches!(self, Self::Done | Self::Partial)
    }
}

impl std::fmt::Display for MemberStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberOutcome {
    member: MemberId,
    status: MemberStatus,
    completed_scope: String,
    output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidOutcome {
    EmptyScope,

    MissingOutput,

    BlankOutput,
}

impl std::fmt::Display for InvalidOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyScope => f.write_str("completed_scope 为空"),
            Self::MissingOutput => f.write_str("状态为 done/partial 但产出正文为空：无法汇总"),
            Self::BlankOutput => f.write_str("产出正文全空白"),
        }
    }
}

impl std::error::Error for InvalidOutcome {}

impl MemberOutcome {
    pub fn new(
        member: MemberId,
        status: MemberStatus,
        completed_scope: impl Into<String>,
        output: impl Into<String>,
    ) -> Result<Self, InvalidOutcome> {
        let completed_scope = completed_scope.into();
        let output = output.into();
        if completed_scope.trim().is_empty() {
            return Err(InvalidOutcome::EmptyScope);
        }
        if status.is_deliverable() {
            if output.trim().is_empty() {
                return Err(InvalidOutcome::MissingOutput);
            }
        } else if !output.trim().is_empty() {
            return Err(InvalidOutcome::BlankOutput);
        }
        Ok(Self {
            member,
            status,
            completed_scope,
            output,
        })
    }

    pub fn done(member: MemberId, scope: &str, output: &str) -> Result<Self, InvalidOutcome> {
        Self::new(member, MemberStatus::Done, scope, output)
    }

    pub fn failed(member: MemberId, scope: &str) -> Result<Self, InvalidOutcome> {
        Self::new(member, MemberStatus::Failed, scope, "")
    }

    pub fn member(&self) -> &MemberId {
        &self.member
    }

    pub fn status(&self) -> MemberStatus {
        self.status
    }

    pub fn completed_scope(&self) -> &str {
        &self.completed_scope
    }

    pub fn output(&self) -> &str {
        &self.output
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainCheck {
    Ok { depth: usize },

    Cycle { node: String, first_at: usize },

    TooDeep { depth: usize, max: usize },
}

pub const MAX_CHAIN_DEPTH: usize = 4;

pub fn check_chain(chain: &[ChainHop], node: &str) -> ChainCheck {
    let depth = chain.len() + 1;
    if depth > MAX_CHAIN_DEPTH {
        return ChainCheck::TooDeep {
            depth,
            max: MAX_CHAIN_DEPTH,
        };
    }
    if let Some(first_at) = chain.iter().position(|h| h.node() == node) {
        return ChainCheck::Cycle {
            node: node.to_string(),
            first_at,
        };
    }
    ChainCheck::Ok { depth }
}

pub trait MemberExecutor: Send + Sync + 'static {
    fn start(
        &self,
        req: MemberStartRequest,
    ) -> impl std::future::Future<Output = Result<MemberOutcome, AdapterError>> + Send;

    /// 给**运行中**的成员追加一条指令（goose 的 `Agent::steer` 语义：成员在
    /// 两轮模型调用之间取走它）。
    ///
    /// `owner` 是成员的发起人：成员标识在一轮里唯一，但两个用户各自的
    /// `cost-analyst-1` 是两个不同的东西 —— 没有发起人就分不出该送给谁。
    /// **没有在跑的成员时必须如实报错**：假装成功会让调用方以为指令送到了，
    /// 而它永远不会生效。
    fn steer(
        &self,
        owner: &UserId,
        member: &MemberId,
        m: Message,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send;

    /// 中途取消正在跑的成员。`scope` 决定只停这一位（`StopRound`）还是停
    /// 这一整间房（`AbortRoom`）。同样：没有在跑的成员如实报错。
    fn abort(
        &self,
        owner: &UserId,
        member: &MemberId,
        scope: AbortScope,
    ) -> impl std::future::Future<Output = Result<(), AdapterError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u8) -> UserId {
        UserId::from_bytes([n; 16])
    }

    fn s(n: u8) -> SessionId {
        SessionId::from_bytes([n; 16])
    }

    fn expert(name: &str) -> ExpertId {
        ExpertId::parse(name).expect("测试用专家名应合法")
    }

    fn member(name: &str) -> MemberId {
        MemberId::parse(name).expect("测试用成员标识应合法")
    }

    fn req(expert_name: &str, member_name: &str) -> MemberStartRequest {
        MemberStartRequest::new(
            u(1),
            s(2),
            expert(expert_name),
            member(member_name),
            "分析成本",
            "请分析 Q3 成本结构",
        )
        .expect("测试用启动请求应合法")
    }

    #[test]
    fn adapter_error_display_keeps_variant_prefix_and_detail() {
        let e = AdapterError::NotFound("成员 m-1 不存在".into());
        assert_eq!(e.to_string(), "not found: 成员 m-1 不存在");
        assert_eq!(e.detail(), "成员 m-1 不存在");
    }

    #[test]
    fn adapter_error_variants_have_distinct_wire_prefixes() {
        let errs = [
            AdapterError::Unauthorized("a".into()),
            AdapterError::Forbidden("a".into()),
            AdapterError::NotFound("a".into()),
            AdapterError::Conflict("a".into()),
            AdapterError::Provider("a".into()),
            AdapterError::Storage("a".into()),
            AdapterError::Internal("a".into()),
        ];
        assert_eq!(errs.len(), 7, "契约 §三 定义了 7 个变体");
        let mut seen = std::collections::BTreeSet::new();
        for e in &errs {
            assert!(seen.insert(e.to_string()), "变体 Display 撞车：{e}");
        }
        assert_eq!(seen.len(), 7, "7 个变体必须产出 7 种不同的 Display");
    }

    #[test]
    fn only_provider_error_is_retryable() {
        assert!(AdapterError::Provider("tunnel 503".into()).is_retryable());
        for e in [
            AdapterError::Unauthorized("x".into()),
            AdapterError::Forbidden("x".into()),
            AdapterError::NotFound("x".into()),
            AdapterError::Conflict("x".into()),
            AdapterError::Storage("x".into()),
            AdapterError::Internal("x".into()),
        ] {
            assert!(!e.is_retryable(), "{e} 不该被当成可重试");
        }
    }

    #[test]
    fn abort_scope_only_room_scope_halts_members() {
        assert!(
            !AbortScope::StopRound.halts_members(),
            "契约七.2：停主持人不得连带停成员"
        );
        assert!(
            AbortScope::AbortRoom.halts_members(),
            "PRD §12.5：AbortRoom 必须波及成员"
        );
    }

    #[test]
    fn abort_scope_display_matches_contract_names() {
        assert_eq!(AbortScope::StopRound.to_string(), "StopRound");
        assert_eq!(AbortScope::AbortRoom.to_string(), "AbortRoom");
    }

    #[test]
    fn message_accepts_non_empty_text_and_keeps_role() {
        let m = Message::user("请补充数据源").expect("非空应合法");
        assert_eq!(m.role(), MessageRole::User);
        assert_eq!(m.text(), "请补充数据源");
        assert_eq!(m.role().as_wire(), "user");
    }

    #[test]
    fn message_rejects_empty_and_whitespace_text() {
        for bad in ["", " ", "\t", "\n", "  \r\n  "] {
            assert_eq!(
                Message::user(bad).unwrap_err(),
                InvalidMessage::EmptyText,
                "输入 {bad:?} 应被判为空消息"
            );
        }
    }

    #[test]
    fn chain_hop_accepts_valid_pair_and_exposes_both_fields() {
        let h = ChainHop::new("node-a", "t-001").expect("应合法");
        assert_eq!(h.node(), "node-a");
        assert_eq!(h.task(), "t-001");
    }

    #[test]
    fn chain_hop_rejects_empty_node_or_task() {
        assert_eq!(
            ChainHop::new("", "t-1").unwrap_err(),
            InvalidChainHop::EmptyNode
        );
        assert_eq!(
            ChainHop::new("  ", "t-1").unwrap_err(),
            InvalidChainHop::EmptyNode
        );
        assert_eq!(
            ChainHop::new("node-a", "").unwrap_err(),
            InvalidChainHop::EmptyTask
        );
        assert_eq!(
            ChainHop::new("node-a", "  ").unwrap_err(),
            InvalidChainHop::EmptyTask
        );
    }

    #[test]
    fn start_request_keeps_owner_session_expert_and_member_distinct() {
        let r = req("cost-analyst", "cost-analyst-1");
        assert_eq!(r.owner(), u(1));
        assert_eq!(r.session(), s(2));
        assert_eq!(r.expert().as_str(), "cost-analyst");
        assert_eq!(r.member().as_str(), "cost-analyst-1");
        assert_ne!(
            r.owner().to_compact_hex(),
            r.session().to_compact_hex(),
            "已检查：owner 与 session 携带不同的身份值"
        );
    }

    #[test]
    fn start_request_rejects_member_of_a_different_expert() {
        let err = MemberStartRequest::new(
            u(1),
            s(2),
            expert("cost-analyst"),
            member("growth-analyst-1"),
            "t",
            "i",
        )
        .unwrap_err();
        assert_eq!(
            err,
            InvalidStartRequest::MemberExpertMismatch {
                expert: "cost-analyst".into(),
                member: "growth-analyst-1".into()
            }
        );
    }

    #[test]
    fn start_request_rejects_empty_title_and_instructions() {
        let base = |title: &str, instr: &str| {
            MemberStartRequest::new(
                u(1),
                s(2),
                expert("cost-analyst"),
                member("cost-analyst-1"),
                title,
                instr,
            )
        };
        assert_eq!(base("", "i").unwrap_err(), InvalidStartRequest::EmptyTitle);
        assert_eq!(
            base("   ", "i").unwrap_err(),
            InvalidStartRequest::EmptyTitle
        );
        assert_eq!(
            base("t", "").unwrap_err(),
            InvalidStartRequest::EmptyInstructions
        );
        assert_eq!(
            base("t", "  \n ").unwrap_err(),
            InvalidStartRequest::EmptyInstructions,
            "全空白 instructions 等同无 instructions"
        );
    }

    #[test]
    fn start_request_chain_and_capabilities_are_opt_in() {
        let r = req("cost-analyst", "cost-analyst-1");
        assert!(r.chain().is_empty(), "默认无上游链");
        assert_eq!(
            r.required_capabilities(),
            None,
            "默认不要求能力（v2 降为可选）"
        );

        let r = r
            .with_chain(ChainHop::new("node-a", "t-001").expect("应合法"))
            .with_required_capabilities(Some(vec!["wiki_search".into()]));
        assert_eq!(r.chain().len(), 1, "已检查：链长应为 1");
        assert_eq!(r.chain()[0].node(), "node-a");
        assert_eq!(
            r.required_capabilities(),
            Some(&["wiki_search".to_string()][..])
        );
    }

    #[test]
    fn outcome_done_requires_output() {
        let o = MemberOutcome::done(member("m-1"), "完成分析", "结论：降 12%")
            .expect("done + 有产出应合法");
        assert_eq!(o.status(), MemberStatus::Done);
        assert_eq!(o.output(), "结论：降 12%");
        assert_eq!(o.completed_scope(), "完成分析");
        assert_eq!(o.member().as_str(), "m-1");
    }

    #[test]
    fn outcome_deliverable_status_without_output_is_rejected() {
        for status in [MemberStatus::Done, MemberStatus::Partial] {
            assert_eq!(
                MemberOutcome::new(member("m-1"), status, "s", "").unwrap_err(),
                InvalidOutcome::MissingOutput,
                "状态 {status} 无产出必须判红"
            );
            assert_eq!(
                MemberOutcome::new(member("m-1"), status, "s", "   ").unwrap_err(),
                InvalidOutcome::MissingOutput,
                "全空白产出等同无产出（状态 {status}）"
            );
        }
    }

    #[test]
    fn outcome_non_deliverable_status_with_output_is_rejected() {
        for status in [MemberStatus::Failed, MemberStatus::Cancelled] {
            assert_eq!(
                MemberOutcome::new(member("m-1"), status, "s", "半截内容").unwrap_err(),
                InvalidOutcome::BlankOutput,
                "状态 {status} 携带产出正文必须判红"
            );
        }
    }

    #[test]
    fn outcome_requires_scope_even_when_failed() {
        for bad in ["", "  "] {
            assert_eq!(
                MemberOutcome::new(member("m-1"), MemberStatus::Failed, bad, "").unwrap_err(),
                InvalidOutcome::EmptyScope
            );
        }
        let o = MemberOutcome::failed(member("m-1"), "只完成了成本结构分析").expect("应合法");
        assert_eq!(o.status(), MemberStatus::Failed);
        assert_eq!(o.output(), "");
    }

    #[test]
    fn member_status_wire_names_are_distinct() {
        let all = [
            MemberStatus::Done,
            MemberStatus::Partial,
            MemberStatus::Failed,
            MemberStatus::Cancelled,
        ];
        let wires: std::collections::BTreeSet<&str> = all.iter().map(|s| s.as_wire()).collect();
        assert_eq!(wires.len(), 4, "4 个状态必须对应 4 个线格式名");
        assert!(wires.contains("done"));
        assert!(wires.contains("partial"));
        assert!(wires.contains("failed"));
        assert!(wires.contains("cancelled"));
    }

    #[test]
    fn partial_counts_as_deliverable_done_failed_cancelled_do_not() {
        assert!(MemberStatus::Done.is_deliverable());
        assert!(
            MemberStatus::Partial.is_deliverable(),
            "部分成功也要能汇总（docs/07 §2.3）"
        );
        assert!(!MemberStatus::Failed.is_deliverable());
        assert!(!MemberStatus::Cancelled.is_deliverable());
    }

    #[test]
    fn empty_chain_first_hop_is_ok_with_depth_one() {
        assert_eq!(
            check_chain(&[], "node-a"),
            ChainCheck::Ok { depth: 1 },
            "链起点必须允许"
        );
    }

    #[test]
    fn chain_detects_a_to_b_to_a_cycle() {
        let chain = vec![
            ChainHop::new("node-a", "t-001").expect("应合法"),
            ChainHop::new("node-b", "t-002").expect("应合法"),
        ];
        assert_eq!(
            check_chain(&chain, "node-a"),
            ChainCheck::Cycle {
                node: "node-a".into(),
                first_at: 0
            }
        );
    }

    #[test]
    fn chain_detects_self_delegation() {
        let chain = vec![ChainHop::new("node-a", "t-001").expect("应合法")];
        assert_eq!(
            check_chain(&chain, "node-a"),
            ChainCheck::Cycle {
                node: "node-a".into(),
                first_at: 0
            },
            "A 派 A 同样是环"
        );
    }

    #[test]
    fn chain_without_repeat_is_ok_and_reports_depth() {
        let chain = vec![
            ChainHop::new("node-a", "t-001").expect("应合法"),
            ChainHop::new("node-b", "t-002").expect("应合法"),
        ];
        assert_eq!(check_chain(&chain, "node-c"), ChainCheck::Ok { depth: 3 });
    }

    #[test]
    fn chain_too_deep_is_reported_with_actual_and_limit() {
        let chain: Vec<ChainHop> = (0..4)
            .map(|i| ChainHop::new(format!("node-{i}"), format!("t-{i}")).expect("应合法"))
            .collect();
        assert_eq!(
            check_chain(&chain, "node-4"),
            ChainCheck::TooDeep {
                depth: 5,
                max: MAX_CHAIN_DEPTH
            }
        );

        let chain3: Vec<ChainHop> = (0..3)
            .map(|i| ChainHop::new(format!("node-{i}"), format!("t-{i}")).expect("应合法"))
            .collect();
        assert_eq!(check_chain(&chain3, "node-3"), ChainCheck::Ok { depth: 4 });
    }

    #[test]
    fn chain_check_outcomes_are_mutually_exclusive_across_full_length_sweep() {
        let mut ok = 0;
        let mut cycle = 0;
        let mut deep = 0;
        for len in 0..=6usize {
            let chain: Vec<ChainHop> = (0..len)
                .map(|i| ChainHop::new(format!("node-{i}"), format!("t-{i}")).expect("应合法"))
                .collect();
            match check_chain(&chain, "node-new") {
                ChainCheck::Ok { .. } => ok += 1,
                ChainCheck::Cycle { .. } => cycle += 1,
                ChainCheck::TooDeep { .. } => deep += 1,
            }
        }
        assert_eq!(ok + cycle + deep, 7, "已检查 7 档链长，每档都要有判定");
        assert_eq!(cycle, 0, "node-new 从未出现过，不该报环");
        assert_eq!(deep, 3, "长度 3/4/5 起的链超深（MAX=4）：3 档");
        assert_eq!(ok, 4, "长度 0..=2 通过：4 档");
    }
}
