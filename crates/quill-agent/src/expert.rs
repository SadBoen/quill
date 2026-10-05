//! 专家领域逻辑：创建 / 更新 / 删除 / 列表 + 归属与可见性。
//!
//! # 存储为什么是 trait 而不是直接调 sqlx
//!
//! `docs/PHASE2_CONTRACT.md` §一的依赖图把 `quill-store` 放在
//! `quill-agent` 的**上一层**（store 是底层实现，agent 消费它）。
//! 但本 crate 的依赖表被约束为**只有** `quill-adapters` / `quill-domain` / `quill-wiki`
//! —— 引入 `sqlx` 会新增外部依赖（须主理人裁决，见任务约束「零新增外部 crate」）。
//!
//! 因此这里定义**端口 trait** [`ExpertRepository`]，由 `quill-server`
//! 用 `sqlx` 实现后注入。这样：
//! - 本 crate 保持零第三方依赖（`Cargo.lock` 不动）；
//! - 领域不变量（可见性、内置保护、软删除）**在本层**判定，
//!   不依赖 DB 的 CHECK 是否被正确写出来；
//! - 集成测试用内存实现，不需要真数据库。
//!
//! # 可见性口径（对齐 `crates/quill-store/migrations/0001_init.sql` 的 `experts` 表）
//!
//! 表里的 `visibility` 是 4 值 CHECK 约束：
//! `default_visible` / `manual_enable` / `builtin_system` / `user_authored`。
//! 另有一条**关键**的交叉 CHECK：
//!
//! ```sql
//! CHECK ((is_builtin = 1) = (owner_user_id = x'00000000000000000000000000000000'))
//! ```
//!
//! 即**内置专家的属主是全零 UserId**。本模块的 [`SYSTEM_OWNER`] 就是这条约束的
//! Rust 侧载体 —— 若不实现它，内置专家就能被普通用户认领（越权）。

use std::collections::BTreeSet;

use quill_adapters::{ExpertId, UserId};

use crate::error::AgentError;

/// 内置系统专家的属主标识（全零 UUID）。
///
/// 对应 `experts` 表的
/// `CHECK ((is_builtin = 1) = (owner_user_id = x'00…00'))`。
pub const SYSTEM_OWNER: UserId = UserId::from_bytes([0u8; 16]);

/// 专家显示名长度上限（对齐 schema 的 `length(display_name) BETWEEN 1 AND 64`）。
pub const MAX_DISPLAY_NAME: usize = 64;

/// 专家可见性（`experts.visibility` 的 4 个合法值）。
///
/// ⚠️ 线格式**必须**与 schema 的 CHECK 列表逐字一致，否则写库时会被 DB 判红
/// 而领域层却以为合法 —— 那是最难查的一类不一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Visibility {
    /// 默认可见：新用户开箱即用。
    DefaultVisible,
    /// 需手动启用：能看到但默认不参与派工。
    ManualEnable,
    /// 内置系统专家（仅 `SYSTEM_OWNER` 持有）。
    BuiltinSystem,
    /// 用户自建：仅创建者可见。
    UserAuthored,
}

impl Visibility {
    /// 线格式（写库 / 传输用）。
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::DefaultVisible => "default_visible",
            Self::ManualEnable => "manual_enable",
            Self::BuiltinSystem => "builtin_system",
            Self::UserAuthored => "user_authored",
        }
    }

    /// 由线格式解析。
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "default_visible" => Some(Self::DefaultVisible),
            "manual_enable" => Some(Self::ManualEnable),
            "builtin_system" => Some(Self::BuiltinSystem),
            "user_authored" => Some(Self::UserAuthored),
            _ => None,
        }
    }

    /// 4 个变体与 4 个线名必须**一一对应**（防止有人加了变体忘了加线名）。
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

