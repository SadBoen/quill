//! 领域实体：`Team`（专家团）
//!
//! # 这个聚合存在的理由（不是"占位一下"）
//!
//! `docs/03_智能体编排设计.md` §2.12.2 的第 4 条论据：
//! 上游 `run_subagent_task` 只有一次性 `session_id`，
//! 而本项目要求「成员是**可同时加入多个团队的一等公民专家**」
//! （Octop 规则 1）。这条要求只有在 `Team` 持有 **`ExpertId`**（可复用身份）
//! 而不是 `MemberId`（一次性实例）时才表达得出来。
//!
//! 因此本模块的不变量是**可测的**，不是注释：
//! - 同一 `ExpertId` 可出现在**多个** `Team` 的成员集合里；
//! - 同一 `Team` 内同一 `ExpertId` 不得重复；
//! - 成员数有上限（背压前置，`docs/03` §2.8）。

use std::collections::BTreeSet;

use crate::{ExpertId, TeamId};

/// 单个团队成员数上限。
///
/// ⚠️ 取 8 与 `docs/03` §2.8 的 `GlobalSemaphore(8)` 对齐：
/// 一个团队的成员数不可能超过全局并发上限，超过即意味着
/// **这个团队永远有成员在排队** —— 上限即"可排他地服务"的前提。
/// ⚠️ **本常量是设计值的占位**，真实值待 `quill-agent` 背压落地后由主理人确认。
pub const MAX_TEAM_MEMBERS: usize = 8;

/// 团队聚合的失败原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamError {
    /// 团队标识为空。
    IdEmpty,
    /// 团队标识非法。
    IdInvalid(String),
    /// 团队无名称。
    EmptyName,
    /// 同一专家重复加入本团队。
    DuplicateMember(ExpertId),
    /// 成员数超上限。
    TooManyMembers {
        /// 当前成员数。
        got: usize,
        /// 上限。
        max: usize,
    },
    /// 加入一个不在名册里的专家 —— 允许任意字符串注册会让
    /// 「谁能进团」退化成字符串匹配，权限边界消失。
    UnknownExpert(ExpertId),
}

impl std::fmt::Display for TeamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IdEmpty => f.write_str("团队标识为空"),
            Self::IdInvalid(s) => write!(f, "团队标识非法：{s}"),
            Self::EmptyName => f.write_str("团队名称为空"),
            Self::DuplicateMember(e) => write!(f, "专家 {e} 已在该团队中"),
            Self::TooManyMembers { got, max } => {
                write!(f, "团队成员数 {got} 超过上限 {max}")
            }
            Self::UnknownExpert(e) => write!(f, "专家 {e} 不在名册中"),
        }
    }
}

impl std::error::Error for TeamError {}

/// 专家团聚合根。
///
/// **持有 `ExpertId` 而非 `MemberId`**：成员集合里的每一项都是**可复用身份**，
/// 因此同一个专家可以同时在多个 `Team` 里 —— 这正是 Octop 规则 1 的要求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    id: TeamId,
    name: String,
    leader: ExpertId,
    members: BTreeSet<ExpertId>,
}

impl Team {
    /// 创建团队。成员集合初始为**空**（不是含 leader）。
    ///
    /// ⚠️ 为什么 leader 不自动入成员集合：自动加入会让
    /// 「成员数上限为 8」在 `add_member` 时出现 off-by-one，
    /// 且 `members()` 的语义从"成员"滑向"成员+主持人"。
    /// leader 与 members 的关系由 [`Team::is_leader`] 表达。
    pub fn new(id: TeamId, name: impl Into<String>, leader: ExpertId) -> Result<Self, TeamError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(TeamError::EmptyName);
        }
        Ok(Self {
            id,
            name,
            leader,
            members: BTreeSet::new(),
        })
    }

    /// 团队标识。
    pub fn id(&self) -> &TeamId {
        &self.id
    }

    /// 团队名称。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 主持人（leader）专家身份。
    pub fn leader(&self) -> &ExpertId {
        &self.leader
    }

    /// 某专家是否为主持人。
    pub fn is_leader(&self, expert: &ExpertId) -> bool {
        &self.leader == expert
    }

    /// 加入成员。
    ///
    /// 不变量（全部**有反向用例**，见本文件 `tests`）：
    /// 1. 不得重复（同一专家在一个团队里只能有一个席位）；
    /// 2. 成员数不得超 `MAX_TEAM_MEMBERS`；
    /// 3. 专家必须在名册内（`roster`）。
    pub fn add_member(
        &mut self,
        expert: ExpertId,
        roster: &BTreeSet<ExpertId>,
    ) -> Result<AddOutcome, TeamError> {
        if !roster.contains(&expert) {
            return Err(TeamError::UnknownExpert(expert));
        }
        if self.members.contains(&expert) {
            return Err(TeamError::DuplicateMember(expert));
        }
        if self.members.len() >= MAX_TEAM_MEMBERS {
            return Err(TeamError::TooManyMembers {
                got: self.members.len() + 1,
                max: MAX_TEAM_MEMBERS,
            });
        }
        self.members.insert(expert);
        Ok(AddOutcome::Added)
    }

    /// 移除成员。返回是否真的移除了。
    ///
    /// ⚠️ 移除一个不在成员集合里的专家返回 `false` 而**不是 Err**：
    /// 「本来就不在」与「移除失败」是不同的语义，混成一个错误会让
    /// 调用方无法区分幂等重试与真实故障。
    pub fn remove_member(&mut self, expert: &ExpertId) -> bool {
        self.members.remove(expert)
    }

    /// 成员数。
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// 是否含该专家（成员集合内，不含 leader 隐含身份）。
    pub fn has_member(&self, expert: &ExpertId) -> bool {
        self.members.contains(expert)
    }

    /// 成员迭代（按 `ExpertId` 序，保证可复现）。
    pub fn members(&self) -> impl Iterator<Item = &ExpertId> {
        self.members.iter()
    }
}

