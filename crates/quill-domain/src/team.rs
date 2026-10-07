use std::collections::BTreeSet;

use crate::{ExpertId, TeamId};

pub const MAX_TEAM_MEMBERS: usize = 8;

/// 主持人不算成员，所以「至少两人」指的是除主持人之外的成员数。
pub const MIN_TEAM_MEMBERS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamError {
    IdEmpty,

    IdInvalid(String),

    EmptyName,

    DuplicateMember(ExpertId),

    TooManyMembers {
        got: usize,
        max: usize,
    },

    TooFewMembers {
        got: usize,
        min: usize,
    },

    /// 团队只能由普通专家组成，不搞嵌套团队。
    MemberIsTeam {
        member: ExpertId,
        team: TeamId,
    },

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
            Self::TooFewMembers { got, min } => {
                write!(f, "团队成员数 {got} 少于下限 {min}")
            }
            Self::MemberIsTeam { member, team } => {
                write!(
                    f,
                    "{member} 是一个团队，不能作为 {team} 的成员（不支持嵌套团队）"
                )
            }
            Self::UnknownExpert(e) => write!(f, "专家 {e} 不在名册中"),
        }
    }
}

impl std::error::Error for TeamError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    id: TeamId,
    name: String,
    leader: ExpertId,
    members: BTreeSet<ExpertId>,
}

impl Team {
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

    pub fn id(&self) -> &TeamId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn leader(&self) -> &ExpertId {
        &self.leader
    }

    pub fn is_leader(&self, expert: &ExpertId) -> bool {
        &self.leader == expert
    }

    pub fn add_member(
        &mut self,
        expert: ExpertId,
        roster: &BTreeSet<ExpertId>,
    ) -> Result<AddOutcome, TeamError> {
        if expert.as_str() == self.id.as_str() {
            return Err(TeamError::MemberIsTeam {
                member: expert,
                team: self.id.clone(),
            });
        }
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

    /// 成员数是否满足上下限。
    ///
    /// 上下限都不在 `add_member` 里判：0 人团队在「往里加人」的过程中是合法的
    /// 中间态（下限必须在人加完之后再查），上限则已经在 add_member 里守住。
    pub fn validate(&self) -> Result<(), TeamError> {
        if self.members.len() < MIN_TEAM_MEMBERS {
            return Err(TeamError::TooFewMembers {
                got: self.members.len(),
                min: MIN_TEAM_MEMBERS,
            });
        }
        Ok(())
    }

    pub fn remove_member(&mut self, expert: &ExpertId) -> bool {
        self.members.remove(expert)
    }

    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    pub fn has_member(&self, expert: &ExpertId) -> bool {
        self.members.contains(expert)
    }

    pub fn members(&self) -> impl Iterator<Item = &ExpertId> {
        self.members.iter()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    Added,
}

pub fn team_of(id: &str, name: &str, leader: &str) -> Result<Team, TeamError> {
    let tid = TeamId::parse(id).map_err(|e| match e {
        crate::TeamIdError::Empty => TeamError::IdEmpty,
        other => TeamError::IdInvalid(other.to_string()),
    })?;
    let lead =
        ExpertId::parse(leader).map_err(|e| TeamError::IdInvalid(format!("leader 无效：{e}")))?;
    Team::new(tid, name, lead)
}

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
        let e = ExpertId::parse("cost-analyst").expect("应合法");
        let r = roster(&["cost-analyst"]);
        let mut t1 = team_of("team-a", "A 队", "growth-analyst").expect("应合法");
        let mut t2 = team_of("team-b", "B 队", "growth-analyst").expect("应合法");
        t1.add_member(e.clone(), &r).expect("应成功");
        t2.add_member(e.clone(), &r).expect("应成功");
        assert_eq!(t1.member_count(), 1);
        assert_eq!(t2.member_count(), 1);
        assert!(t1.has_member(&e) && t2.has_member(&e));

        assert!(t1.remove_member(&e));
        assert!(!t1.has_member(&e));
        assert!(t2.has_member(&e), "从 t1 移除成员不得影响 t2 的成员关系");
    }

    #[test]
    fn remove_member_returns_false_for_non_member() {
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
        let msg = TeamError::DuplicateMember(ExpertId::parse("cost-analyst").expect("应合法"))
            .to_string();
        assert!(msg.contains("cost-analyst"), "错误须含专家名：{msg}");
        let msg2 = TeamError::TooManyMembers { got: 9, max: 8 }.to_string();
        assert!(
            msg2.contains('9') && msg2.contains('8'),
            "错误须含实际值与上限：{msg2}"
        );
    }

    /// 下限只由 validate() 判：0 人团队在往里加人的过程中是合法中间态。
    #[test]
    fn validate_rejects_fewer_than_two_members() {
        let mut t = team_of("growth-squad", "增长小队", "cost-analyst").expect("应合法");
        let r = roster3();
        assert_eq!(
            t.validate().unwrap_err(),
            TeamError::TooFewMembers { got: 0, min: 2 },
            "空团队不是可交付的团队"
        );

        t.add_member(ExpertId::parse("cost-analyst").expect("应合法"), &r)
            .expect("应成功");
        assert_eq!(
            t.validate().unwrap_err(),
            TeamError::TooFewMembers { got: 1, min: 2 },
            "只有主持人 + 1 个成员仍然不成立（主持人不计入成员数）"
        );

        t.add_member(ExpertId::parse("growth-analyst").expect("应合法"), &r)
            .expect("应成功");
        assert_eq!(t.validate(), Ok(()), "两名成员必须放行");
    }

    /// 成员不能是团队自己：quill 不支持嵌套团队。判在 add_member 上而不是
    /// validate 上，因为这一条要拦的是「加进来」这个动作。
    #[test]
    fn a_team_cannot_become_a_member_of_itself() {
        let tid = TeamId::parse("growth-squad").expect("应合法");
        let r = roster(&["growth-squad", "cost-analyst"]);
        let mut t = Team::new(
            tid.clone(),
            "增长小队",
            ExpertId::parse("lead").expect("应合法"),
        )
        .expect("应合法");
        assert_eq!(
            t.add_member(ExpertId::parse("growth-squad").expect("应合法"), &r)
                .unwrap_err(),
            TeamError::MemberIsTeam {
                member: ExpertId::parse("growth-squad").expect("应合法"),
                team: tid,
            },
            "把自己当成员必须判红"
        );
        assert_eq!(t.member_count(), 0, "被拒的加入不得留下痕迹");
    }

    /// 名册里没有的专家和「团队自己」要能区分开：前者是名册问题，后者是规则问题，
    /// 错误文案不同，修复动作也不同。
    #[test]
    fn nested_team_is_not_mistaken_for_an_unknown_expert() {
        let r = roster3();
        let mut t = team_of("growth-squad", "增长小队", "cost-analyst").expect("应合法");
        let err = t
            .add_member(ExpertId::parse("growth-squad").expect("应合法"), &r)
            .unwrap_err();
        assert!(
            !matches!(err, TeamError::UnknownExpert(_)),
            "团队自己必须报「不能嵌套」，而不是「不在名册」：{err}"
        );
        assert!(err.to_string().contains("growth-squad"), "{err}");
    }

    #[test]
    fn min_and_max_are_two_and_eight() {
        assert_eq!(MIN_TEAM_MEMBERS, 2);
        assert_eq!(MAX_TEAM_MEMBERS, 8);
    }
}
