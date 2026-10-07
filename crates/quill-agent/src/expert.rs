use std::collections::BTreeSet;

use quill_adapters::{ExpertId, UserId};

use crate::error::AgentError;

pub const SYSTEM_OWNER: UserId = UserId::from_bytes([0u8; 16]);

pub const MAX_DISPLAY_NAME: usize = 64;

/// 人格正文上限。理由：人格正文会整条进入每一次模型请求的 system 消息，
/// 超过这个量就该拆成技能/知识而不是继续堆提示词。上限与 0004 的 CHECK 同口径。
pub const MAX_INSTRUCTIONS: usize = 20_000;

/// 偏好模型名上限，与 0004 的 CHECK 同口径。
pub const MAX_MODEL: usize = 128;

/// 来源模板 id 上限，与 0005 的 CHECK 同口径。取值规则是 `^[a-z0-9-]{1,64}$`：
/// 与专家 id 同一套字符集（模板 id 就是前端静态库里的那一批 id），
/// 但刻意不比 ExpertId::parse 更严 —— 契约冻结的就是这个正则。
pub const MAX_SOURCE_TEMPLATE: usize = 64;

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
    instructions: String,
    model: Option<String>,
    /// 派生来源的模板 id。`None` = 手建 / 内置专家，不来自任何模板。
    source_template: Option<String>,
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
        Self::user_authored_with_persona(owner, id, display_name, description, "", None)
    }

    /// 带人格的构造入口。`user_authored` 保留四位参数是为了不动既有调用点
    /// （quill-cli 的 `quill experts add` 走的仍是零人格版本）。
    pub fn user_authored_with_persona(
        owner: UserId,
        id: ExpertId,
        display_name: impl Into<String>,
        description: impl Into<String>,
        instructions: impl Into<String>,
        model: Option<String>,
    ) -> Result<Self, AgentError> {
        Self::user_authored_with_source(
            owner,
            id,
            display_name,
            description,
            instructions,
            model,
            None,
        )
    }

    /// 带来源模板的构造入口。`source_template` 记录「这个专家是从哪个模板生成的」；
    /// 同一个模板可以传进来生成任意多个专家（1:N）。
    pub fn user_authored_with_source(
        owner: UserId,
        id: ExpertId,
        display_name: impl Into<String>,
        description: impl Into<String>,
        instructions: impl Into<String>,
        model: Option<String>,
        source_template: Option<String>,
    ) -> Result<Self, AgentError> {
        let display_name = display_name.into();
        let description = description.into();
        let instructions = instructions.into();
        check_display_name(&display_name)?;
        check_instructions(&instructions)?;
        let model = check_model(model)?;
        let source_template = check_source_template(source_template)?;
        if owner == SYSTEM_OWNER {
            return Err(AgentError::ExpertBuiltinProtected { id });
        }
        Ok(Self {
            id,
            owner,
            display_name,
            description,
            instructions,
            model,
            source_template,
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
        Self::builtin_with_persona(id, display_name, description, "", None)
    }

    pub fn builtin_with_persona(
        id: ExpertId,
        display_name: impl Into<String>,
        description: impl Into<String>,
        instructions: impl Into<String>,
        model: Option<String>,
    ) -> Result<Self, AgentError> {
        Self::builtin_with_source(id, display_name, description, instructions, model, None)
    }

    pub fn builtin_with_source(
        id: ExpertId,
        display_name: impl Into<String>,
        description: impl Into<String>,
        instructions: impl Into<String>,
        model: Option<String>,
        source_template: Option<String>,
    ) -> Result<Self, AgentError> {
        let display_name = display_name.into();
        let description = description.into();
        let instructions = instructions.into();
        check_display_name(&display_name)?;
        check_instructions(&instructions)?;
        let model = check_model(model)?;
        let source_template = check_source_template(source_template)?;
        Ok(Self {
            id,
            owner: SYSTEM_OWNER,
            display_name,
            description,
            instructions,
            model,
            source_template,
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

    /// 人格正文（goose custom agent 的 markdown 正文）。空串 = 没有额外人格。
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// 偏好模型。`None` = 跟随实例默认模型（与 0004 里 model IS NULL 同义）。
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// 派生来源的模板 id。`None` = 不来自任何模板（手建 / 内置专家）。
    pub fn source_template(&self) -> Option<&str> {
        self.source_template.as_deref()
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

    pub fn set_instructions(
        &mut self,
        actor: &UserId,
        instructions: impl Into<String>,
    ) -> Result<(), AgentError> {
        self.assert_modifiable(actor)?;
        let instructions = instructions.into();
        check_instructions(&instructions)?;
        self.instructions = instructions;
        Ok(())
    }

    /// `None` 明确表示清除偏好模型（回到实例默认模型）。
    pub fn set_model(&mut self, actor: &UserId, model: Option<String>) -> Result<(), AgentError> {
        self.assert_modifiable(actor)?;
        self.model = check_model(model)?;
        Ok(())
    }

    /// `None` 明确表示清除来源模板（这个专家不再声称自己派生自任何模板）。
    pub fn set_source_template(
        &mut self,
        actor: &UserId,
        source_template: Option<String>,
    ) -> Result<(), AgentError> {
        self.assert_modifiable(actor)?;
        self.source_template = check_source_template(source_template)?;
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

fn check_instructions(raw: &str) -> Result<(), AgentError> {
    let chars = raw.chars().count();
    if chars > MAX_INSTRUCTIONS {
        return Err(AgentError::ExpertInstructionsInvalid {
            raw: raw.to_string(),
            reason: "超过 20000 个字符",
        });
    }
    Ok(())
}

/// `None` 与 `Some(非空)` 都合法；`Some(全空白)` 判红——那等于把模型切成空串，
/// 运行时只会得到一个打不通的请求，而不是回落到默认模型。
fn check_model(model: Option<String>) -> Result<Option<String>, AgentError> {
    let Some(m) = model else {
        return Ok(None);
    };
    let trimmed = m.trim();
    if trimmed.is_empty() {
        return Err(AgentError::ExpertModelInvalid {
            raw: m,
            reason: "trim 后为空串；要跟随实例默认模型请传 null 或省略该字段",
        });
    }
    if trimmed.chars().count() > MAX_MODEL {
        return Err(AgentError::ExpertModelInvalid {
            raw: m,
            reason: "超过 128 个字符",
        });
    }
    Ok(Some(trimmed.to_string()))
}

/// `None` 与 `Some(合法模板 id)` 都合法。取值规则 `^[a-z0-9-]{1,64}$`，与 0005 的
/// CHECK 同口径；不 trim、不放行空白，因为模板 id 是静态库里的标识，不是用户
/// 随手写的名字。**刻意不做存在性校验**：模板库由前端静态 vendor 进来，后端
/// 没有模板表可查，硬要查就得先造一张并不该由后端拥有的表。
fn check_source_template(src: Option<String>) -> Result<Option<String>, AgentError> {
    let Some(s) = src else {
        return Ok(None);
    };
    if s.is_empty() {
        return Err(AgentError::ExpertSourceTemplateInvalid {
            raw: s,
            reason: "为空串；不来自模板请传 null 或省略该字段",
        });
    }
    let len = s.chars().count();
    if len > MAX_SOURCE_TEMPLATE {
        return Err(AgentError::ExpertSourceTemplateInvalid {
            raw: s,
            reason: "超过 64 个字符",
        });
    }
    if s.chars().any(|c| !matches!(c, 'a'..='z' | '0'..='9' | '-')) {
        return Err(AgentError::ExpertSourceTemplateInvalid {
            raw: s,
            reason: "含非法字符，只允许小写字母、数字与连字符",
        });
    }
    Ok(Some(s))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewExpert {
    pub id: ExpertId,

    pub display_name: String,

    pub description: String,

    /// 人格正文；空串 = 没有额外人格。
    pub instructions: String,

    /// 偏好模型；`None` = 跟随实例默认模型。
    pub model: Option<String>,

    /// 派生来源的模板 id；`None` = 不来自任何模板。
    pub source_template: Option<String>,
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
        let expert = Expert::user_authored_with_source(
            owner,
            new.id,
            new.display_name,
            new.description,
            new.instructions,
            new.model,
            new.source_template,
        )?;
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
        let expert = Expert::builtin_with_source(
            new.id,
            new.display_name,
            new.description,
            new.instructions,
            new.model,
            new.source_template,
        )?;
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

    pub fn set_instructions(
        &self,
        actor: &UserId,
        id: &ExpertId,
        instructions: impl Into<String>,
    ) -> Result<Expert, AgentError> {
        let mut e = self.must_get_owned(actor, id)?;
        e.set_instructions(actor, instructions)?;
        self.repo.put(&e)?;
        Ok(e)
    }

    pub fn set_model(
        &self,
        actor: &UserId,
        id: &ExpertId,
        model: Option<String>,
    ) -> Result<Expert, AgentError> {
        let mut e = self.must_get_owned(actor, id)?;
        e.set_model(actor, model)?;
        self.repo.put(&e)?;
        Ok(e)
    }

    pub fn set_source_template(
        &self,
        actor: &UserId,
        id: &ExpertId,
        source_template: Option<String>,
    ) -> Result<Expert, AgentError> {
        let mut e = self.must_get_owned(actor, id)?;
        e.set_source_template(actor, source_template)?;
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
            instructions: "你是测试用专家".into(),
            model: None,
            source_template: None,
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

    #[test]
    fn persona_survives_the_registry_round_trip() {
        let r = registry();
        let mut n = new("cost-analyst");
        n.instructions = "你是一名成本分析师，先问清口径再算数。".into();
        n.model = Some("qwen3-max".into());
        r.create_user_expert(u(1), n).expect("应创建成功");

        let back = r.get_visible(&u(1), &e("cost-analyst")).expect("应可读");
        assert_eq!(
            back.instructions(),
            "你是一名成本分析师，先问清口径再算数。"
        );
        assert_eq!(back.model(), Some("qwen3-max"));
    }

    #[test]
    fn model_is_trimmed_and_none_means_follow_the_instance_default() {
        let r = registry();
        let mut n = new("cost-analyst");
        n.model = Some("  qwen3-max  ".into());
        let got = r.create_user_expert(u(1), n).expect("应创建成功");
        assert_eq!(got.model(), Some("qwen3-max"), "模型名必须 trim 后落库");

        let cleared = r
            .set_model(&u(1), &e("cost-analyst"), None)
            .expect("清除偏好模型应成功");
        assert_eq!(
            cleared.model(),
            None,
            "None 必须表示回到实例默认模型，而不是空串"
        );
    }

    #[test]
    fn blank_model_name_is_rejected_because_it_would_become_an_unreachable_request() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        for bad in ["", "   ", "\t\n"] {
            let err = r
                .set_model(&u(1), &e("cost-analyst"), Some(bad.to_string()))
                .expect_err("空模型名必须判红");
            assert!(
                matches!(err, AgentError::ExpertModelInvalid { .. }),
                "模型名 {bad:?} 必须判红：{err:?}"
            );
        }
        let got = r.get_visible(&u(1), &e("cost-analyst")).expect("应可取");
        assert_eq!(got.model(), None, "失败的写入不得改掉原值");
    }

    #[test]
    fn instructions_length_is_capped_at_twenty_thousand_chars() {
        let r = registry();
        let mut ok = new("cost-analyst");
        ok.instructions = "人".repeat(MAX_INSTRUCTIONS);
        let got = r.create_user_expert(u(1), ok).expect("上限本身必须放行");
        assert_eq!(got.instructions().chars().count(), MAX_INSTRUCTIONS);

        let mut bad = new("cost-analyst-2");
        bad.instructions = "人".repeat(MAX_INSTRUCTIONS + 1);
        let err = r.create_user_expert(u(1), bad).unwrap_err();
        assert!(
            matches!(err, AgentError::ExpertInstructionsInvalid { .. }),
            "超长人格必须判红：{err:?}"
        );
        assert!(
            err.to_string().contains("20001"),
            "文案应给出实际字数：{err}"
        );
    }

    #[test]
    fn over_long_instructions_is_rejected_on_update_too() {
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");
        let err = r
            .set_instructions(&u(1), &e("cost-analyst"), "人".repeat(MAX_INSTRUCTIONS + 1))
            .expect_err("更新超长也必须判红");
        assert!(matches!(err, AgentError::ExpertInstructionsInvalid { .. }));
        let got = r.get_visible(&u(1), &e("cost-analyst")).expect("应可取");
        assert_eq!(
            got.instructions(),
            "你是测试用专家",
            "失败的更新不得把旧人格冲掉"
        );
    }

    #[test]
    fn empty_instructions_are_legal_and_mean_no_extra_persona() {
        let r = registry();
        let mut n = new("cost-analyst");
        n.instructions = String::new();
        let got = r.create_user_expert(u(1), n).expect("空人格必须合法");
        assert_eq!(got.instructions(), "");
        assert!(got.instructions().trim().is_empty());
    }

    #[test]
    fn persona_edits_obey_the_same_owner_rules_as_the_name() {
        let r = registry();
        r.create_builtin_expert(new("builtin-helper"))
            .expect("应创建");
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("应创建");

        assert_eq!(
            r.set_instructions(&u(2), &e("cost-analyst"), "劫持")
                .unwrap_err(),
            AgentError::ExpertNotFound {
                id: e("cost-analyst")
            }
        );
        assert_eq!(
            r.set_model(&SYSTEM_OWNER, &e("builtin-helper"), Some("m".into()))
                .unwrap_err(),
            AgentError::ExpertBuiltinProtected {
                id: e("builtin-helper")
            },
            "内置专家的人格与内置专家本身一样受保护"
        );
    }

    #[test]
    fn the_legacy_four_arg_constructor_still_yields_a_zero_persona_expert() {
        // quill-cli 的 `quill experts add` 走的是这个四位签名，不能因为
        // 加人格字段就把它删掉或改签名。
        let got = Expert::user_authored(u(1), e("cost-analyst"), "成本分析师", "描述")
            .expect("四位签名仍应可用");
        assert_eq!(got.instructions(), "");
        assert_eq!(got.model(), None);
        assert_eq!(got.source_template(), None);
    }

    #[test]
    fn one_template_can_spawn_many_experts_and_each_remembers_its_own() {
        let r = registry();
        for (name, shown) in [("prog-1", "程序员1号"), ("prog-2", "程序员2号")] {
            let mut n = new(name);
            n.display_name = shown.into();
            n.source_template = Some("ai-coding-coach".into());
            r.create_user_expert(u(1), n).expect("派生专家应创建成功");
        }
        for name in ["prog-1", "prog-2"] {
            assert_eq!(
                r.get_visible(&u(1), &e(name))
                    .expect("应可读")
                    .source_template(),
                Some("ai-coding-coach"),
                "{name} 必须记住自己派生自 ai-coding-coach"
            );
        }

        let cleared = r
            .set_source_template(&u(1), &e("prog-1"), None)
            .expect("清除来源模板应成功");
        assert_eq!(cleared.source_template(), None, "None 必须表示不再来自模板");
        assert_eq!(
            r.get_visible(&u(1), &e("prog-2"))
                .expect("应可读")
                .source_template(),
            Some("ai-coding-coach"),
            "🔴 清 A 不得顺手把同模板的 B 也清了"
        );
    }

    #[test]
    fn source_template_follows_the_frozen_regex_and_nothing_else() {
        let r = registry();
        r.create_user_expert(u(1), new("prog-1")).expect("应创建");

        for ok in [
            "a",
            "ai-coding-coach",
            "tpl-1",
            &"a".repeat(MAX_SOURCE_TEMPLATE),
        ] {
            let got = r
                .set_source_template(&u(1), &e("prog-1"), Some(ok.to_string()))
                .unwrap_or_else(|e| panic!("合法模板 id {ok:?} 必须放行：{e}"));
            assert_eq!(got.source_template(), Some(ok), "取值不该被改写");
        }
        // 冻结契约就是 `^[a-z0-9-]{1,64}$`，比 ExpertId::parse 松（后者还禁首尾
        // 连字符与连续 --）。这里钉住这个差距，免得以后有人「顺手收紧」改坏契约。
        assert_eq!(
            r.set_source_template(&u(1), &e("prog-1"), Some("-lead".into()))
                .map(|e| e.source_template().map(str::to_string)),
            Ok(Some("-lead".into())),
            "契约正则允许首字符是连字符"
        );

        let too_long = "a".repeat(MAX_SOURCE_TEMPLATE + 1);
        for bad in [
            "",
            "  ai-coding-coach",
            "ai-coding-coach ",
            "AI-Coding",
            "ai_coding",
            "ai coding",
            "编程教练",
            too_long.as_str(),
        ] {
            let err = r
                .set_source_template(&u(1), &e("prog-1"), Some(bad.to_string()))
                .expect_err(&format!("不合法的模板 id {bad:?} 必须判红"));
            assert!(
                matches!(err, AgentError::ExpertSourceTemplateInvalid { .. }),
                "模板 id {bad:?} 必须判红：{err:?}"
            );
            assert!(
                err.to_string().contains("下一步"),
                "文案要带修复方向：{err}"
            );
        }
        assert_eq!(
            r.get_visible(&u(1), &e("prog-1"))
                .expect("应可取")
                .source_template(),
            Some("-lead"),
            "失败的写入不得改掉上一次成功写入的值"
        );
    }

    #[test]
    fn source_template_is_also_checked_on_create_and_obeys_the_owner_rules() {
        let r = registry();
        let mut n = new("prog-1");
        n.source_template = Some("ai_coding".into());
        let err = r.create_user_expert(u(1), n).unwrap_err();
        assert!(
            matches!(err, AgentError::ExpertSourceTemplateInvalid { .. }),
            "创建时也必须校验：{err:?}"
        );

        r.create_user_expert(u(1), new("prog-2")).expect("应创建");
        assert_eq!(
            r.set_source_template(&u(2), &e("prog-2"), Some("tpl-1".into()))
                .unwrap_err(),
            AgentError::ExpertNotFound { id: e("prog-2") }
        );
        r.create_builtin_expert(new("builtin-helper"))
            .expect("应创建");
        assert_eq!(
            r.set_source_template(&SYSTEM_OWNER, &e("builtin-helper"), Some("tpl-1".into()))
                .unwrap_err(),
            AgentError::ExpertBuiltinProtected {
                id: e("builtin-helper")
            },
            "内置专家受保护，改不了来源模板"
        );
    }
}