/// 专家实体（`experts` 表的领域投影）。
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
    /// 新建专家。**内置专家必须走 [`Expert::builtin`]**。
    ///
    /// 不变量：
    /// 1. `display_name` 非空且不超过 [`MAX_DISPLAY_NAME`]；
    /// 2. 用户自建专家的可见性**只能是** `UserAuthored` —— 让调用方
    ///    传 `BuiltinSystem` 会与 `is_builtin` 的交叉 CHECK 直接冲突。
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
            // ⚠️ 反向不变量：全零属主在 DB 侧等价于 is_builtin=1，
            // 这里若放过，用户就能造出一个「假装是内置」的专家。
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

    /// 新建内置系统专家（属主固定为 [`SYSTEM_OWNER`]，不可默认关闭）。
    ///
    /// ⚠️ 内置专家的 `default_enabled` 恒为 `true`：它随版本发布，
    /// 「默认关闭的内置专家」等于一条谁都看不到的死数据。
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

    /// 专家标识。
    pub fn id(&self) -> &ExpertId {
        &self.id
    }

    /// 属主。
    pub fn owner(&self) -> UserId {
        self.owner
    }

    /// 显示名。
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// 描述。
    pub fn description(&self) -> &str {
        &self.description
    }

    /// 可见性。
    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    /// 是否默认启用（`default_enabled`）。
    pub fn default_enabled(&self) -> bool {
        self.default_enabled
    }

    /// 是否内置（`is_builtin`）。
    pub fn is_builtin(&self) -> bool {
        self.builtin
    }

    /// 是否已软删除（`deleted_at IS NOT NULL`）。
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    /// 某用户**是否看得见**这个专家。
    ///
    /// 口径（对齐 `PHASE2_CONTRACT` §5.1「按用户可见性过滤」）：
    /// | 可见性 | 可见范围 |
    /// |---|---|
    /// | `builtin_system` | 所有用户 |
    /// | `default_visible` | 所有用户 |
    /// | `manual_enable` | 所有用户（可见但默认不启用） |
    /// | `user_authored` | **仅属主** |
    ///
    /// ⚠️ 已软删除的专家对**所有人**不可见 —— 包括属主。
    /// 「删除后仍能通过列表看到自己删的专家」会让用户以为没删掉。
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

    /// 某用户**是否可修改 / 删除**（仅创建者；内置专家谁都不行）。
    ///
    /// ⚠️ 内置专家**没有人类属主**，所以 `owner == SYSTEM_OWNER` 这个判断
    /// 天然让所有真实用户都改不了它 —— 不需要额外的 `is_builtin` 分支。
    pub fn is_modifiable_by(&self, actor: &UserId) -> bool {
        !self.builtin && !self.deleted && *actor == self.owner
    }

    /// 改显示名。**非属主 / 内置 / 已删一律判红。**
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

    /// 改描述。
    pub fn redescribe(
        &mut self,
        actor: &UserId,
        description: impl Into<String>,
    ) -> Result<(), AgentError> {
        self.assert_modifiable(actor)?;
        self.description = description.into();
        Ok(())
    }

    /// 改默认启用开关。
    pub fn set_default_enabled(&mut self, actor: &UserId, on: bool) -> Result<(), AgentError> {
        self.assert_modifiable(actor)?;
        // ⚠️ 反向不变量：内置专家不可被关掉。
        // 少了这一条，内置专家能被用户悄悄停用而不留痕迹。
        if self.builtin && !on {
            return Err(AgentError::ExpertBuiltinProtected {
                id: self.id.clone(),
            });
        }
        self.default_enabled = on;
        Ok(())
    }

    /// 软删除（`deleted_at`）。**幂等**：已删再删返回 `Ok(false)`。
    ///
    /// 返回 `bool` 而非 `Result` 的原因与 `Team::remove_member` 相同：
    /// 「本来就不在」与「删除失败」语义不同，混成一个错误会让调用方
    /// 无法区分幂等重试与真实故障。
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

    /// 交叉不变量自检：`is_builtin ⇔ owner == SYSTEM_OWNER`。
    ///
    /// 这是 `experts` 表那条 CHECK 的 Rust 侧镜像。
    /// ⚠️ 它之所以必要：表里那条 CHECK 只在**写库那一刻**生效，
    /// 而内存实现（测试用）没有 DB 兜底 —— 不显式校验就等于
    /// 「测试全绿但生产写库失败」。
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