/// `add_member` 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    /// 已加入。
    Added,
}

/// 便捷构造：合法团队标识 + 名称 + leader。
pub fn team_of(id: &str, name: &str, leader: &str) -> Result<Team, TeamError> {
    let tid = TeamId::parse(id).map_err(|e| match e {
        crate::TeamIdError::Empty => TeamError::IdEmpty,
        other => TeamError::IdInvalid(other.to_string()),
    })?;
    let lead =
        ExpertId::parse(leader).map_err(|e| TeamError::IdInvalid(format!("leader 无效：{e}")))?;
    Team::new(tid, name, lead)
}

/// 构造名册（专家注册表）。
pub fn roster(ids: &[&str]) -> BTreeSet<ExpertId> {
    ids.iter()
        .map(|s| ExpertId::parse(s).expect("测试用专家名应合法"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster3() -> BTreeSet<ExpertId> {
        roster(&["cost-analyst", "growth-analyst", "risk-reviewer"])
    }

    #[test]
    fn team_starts_with_empty_member_set_and_keeps_leader() {
        let t = team_of("growth-squad", "增长小队", "cost-analyst").expect("应合法");
        assert_eq!(t.id().as_str(), "growth-squad");
        assert_eq!(t.name(), "增长小队");
        assert_eq!(t.leader().as_str(), "cost-analyst");
        assert_eq!(
            t.member_count(),
            0,
            "leader 不自动入成员集合（避免成员数 off-by-one）"
        );
        assert!(
            t.is_leader(&ExpertId::parse("cost-analyst").expect("应合法")),
            "leader 身份须可判定"
        );
        assert!(
            !t.has_member(&ExpertId::parse("cost-analyst").expect("应合法")),
            "leader 与成员是两个概念"
        );
    }

    #[test]
    fn team_rejects_empty_id_and_name() {
        assert_eq!(
            team_of("", "名", "cost-analyst").unwrap_err(),
            TeamError::IdEmpty
        );
        assert_eq!(
            team_of(" ", "名", "cost-analyst").unwrap_err(),
            TeamError::IdEmpty
        );
        assert_eq!(
            team_of("t-1", "   ", "cost-analyst").unwrap_err(),
            TeamError::EmptyName
        );
    }

    #[test]
    fn team_rejects_malformed_id() {
        assert!(matches!(
            team_of("Bad Name", "名", "cost-analyst").unwrap_err(),
            TeamError::IdInvalid(_)
        ));
    }

    #[test]
    fn add_member_registers_expert_and_count_grows() {
        let mut t = team_of("growth-squad", "增长小队", "cost-analyst").expect("应合法");
        let r = roster3();
        assert_eq!(
            t.add_member(ExpertId::parse("cost-analyst").expect("应合法"), &r)
                .expect("应成功"),
            AddOutcome::Added
        );
        assert_eq!(t.member_count(), 1, "已检查：加入 1 人后应为 1");
        assert!(t.has_member(&ExpertId::parse("cost-analyst").expect("应合法")));
    }

    #[test]
    fn add_member_rejects_duplicate_expert_in_same_team() {
        // 反向用例：同一团队不得有两个相同专家席位。
        let mut t = team_of("growth-squad", "增长小队", "cost-analyst").expect("应合法");
        let r = roster3();
        let e = ExpertId::parse("cost-analyst").expect("应合法");
        t.add_member(e.clone(), &r).expect("首次应成功");
        assert_eq!(
            t.add_member(e.clone(), &r).unwrap_err(),
            TeamError::DuplicateMember(e.clone())
        );
        assert_eq!(t.member_count(), 1, "重复加入不得改变成员数");
    }

    #[test]
    fn add_member_rejects_expert_not_in_roster() {
        // 反向用例：名册外的专家不得进团。
        let mut t = team_of("growth-squad", "增长小队", "cost-analyst").expect("应合法");
        let r = roster3();
        let stranger = ExpertId::parse("stranger").expect("应合法");
        assert_eq!(
            t.add_member(stranger.clone(), &r).unwrap_err(),
            TeamError::UnknownExpert(stranger)
        );
        assert_eq!(t.member_count(), 0, "被拒的加入不得留下痕迹");
    }

    #[test]
    fn add_member_enforces_cap_and_reports_got_and_max() {
        // 反向用例：超上限判红，且错误里带实际值与上限（可诊断）。
        let mut t = team_of("big-team", "大团队", "cost-analyst").expect("应合法");
        let mut r: BTreeSet<ExpertId> = BTreeSet::new();
        for i in 0..=MAX_TEAM_MEMBERS {
            let name = format!("expert-{i}");
            let e = ExpertId::parse(&name).expect("应合法");
            r.insert(e);
        }
        for i in 0..MAX_TEAM_MEMBERS {
            let e = ExpertId::parse(&format!("expert-{i}")).expect("应合法");
            t.add_member(e, &r).expect("上限内应成功");
        }
        assert_eq!(t.member_count(), MAX_TEAM_MEMBERS, "已检查：应恰好到上限");
        let overflow = ExpertId::parse("expert-8").expect("应合法");
        assert_eq!(
            t.add_member(overflow, &r).unwrap_err(),
            TeamError::TooManyMembers {
                got: MAX_TEAM_MEMBERS + 1,
                max: MAX_TEAM_MEMBERS
            }
        );
        assert_eq!(
            t.member_count(),
            MAX_TEAM_MEMBERS,
            "超限被拒后成员数不得变化"
        );
    }

    #[test]
    fn same_expert_can_join_multiple_teams() {
        // 🔴 Octop 规则 1 的可执行证据：
        // 一个专家身份同时存在于两个团队，且互不影响。
        let e = ExpertId::parse("cost-analyst").expect("应合法");
        let r = roster(&["cost-analyst"]);
        let mut t1 = team_of("team-a", "A 队", "growth-analyst").expect("应合法");
        let mut t2 = team_of("team-b", "B 队", "growth-analyst").expect("应合法");
        t1.add_member(e.clone(), &r).expect("应成功");
        t2.add_member(e.clone(), &r).expect("应成功");
        assert_eq!(t1.member_count(), 1);
        assert_eq!(t2.member_count(), 1);
        assert!(t1.has_member(&e) && t2.has_member(&e));

        // ⚠️ 但**执行实例**不可跨团队共享：从 t1 移除不影响 t2。
        assert!(t1.remove_member(&e));
        assert!(!t1.has_member(&e));
        assert!(t2.has_member(&e), "从 t1 移除成员不得影响 t2 的成员关系");
    }

    #[test]
    fn remove_member_returns_false_for_non_member() {
        // 幂等语义：移除非成员返回 false 而非 Err（区别于「移除失败」）。
        let mut t = team_of("t-1", "队", "cost-analyst").expect("应合法");
        let absent = ExpertId::parse("growth-analyst").expect("应合法");
        assert!(!t.remove_member(&absent), "移除非成员应返回 false");
        assert_eq!(t.member_count(), 0, "移除非成员不得改变成员数");
    }

    #[test]
    fn remove_member_twice_is_idempotent() {
        let mut t = team_of("t-1", "队", "cost-analyst").expect("应合法");
        let r = roster3();
        let e = ExpertId::parse("cost-analyst").expect("应合法");
        t.add_member(e.clone(), &r).expect("应成功");
        assert!(t.remove_member(&e), "首次移除应成功");
        assert!(!t.remove_member(&e), "二次移除应返回 false（幂等）");
        assert_eq!(t.member_count(), 0);
    }

    #[test]
    fn members_iterate_in_deterministic_order() {
        let mut t = team_of("t-1", "队", "lead").expect("应合法");
        let r = roster3();
        for name in ["risk-reviewer", "cost-analyst", "growth-analyst"] {
            t.add_member(ExpertId::parse(name).expect("应合法"), &r)
                .expect("应成功");
        }
        let got: Vec<&str> = t.members().map(|e| e.as_str()).collect();
        assert_eq!(
            got,
            vec!["cost-analyst", "growth-analyst", "risk-reviewer"],
            "成员必须按 ExpertId 序输出，保证测试可复现"
        );
    }

    #[test]
    fn empty_roster_rejects_every_expert() {
        // 反向用例：名册为空时任何专家都进不来。
        let mut t = team_of("t-1", "队", "cost-analyst").expect("应合法");
        let empty = BTreeSet::new();
        let e = ExpertId::parse("cost-analyst").expect("应合法");
        assert_eq!(
            t.add_member(e.clone(), &empty).unwrap_err(),
            TeamError::UnknownExpert(e)
        );
        assert_eq!(t.member_count(), 0);
    }

    #[test]
    fn error_display_carries_the_offending_identity() {
        // 错误信息必须能定位到"是谁"，否则用户无法自诊断（铁律七）。
        let msg = TeamError::DuplicateMember(ExpertId::parse("cost-analyst").expect("应合法"))
            .to_string();
        assert!(msg.contains("cost-analyst"), "错误须含专家名：{msg}");
        let msg2 = TeamError::TooManyMembers { got: 9, max: 8 }.to_string();
        assert!(
            msg2.contains('9') && msg2.contains('8'),
            "错误须含实际值与上限：{msg2}"
        );
    }
}
