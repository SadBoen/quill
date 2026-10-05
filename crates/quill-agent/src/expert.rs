use std::collections::BTreeSet;

use quill_adapters::{ExpertId, UserId};

use crate::error::AgentError;

pub const SYSTEM_OWNER: UserId = UserId::from_bytes([0u8; 16]);

pub const MAX_DISPLAY_NAME: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Visibility {
    DefaultVisible,

    ManualEnable,

    BuiltinSystem,

    UserAuthored,
}

impl Visibility {
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::DefaultVisible => "default_visible",
            Self::ManualEnable => "manual_enable",
            Self::BuiltinSystem => "builtin_system",
            Self::UserAuthored => "user_authored",
        }
    }

    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "default_visible" => Some(Self::DefaultVisible),
            "manual_enable" => Some(Self::ManualEnable),
            "builtin_system" => Some(Self::BuiltinSystem),
            "user_authored" => Some(Self::UserAuthored),
            _ => None,
        }
    }

    pub fn all() -> [Visibility; 4] {
        [
            Self::DefaultVisible,
            Self::ManualEnable,
            Self::BuiltinSystem,
            Self::UserAuthored,
        ]
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expert {
    id: ExpertId,
    owner: UserId,
    display_name: String,
    description: String,
    visibility: Visibility,
    default_enabled: bool,
    builtin: bool,
    deleted: bool,
}

impl Expert {
    pub fn user_authored(
        owner: UserId,
        id: ExpertId,
        display_name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, AgentError> {
        let display_name = display_name.into();
        let description = description.into();
        check_display_name(&display_name)?;
        if owner == SYSTEM_OWNER {
            return Err(AgentError::ExpertBuiltinProtected { id });
        }
        Ok(Self {
            id,
            owner,
            display_name,
            description,
            visibility: Visibility::UserAuthored,
            default_enabled: true,
            builtin: false,
            deleted: false,
        })
    }

    pub fn builtin(
        id: ExpertId,
        display_name: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<Self, AgentError> {
        let display_name = display_name.into();
        let description = description.into();
        check_display_name(&display_name)?;
        Ok(Self {
            id,
            owner: SYSTEM_OWNER,
            display_name,
            description,
            visibility: Visibility::BuiltinSystem,
            default_enabled: true,
            builtin: true,
            deleted: false,
        })
    }

    pub fn id(&self) -> &ExpertId {
        &self.id
    }

    pub fn owner(&self) -> UserId {
        self.owner
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    pub fn default_enabled(&self) -> bool {
        self.default_enabled
    }

    pub fn is_builtin(&self) -> bool {
        self.builtin
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    pub fn is_visible_to(&self, viewer: &UserId) -> bool {
        if self.deleted {
            return false;
        }
        match self.visibility {
            Visibility::UserAuthored => *viewer == self.owner,
            Visibility::BuiltinSystem | Visibility::DefaultVisible | Visibility::ManualEnable => {
                true
            }
        }
    }

    pub fn is_modifiable_by(&self, actor: &UserId) -> bool {
        !self.builtin && !self.deleted && *actor == self.owner
    }

    pub fn rename(
        &mut self,
        actor: &UserId,
        display_name: impl Into<String>,
    ) -> Result<(), AgentError> {
        self.assert_modifiable(actor)?;
        let display_name = display_name.into();
        check_display_name(&display_name)?;
        self.display_name = display_name;
        Ok(())
    }

    pub fn redescribe(
        &mut self,
        actor: &UserId,
        description: impl Into<String>,
    ) -> Result<(), AgentError> {
        self.assert_modifiable(actor)?;
        self.description = description.into();
        Ok(())
    }

    pub fn set_default_enabled(&mut self, actor: &UserId, on: bool) -> Result<(), AgentError> {
        self.assert_modifiable(actor)?;

        if self.builtin && !on {
            return Err(AgentError::ExpertBuiltinProtected {
                id: self.id.clone(),
            });
        }
        self.default_enabled = on;
        Ok(())
    }

    pub fn soft_delete(&mut self, actor: &UserId) -> Result<bool, AgentError> {
        if self.deleted {
            return Ok(false);
        }
        self.assert_modifiable(actor)?;
        self.deleted = true;
        Ok(true)
    }

    fn assert_modifiable(&self, actor: &UserId) -> Result<(), AgentError> {
        if self.builtin {
            return Err(AgentError::ExpertBuiltinProtected {
                id: self.id.clone(),
            });
        }
        if self.deleted {
            return Err(AgentError::ExpertDeleted {
                id: self.id.clone(),
            });
        }
        if *actor != self.owner {
            return Err(AgentError::ExpertNotModifiable {
                id: self.id.clone(),
            });
        }
        Ok(())
    }

    pub fn check_cross_invariant(&self) -> Result<(), AgentError> {
        if self.builtin == (self.owner == SYSTEM_OWNER) {
            Ok(())
        } else {
            Err(AgentError::InvariantBroken {
                detail: format!(
                    "专家 {} 的 is_builtin={} 与属主={} 不一致（schema CHECK 会拒绝这次写入）",
                    self.id,
                    u8::from(self.builtin),
                    self.owner
                ),
            })
        }
    }
}

fn check_display_name(raw: &str) -> Result<(), AgentError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AgentError::ExpertDisplayNameInvalid {
            raw: raw.to_string(),
            reason: "不能为空或全空白",
        });
    }

    let chars = trimmed.chars().count();
    if chars > MAX_DISPLAY_NAME {
        return Err(AgentError::ExpertDisplayNameInvalid {
            raw: raw.to_string(),
            reason: "超过 64 个字符",
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewExpert {
    pub id: ExpertId,

    pub display_name: String,

    pub description: String,
}

pub trait ExpertRepository: Send + Sync + 'static {
    fn get(&self, owner: &UserId, id: &ExpertId) -> Result<Option<Expert>, AgentError>;

    fn list_owned(&self, owner: &UserId) -> Result<Vec<Expert>, AgentError>;

    fn put(&self, expert: &Expert) -> Result<(), AgentError>;

    fn roster(&self, viewer: &UserId) -> Result<BTreeSet<ExpertId>, AgentError>;
}

#[derive(Debug)]
pub struct ExpertRegistry<R: ExpertRepository> {
    repo: R,
}

impl<R: ExpertRepository> ExpertRegistry<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    pub fn repo(&self) -> &R {
        &self.repo
    }

    pub fn create_user_expert(&self, owner: UserId, new: NewExpert) -> Result<Expert, AgentError> {
        if let Some(existing) = self.repo.get(&owner, &new.id)? {
            if !existing.is_deleted() {
                return Err(AgentError::ExpertExists { id: new.id });
            }
        }
        let expert = Expert::user_authored(owner, new.id, new.display_name, new.description)?;
        expert.check_cross_invariant()?;
        self.repo.put(&expert)?;
        Ok(expert)
    }

    pub fn create_builtin_expert(&self, new: NewExpert) -> Result<Expert, AgentError> {
        if let Some(existing) = self.repo.get(&SYSTEM_OWNER, &new.id)? {
            if !existing.is_deleted() {
                return Err(AgentError::ExpertExists { id: new.id });
            }
        }
        let expert = Expert::builtin(new.id, new.display_name, new.description)?;
        expert.check_cross_invariant()?;
        self.repo.put(&expert)?;
        Ok(expert)
    }

    pub fn get_visible(&self, viewer: &UserId, id: &ExpertId) -> Result<Expert, AgentError> {
        if let Some(e) = self.repo.get(viewer, id)? {
            if e.is_visible_to(viewer) {
                return Ok(e);
            }
            return Err(AgentError::ExpertNotFound { id: id.clone() });
        }

        if let Some(e) = self.repo.get(&SYSTEM_OWNER, id)? {
            if e.is_visible_to(viewer) {
                return Ok(e);
            }
        }
        Err(AgentError::ExpertNotFound { id: id.clone() })
    }

    pub fn list_visible(&self, viewer: &UserId) -> Result<Vec<Expert>, AgentError> {
        let mut out: Vec<Expert> = self
            .repo
            .list_owned(viewer)?
            .into_iter()
            .filter(|e| e.is_visible_to(viewer))
            .collect();
        out.extend(
            self.repo
                .list_owned(&SYSTEM_OWNER)?
                .into_iter()
                .filter(|e| e.is_visible_to(viewer)),
        );
        out.sort_by(|a, b| a.id.cmp(&b.id));

        out.dedup_by(|a, b| a.id == b.id);
        Ok(out)
    }

    pub fn rename(
        &self,
        actor: &UserId,
        id: &ExpertId,
        display_name: impl Into<String>,
    ) -> Result<Expert, AgentError> {
        let mut e = self.must_get_owned(actor, id)?;
        e.rename(actor, display_name)?;
        e.check_cross_invariant()?;
        self.repo.put(&e)?;
        Ok(e)
    }

    pub fn redescribe(
        &self,
        actor: &UserId,
        id: &ExpertId,
        description: impl Into<String>,
    ) -> Result<Expert, AgentError> {
        let mut e = self.must_get_owned(actor, id)?;
        e.redescribe(actor, description)?;
        self.repo.put(&e)?;
        Ok(e)
    }

    pub fn set_default_enabled(
        &self,
        actor: &UserId,
        id: &ExpertId,
        on: bool,
    ) -> Result<Expert, AgentError> {
        let mut e = self.must_get_owned(actor, id)?;
        e.set_default_enabled(actor, on)?;
        self.repo.put(&e)?;
        Ok(e)
    }

    pub fn delete(&self, actor: &UserId, id: &ExpertId) -> Result<bool, AgentError> {
        if let Some(e) = self.repo.get(actor, id)? {
            if e.builtin {
                return Err(AgentError::ExpertBuiltinProtected { id: id.clone() });
            }
            if e.deleted {
                return Ok(false);
            }
            let mut e = e;
            let changed = e.soft_delete(actor)?;
            if changed {
                self.repo.put(&e)?;
            }
            return Ok(changed);
        }

        if let Some(e) = self.repo.get(&SYSTEM_OWNER, id)? {
            if e.deleted {
                return Ok(false);
            }
            return Err(AgentError::ExpertBuiltinProtected { id: id.clone() });
        }
        Err(AgentError::ExpertNotFound { id: id.clone() })
    }

    fn must_get_owned(&self, actor: &UserId, id: &ExpertId) -> Result<Expert, AgentError> {
        if let Some(e) = self.repo.get(actor, id)? {
            if e.builtin {
                return Err(AgentError::ExpertBuiltinProtected { id: id.clone() });
            }
            if e.deleted {
                return Err(AgentError::ExpertDeleted { id: id.clone() });
            }
            if !e.is_modifiable_by(actor) {
                return Err(AgentError::ExpertNotModifiable { id: id.clone() });
            }
            return Ok(e);
        }

        if let Some(e) = self.repo.get(&SYSTEM_OWNER, id)? {
            if e.deleted {
                return Err(AgentError::ExpertDeleted { id: id.clone() });
            }
            return Err(AgentError::ExpertBuiltinProtected { id: id.clone() });
        }
        Err(AgentError::ExpertNotFound { id: id.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    fn u(n: u8) -> UserId {
        UserId::from_bytes([n; 16])
    }

    fn e(name: &str) -> ExpertId {
        ExpertId::parse(name).expect("测试用专家名应合法")
    }

    #[derive(Debug, Default)]
    struct MemRepo {
        rows: Mutex<BTreeMap<(UserId, String), Expert>>,
    }

    impl ExpertRepository for MemRepo {
        fn get(&self, owner: &UserId, id: &ExpertId) -> Result<Option<Expert>, AgentError> {
            Ok(self
                .rows
                .lock()
                .expect("仓库锁不应被毒化")
                .get(&(*owner, id.as_str().to_string()))
                .cloned())
        }

        fn list_owned(&self, owner: &UserId) -> Result<Vec<Expert>, AgentError> {
            Ok(self
                .rows
                .lock()
                .expect("仓库锁不应被毒化")
                .iter()
                .filter(|((o, _), _)| o == owner)
                .map(|(_, v)| v.clone())
                .collect())
        }

        fn put(&self, expert: &Expert) -> Result<(), AgentError> {
            let key = (expert.owner(), expert.id().as_str().to_string());
            self.rows
                .lock()
                .expect("仓库锁不应被毒化")
                .insert(key, expert.clone());
            Ok(())
        }

        fn roster(&self, viewer: &UserId) -> Result<BTreeSet<ExpertId>, AgentError> {
            let g = self.rows.lock().expect("仓库锁不应被毒化");
            let mut out = BTreeSet::new();
            for ((_, _), v) in g.iter() {
                if v.is_visible_to(viewer) {
                    out.insert(v.id().clone());
                }
            }
            Ok(out)
        }
    }

    fn new(name: &str) -> NewExpert {
        NewExpert {
            id: e(name),
            display_name: format!("{name} 专家"),
            description: "测试用描述".into(),
        }
    }

    fn registry() -> ExpertRegistry<MemRepo> {
        ExpertRegistry::new(MemRepo::default())
    }

    #[test]
    fn create_user_expert_persists_with_user_authored_visibility() {
        let r = registry();
        let got = r
            .create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建成功");
        assert_eq!(got.id().as_str(), "cost-analyst");
        assert_eq!(got.owner(), u(1));
        assert_eq!(got.visibility(), Visibility::UserAuthored);
        assert!(got.default_enabled(), "自建专家默认启用");
        assert!(!got.is_builtin());
        assert!(!got.is_deleted());
    }

    #[test]
    fn duplicate_expert_id_under_same_owner_is_rejected() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("首次应成功");
        assert_eq!(
            r.create_user_expert(u(1), new("cost-analyst")).unwrap_err(),
            AgentError::ExpertExists {
                id: e("cost-analyst")
            }
        );
    }

    #[test]
    fn same_expert_id_under_different_owners_is_allowed() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("A 应成功");
        r.create_user_expert(u(2), new("cost-analyst"))
            .expect("B 也应成功");
        assert_eq!(
            r.repo().list_owned(&u(1)).expect("应可列出").len(),
            1,
            "A 名下应只有 1 个"
        );
        assert_eq!(
            r.repo().list_owned(&u(2)).expect("应可列出").len(),
            1,
            "B 名下应只有 1 个"
        );
    }

    #[test]
    fn display_name_must_be_non_empty_and_within_64_chars() {
        let r = registry();
        for bad in ["", "   ", "\t\n"] {
            let mut n = new("cost-analyst");
            n.display_name = bad.to_string();
            let err = r.create_user_expert(u(1), n).unwrap_err();
            assert!(
                matches!(err, AgentError::ExpertDisplayNameInvalid { .. }),
                "空显示名 {bad:?} 必须判红：{err:?}"
            );
        }
        let mut n = new("cost-analyst");
        n.display_name = "x".repeat(65);
        assert!(matches!(
            r.create_user_expert(u(1), n).unwrap_err(),
            AgentError::ExpertDisplayNameInvalid { .. }
        ));
    }

    #[test]
    fn sixty_four_chinese_chars_are_accepted() {
        let r = registry();
        let mut n = new("cost-analyst");
        n.display_name = "成".repeat(64);
        let got = r.create_user_expert(u(1), n).expect("64 个汉字应合法");
        assert_eq!(got.display_name().chars().count(), 64);
    }

    #[test]
    fn builtin_expert_is_owned_by_system_owner_and_cannot_be_modified() {
        let r = registry();
        let b = r
            .create_builtin_expert(new("builtin-helper"))
            .expect("应创建成功");
        assert_eq!(b.owner(), SYSTEM_OWNER, "内置专家属主必须是全零");
        assert!(b.is_builtin());
        assert_eq!(b.visibility(), Visibility::BuiltinSystem);

        for actor in [u(1), u(2), SYSTEM_OWNER] {
            let err = r
                .rename(&actor, &e("builtin-helper"), "改名")
                .expect_err("内置专家谁都不该改得动");
            assert_eq!(
                err,
                AgentError::ExpertBuiltinProtected {
                    id: e("builtin-helper")
                },
                "actor {actor} 改内置专家应判保护"
            );
        }
    }

    #[test]
    fn builtin_expert_cannot_be_disabled() {
        let r = registry();
        r.create_builtin_expert(new("builtin-helper"))
            .expect("应创建");
        let err = r
            .set_default_enabled(&SYSTEM_OWNER, &e("builtin-helper"), false)
            .expect_err("内置专家不可关闭");
        assert_eq!(
            err,
            AgentError::ExpertBuiltinProtected {
                id: e("builtin-helper")
            }
        );
    }

    #[test]
    fn user_cannot_claim_the_system_owner_identity() {
        let got = Expert::user_authored(SYSTEM_OWNER, e("fake"), "假内置", "描述");
        assert_eq!(
            got.unwrap_err(),
            AgentError::ExpertBuiltinProtected { id: e("fake") }
        );
    }

    #[test]
    fn cross_invariant_holds_for_both_construction_paths_and_detects_tampering() {
        Expert::builtin(e("b"), "b", "d")
            .expect("内置应自洽")
            .check_cross_invariant()
            .expect("内置专家交叉不变量应成立");
        Expert::user_authored(u(7), e("u"), "u", "d")
            .expect("自建应自洽")
            .check_cross_invariant()
            .expect("自建专家交叉不变量应成立");

        let mut tampered = Expert::user_authored(u(7), e("u"), "u", "d").expect("应合法");
        tampered.builtin = true;
        let err = tampered
            .check_cross_invariant()
            .expect_err("篡改必须被检出");
        assert!(
            matches!(err, AgentError::InvariantBroken { .. }),
            "须判不变量破坏：{err:?}"
        );
    }

    #[test]
    fn user_authored_expert_is_visible_only_to_its_owner() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        assert_eq!(
            r.list_visible(&u(1)).expect("应可列出").len(),
            1,
            "属主应看得见自己的专家"
        );
        assert!(
            r.list_visible(&u(2)).expect("应可列出").is_empty(),
            "🔴 跨用户隔离：B 用户不得看到 A 的自建专家"
        );
    }

    #[test]
    fn builtin_expert_is_visible_to_every_user() {
        let r = registry();
        r.create_builtin_expert(new("builtin-helper"))
            .expect("应创建");
        for viewer in [u(1), u(2), u(3)] {
            assert_eq!(
                r.list_visible(&viewer).expect("应可列出").len(),
                1,
                "内置专家对用户 {viewer} 应可见"
            );
        }
    }

    #[test]
    fn invisible_expert_reports_not_found_not_forbidden() {
        let r = registry();
        r.create_user_expert(u(1), new("secret-expert"))
            .expect("应创建");
        let err = r
            .get_visible(&u(2), &e("secret-expert"))
            .expect_err("不可见");
        assert_eq!(
            err,
            AgentError::ExpertNotFound {
                id: e("secret-expert")
            }
        );
        assert_ne!(
            err.code(),
            "expert_not_modifiable",
            "可见性问题不得复用「不可修改」的码"
        );
    }

    #[test]
    fn list_visible_is_sorted_and_deduplicated() {
        let r = registry();
        r.create_user_expert(u(1), new("zeta")).expect("应创建");
        r.create_user_expert(u(1), new("alpha")).expect("应创建");
        r.create_builtin_expert(new("mid")).expect("应创建");
        let ids: Vec<String> = r
            .list_visible(&u(1))
            .expect("应可列出")
            .iter()
            .map(|e| e.id().as_str().to_string())
            .collect();
        assert_eq!(
            ids,
            vec!["alpha", "mid", "zeta"],
            "必须按 ExpertId 序且不重复"
        );
    }

    #[test]
    fn user_expert_shadowing_a_builtin_name_appears_once() {
        let r = registry();
        r.create_builtin_expert(new("helper")).expect("应创建");
        r.create_user_expert(u(1), new("helper")).expect("应创建");
        let got = r.list_visible(&u(1)).expect("应可列出");
        assert_eq!(got.len(), 1, "同名只应出现 1 条：{got:?}");
        assert_eq!(
            got[0].owner(),
            u(1),
            "应保留用户那份（用户自建优先于同名内置）"
        );
    }

    #[test]
    fn roster_matches_list_visible_scope() {
        let r = registry();
        r.create_user_expert(u(1), new("mine")).expect("应创建");
        r.create_user_expert(u(2), new("theirs")).expect("应创建");
        r.create_builtin_expert(new("shared")).expect("应创建");

        let roster = r.repo().roster(&u(1)).expect("应可取名册");
        let list: BTreeSet<ExpertId> = r
            .list_visible(&u(1))
            .expect("应可列出")
            .iter()
            .map(|e| e.id().clone())
            .collect();
        assert_eq!(roster, list, "名册与列表口径必须完全一致");
        assert!(roster.contains(&e("shared")));
        assert!(!roster.contains(&e("theirs")), "🔴 跨用户隔离");
    }

    #[test]
    fn owner_can_rename_and_redescribe() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        let e1 = r
            .rename(&u(1), &e("cost-analyst"), "成本分析师")
            .expect("属主应可改名");
        assert_eq!(e1.display_name(), "成本分析师");
        let e2 = r
            .redescribe(&u(1), &e("cost-analyst"), "新描述")
            .expect("属主应可改描述");
        assert_eq!(e2.description(), "新描述");
    }

    #[test]
    fn non_owner_rename_reports_not_found_not_modifiable() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        assert_eq!(
            r.rename(&u(2), &e("cost-analyst"), "劫持").unwrap_err(),
            AgentError::ExpertNotFound {
                id: e("cost-analyst")
            }
        );
    }

    #[test]
    fn builtin_expert_modification_is_reported_as_protected_not_not_found() {
        let r = registry();
        r.create_builtin_expert(new("builtin-helper"))
            .expect("应创建");
        assert_eq!(
            r.rename(&u(2), &e("builtin-helper"), "劫持").unwrap_err(),
            AgentError::ExpertBuiltinProtected {
                id: e("builtin-helper")
            }
        );
    }

    #[test]
    fn renaming_to_blank_name_is_rejected_and_leaves_old_name_intact() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        assert!(r.rename(&u(1), &e("cost-analyst"), "  ").is_err());
        let got = r.get_visible(&u(1), &e("cost-analyst")).expect("应可取");
        assert_eq!(
            got.display_name(),
            "cost-analyst 专家",
            "失败的改名不得改掉原名"
        );
    }

    #[test]
    fn owner_can_toggle_default_enabled_off_and_on() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        let off = r
            .set_default_enabled(&u(1), &e("cost-analyst"), false)
            .expect("应可关闭");
        assert!(!off.default_enabled());
        let on = r
            .set_default_enabled(&u(1), &e("cost-analyst"), true)
            .expect("应可开启");
        assert!(on.default_enabled());
    }

    #[test]
    fn soft_delete_hides_expert_from_its_own_owner() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        assert!(r.delete(&u(1), &e("cost-analyst")).expect("首次应删成功"));
        assert!(
            r.list_visible(&u(1)).expect("应可列出").is_empty(),
            "已删专家对属主也不可见（否则用户以为没删掉）"
        );
        assert_eq!(
            r.get_visible(&u(1), &e("cost-analyst")).unwrap_err(),
            AgentError::ExpertNotFound {
                id: e("cost-analyst")
            }
        );
    }

    #[test]
    fn delete_twice_is_idempotent() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        assert!(r.delete(&u(1), &e("cost-analyst")).expect("首次应成功"));
        assert!(
            !r.delete(&u(1), &e("cost-analyst")).expect("二次不应报错"),
            "二次删除应返回 false（幂等）"
        );
    }

    #[test]
    fn deleted_expert_cannot_be_renamed_or_toggled() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        r.delete(&u(1), &e("cost-analyst")).expect("应删成功");
        assert_eq!(
            r.rename(&u(1), &e("cost-analyst"), "复活").unwrap_err(),
            AgentError::ExpertDeleted {
                id: e("cost-analyst")
            }
        );
        assert_eq!(
            r.set_default_enabled(&u(1), &e("cost-analyst"), true)
                .unwrap_err(),
            AgentError::ExpertDeleted {
                id: e("cost-analyst")
            }
        );
    }

    #[test]
    fn non_owner_cannot_delete_someone_elses_expert() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");

        assert_eq!(
            r.delete(&u(2), &e("cost-analyst")).unwrap_err(),
            AgentError::ExpertNotFound {
                id: e("cost-analyst")
            }
        );
    }

    #[test]
    fn builtin_expert_cannot_be_deleted() {
        let r = registry();
        r.create_builtin_expert(new("builtin-helper"))
            .expect("应创建");
        assert_eq!(
            r.delete(&SYSTEM_OWNER, &e("builtin-helper")).unwrap_err(),
            AgentError::ExpertBuiltinProtected {
                id: e("builtin-helper")
            }
        );
    }

    #[test]
    fn deleting_frees_the_name_for_recreation() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("首次应成功");
        r.delete(&u(1), &e("cost-analyst")).expect("应删成功");
        let again = r
            .create_user_expert(u(1), new("cost-analyst"))
            .expect("删掉后同名应可重建");
        assert!(!again.is_deleted());
    }

    #[test]
    fn visibility_wire_names_match_the_schema_check_list_exactly() {
        let want = [
            "default_visible",
            "manual_enable",
            "builtin_system",
            "user_authored",
        ];
        for (v, w) in Visibility::all().iter().zip(want.iter()) {
            assert_eq!(v.as_wire(), *w);
            assert_eq!(Visibility::from_wire(w), Some(*v), "{w} 必须能解析回来");
        }
        assert_eq!(want.len(), 4, "schema 的 CHECK 是 4 值");
    }

    #[test]
    fn unknown_wire_name_is_rejected_rather_than_defaulting() {
        for bad in ["", "public", "Default_Visible", "builtin"] {
            assert_eq!(Visibility::from_wire(bad), None, "{bad:?} 必须解析失败");
        }
    }
}