/// 显示名校验：非空、不超长。
fn check_display_name(raw: &str) -> Result<(), AgentError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AgentError::ExpertDisplayNameInvalid {
            raw: raw.to_string(),
            reason: "不能为空或全空白",
        });
    }
    // ⚠️ 按**字符**数而不是字节数：中文名 21 字 = 63 字节，
    // 按字节算会把合法的中文名误判成超长。
    let chars = trimmed.chars().count();
    if chars > MAX_DISPLAY_NAME {
        return Err(AgentError::ExpertDisplayNameInvalid {
            raw: raw.to_string(),
            reason: "超过 64 个字符",
        });
    }
    Ok(())
}

/// 新建专家的入参。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewExpert {
    /// 专家标识（kebab-case，由调用方用 `ExpertId::parse` 校验）。
    pub id: ExpertId,
    /// 显示名。
    pub display_name: String,
    /// 描述。
    pub description: String,
}

/// 专家存储端口（由 `quill-server` 用 `sqlx` 实现）。
///
/// ⚠️ **方法全部带 `owner` 参数**：`experts` 的主键是复合键
/// `(owner_user_id, id)`，而**只按 id 查**就等于放弃了跨用户隔离
/// （`V1_SCOPE_CONSTRAINTS.md` §五「多用户隔离 = 最高优先级」）。
/// 类型上带 `UserId` 是最便宜的强制手段 —— 漏传在编译期就断掉。
pub trait ExpertRepository: Send + Sync + 'static {
    /// 按 `(owner, id)` 取专家。已软删除的**也返回**（由调用方判可见性）。
    fn get(&self, owner: &UserId, id: &ExpertId) -> Result<Option<Expert>, AgentError>;

    /// 列出某属主的全部专家（含已软删除）。
    fn list_owned(&self, owner: &UserId) -> Result<Vec<Expert>, AgentError>;

    /// 写入（upsert）。
    ///
    /// ⚠️ **这是 upsert 而不是 insert**：软删除后再用同名重建，
    /// 在 `experts` 的复合主键 `(owner_user_id, id)` 上是**同一行**，
    /// 真库实现必须用 `INSERT … ON CONFLICT (owner_user_id, id) DO UPDATE`。
    /// 若把它实现成纯 `INSERT`，「删除后重建同名专家」会在写库那一刻失败，
    /// 而领域层已经返回了成功 —— 一类「层间不一致且静默」的 bug。
    ///
    /// 唯一性由 [`ExpertRegistry::create_user_expert`] 的先查后建保证，
    /// 不靠本方法（见那里的注释）。
    fn put(&self, expert: &Expert) -> Result<(), AgentError>;

    /// 名册：某用户当前**可见且未删**的专家标识集合。
    ///
    /// 存在的理由：`quill_domain::Team::add_member` 要求传入名册，
    /// 而名册的过滤口径（可见性 + 软删除）必须与列表口径**完全一致** ——
    /// 两套口径就会让「列表里选得到的专家却加不进团」。
    fn roster(&self, viewer: &UserId) -> Result<BTreeSet<ExpertId>, AgentError>;
}

/// 专家注册表：领域逻辑 + 存储端口。
///
/// 这是 HTTP 层的 `POST /api/experts` / `PATCH` / `DELETE` / `GET` 背后的编排体。
#[derive(Debug)]
pub struct ExpertRegistry<R: ExpertRepository> {
    repo: R,
}

impl<R: ExpertRepository> ExpertRegistry<R> {
    /// 组装注册表。
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    /// 只读访问底层仓库（供上层做联表查询）。
    pub fn repo(&self) -> &R {
        &self.repo
    }

    /// 创建用户自建专家。
    ///
    /// 判红顺序**刻意**是「先查重再建」：让 DB 的唯一索引当第一道防线是
    /// 双重保险，但用户看到的第一条错误必须是中文的「已存在」。
    ///
    /// ⚠️ **已软删除的同名行不算「已存在」**：软删除释放名字，
    /// 与 `TaskDispatch` 账本的 `begin()` 返回既有记录是两种不同语义 ——
    /// 专家名是**用户资产**（要能复用），派工键是**幂等凭证**（要能识别）。
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

    /// 创建内置专家（仅系统属主）。
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

    /// 取一个**对该用户可见**的专家。
    ///
    /// ⚠️ 不可见时返回 [`AgentError::ExpertNotFound`] 而**不是** `Forbidden`：
    /// 见 [`Expert::is_visible_to`] 的注释（避免跨用户枚举）。
    pub fn get_visible(&self, viewer: &UserId, id: &ExpertId) -> Result<Expert, AgentError> {
        // 先看属主自己的那份：user_authored 专家只在这里。
        if let Some(e) = self.repo.get(viewer, id)? {
            if e.is_visible_to(viewer) {
                return Ok(e);
            }
            return Err(AgentError::ExpertNotFound { id: id.clone() });
        }
        // 再看内置 / 共享的那份（属主是 SYSTEM_OWNER）。
        if let Some(e) = self.repo.get(&SYSTEM_OWNER, id)? {
            if e.is_visible_to(viewer) {
                return Ok(e);
            }
        }
        Err(AgentError::ExpertNotFound { id: id.clone() })
    }

    /// 列出某用户可见的专家（按 `ExpertId` 序，保证可复现）。
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
        // ⚠️ 同一 id 可能既在用户表又在系统表里可见（用户自建了一个
        // 与内置同名的专家）。去重后取**用户那份**（先 push 的那份），
        // 否则列表里会出现两条同名专家，用户会以为建了两次。
        out.dedup_by(|a, b| a.id == b.id);
        Ok(out)
    }

    /// 改名。
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

    /// 改描述。
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

    /// 改默认启用开关。
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

    /// 软删除。返回是否真的删了（幂等：`false` = 本来就已删）。
    ///
    /// ⚠️ **幂等检查必须在 `must_get_owned` 之前**：
    /// 后者对已删行返回 `ExpertDeleted`（改名时该报错），
    /// 而删除的语义是「重复删除返回 `false`」—— 与 `Team::remove_member`
    /// 的口径一致。顺序反了就会让「重试删除」变成「删除失败」。
    pub fn delete(&self, actor: &UserId, id: &ExpertId) -> Result<bool, AgentError> {
        if let Some(e) = self.repo.get(actor, id)? {
            // ⚠️ 内置检查同样排在最前：理由同 `must_get_owned`。
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
        // 系统命名空间：内置专家（与 `must_get_owned` 同口径）。
        if let Some(e) = self.repo.get(&SYSTEM_OWNER, id)? {
            if e.deleted {
                return Ok(false);
            }
            return Err(AgentError::ExpertBuiltinProtected { id: id.clone() });
        }
        Err(AgentError::ExpertNotFound { id: id.clone() })
    }

    /// 取「可修改」的那份专家。
    ///
    /// # 为什么越权时报 `ExpertNotFound` 而不是 `ExpertNotModifiable`
    ///
    /// `experts` 的主键是 `(owner_user_id, id)`。要判定「B 改 A 的专家」，
    /// 就得**按 id 扫所有属主的行** —— 那本身就是一个跨用户枚举接口
    /// （B 能靠报错差异枚举 A 的私有专家名）。
    /// 因此这里只在**两个命名空间**里找：actor 自己的 + 系统的。
    /// 找不到就是「不存在或不可见」，两种情况都不泄漏。
    ///
    /// ⚠️ 由此产生的一个后果：`AgentError::ExpertNotModifiable`
    /// 在**本方法**里不可达（能在这里取出的行，归属必然正确）。
    /// 它仍然可达 —— `Expert::rename` 等领域方法会经 `assert_modifiable` 产出它，
    /// 而那些方法是 `pub` 的（调用方可能持有从别处取到的 `Expert` 值对象）。
    fn must_get_owned(&self, actor: &UserId, id: &ExpertId) -> Result<Expert, AgentError> {
        if let Some(e) = self.repo.get(actor, id)? {
            // ⚠️ **内置检查必须排在归属检查之前**：内置专家的「属主」是
            // 全零 `SYSTEM_OWNER`，若用 `actor == owner` 判定，
            // 以 `SYSTEM_OWNER` 身份操作会走进「归属正确」分支，
            // 然后被 `is_modifiable_by` 的 `!builtin` 拦下，
            // 报出「你不是创建者」—— 一个对用户毫无意义、且会误导排障的错。
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
        // 系统命名空间：内置专家在这里。它对所有人可见，
        // 因此「这是内置专家你改不了」不泄漏任何私有信息，可以直说。
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

    /// 内存实现：复合主键 `(owner, id)`。
    ///
    /// ⚠️ 它**刻意复刻**了 DB 的唯一约束语义（同名同属主不可重复），
    /// 否则「唯一索引能不能拦住重复」这件事在测试里根本没被验证。
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
            // ⚠️ upsert 语义：软删除后同名重建是**同一行**，
            // 与真库 `ON CONFLICT (owner_user_id, id) DO UPDATE` 一致。
            // 唯一性由 `create_user_expert` 的先查后建保证，不靠这里。
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

    // ── 创建 ──

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
        // 反向用例：复合主键冲突必须判红。
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
        // 🔴 反向用例：主键是 (owner, id) 而不是单 id。
        // 若误改成单列主键，A 用户就再也建不出同名的专家了。
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
        // 反向用例（长度按字符不按字节）：64 个汉字 = 192 字节，
        // 按字节算会被误判超长。
        let r = registry();
        let mut n = new("cost-analyst");
        n.display_name = "成".repeat(64);
        let got = r.create_user_expert(u(1), n).expect("64 个汉字应合法");
        assert_eq!(got.display_name().chars().count(), 64);
    }

    // ── 内置专家保护 ──

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
        // 反向用例：关掉内置专家 = 一条谁都看不到的死数据，且不留痕迹。
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
        // 🔴 反向用例：全零属主在 DB 侧等价于 is_builtin=1，
        // 放过它就等于让用户造出「假装内置」的专家。
        let got = Expert::user_authored(SYSTEM_OWNER, e("fake"), "假内置", "描述");
        assert_eq!(
            got.unwrap_err(),
            AgentError::ExpertBuiltinProtected { id: e("fake") }
        );
    }

    #[test]
    fn cross_invariant_holds_for_both_construction_paths_and_detects_tampering() {
        // 正向：两条构造路径都必须自洽。
        Expert::builtin(e("b"), "b", "d")
            .expect("内置应自洽")
            .check_cross_invariant()
            .expect("内置专家交叉不变量应成立");
        Expert::user_authored(u(7), e("u"), "u", "d")
            .expect("自建应自洽")
            .check_cross_invariant()
            .expect("自建专家交叉不变量应成立");

        // 反向：被篡改的实体（builtin=true 但属主不是系统）必须被检出。
        // 这条对应「测试内存实现没有 DB CHECK 兜底」的风险。
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

    // ── 可见性 ──

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
        // 🔴 反向用例：Forbidden 会让 B 用户靠报错差异枚举 A 的专家名。
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
        // 反向用例：用户自建一个与内置同名的专家，列表里不该出现两条。
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
        // 名册与列表口径必须一致，否则「列表里选得到的专家却加不进团」。
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

    // ── 更新 ──

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
        // 🔴 反向用例：报 `ExpertNotModifiable` 等于确认「A 确实有这个专家」，
        // 那就是跨用户枚举接口。要判定「非属主」就得按 id 扫所有属主的行。
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
        // 🔴 与上一条相反的口径：内置专家对**所有人可见**，
        // 所以说「它是内置的、你改不了」不泄漏任何私有信息，可以直说。
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
        // 反向用例：失败的更新不得留下痕迹。
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

    // ── 删除 ──

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
        // 幂等语义：二次删除返回 false 而非 Err（区别于「删除失败」）。
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
        // 口径同 `non_owner_rename_reports_not_found_not_modifiable`：
        // 不确认「A 确实有这个专家」。
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
        // 软删除语义：名字被释放，用户可以重建同名专家。
        let r = registry();
        r.create_user_expert(u(1), new("cost-analyst"))
            .expect("首次应成功");
        r.delete(&u(1), &e("cost-analyst")).expect("应删成功");
        let again = r
            .create_user_expert(u(1), new("cost-analyst"))
            .expect("删掉后同名应可重建");
        assert!(!again.is_deleted());
    }

    // ── 线格式 ──

    #[test]
    fn visibility_wire_names_match_the_schema_check_list_exactly() {
        // ⚠️ 逐字对齐 `experts.visibility` 的 CHECK：
        // 任一改名都会让写库当场失败，而领域层却以为合法。
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
        // 反向用例：未知值若落进 DefaultVisible，就会把私有专家公开。
        for bad in ["", "public", "Default_Visible", "builtin"] {
            assert_eq!(Visibility::from_wire(bad), None, "{bad:?} 必须解析失败");
        }
    }
}
