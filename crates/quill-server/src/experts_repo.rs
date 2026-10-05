//! `ExpertRepository` 的 sqlx 实现（表 `experts`）。
//!
//! # 跨用户隔离怎么在这一层保证
//!
//! `experts` 的主键是**复合键** `(owner_user_id, id)`，因此：
//!
//! - [`SqlxExpertRepository::get`] / `put` 的每条语句都带 `owner_user_id = ?`；
//! - [`SqlxExpertRepository::list_owned`] 的 `WHERE` 只有 `owner_user_id = ?`；
//! - [`SqlxExpertRepository::roster`] 是唯一「跨属主扫描」的方法，它的 `WHERE`
//!   把可见性写进 SQL：`deleted_at IS NULL AND (visibility IN (…) OR owner_user_id = ?)`。
//!
//! ⚠️ **不存在「先查后判」**：可见性判定由 SQL 的 `WHERE` 完成，
//! Rust 侧只对**已经取回的行**做领域判定。若改成「取全表再在 Rust 里过滤」，
//! 一次疏忽就会把别人的私有专家名带进内存 —— 那正是本项目最贵的 bug 类型。
//!
//! # `put` 必须是 upsert（不是 insert）
//!
//! 软删除后再用同名重建，在复合主键上**是同一行**。纯 `INSERT` 会让
//! 「删掉再重建」在写库那一刻失败，而领域层（`ExpertRegistry::create_user_expert`）
//! 已经返回成功 —— 层间不一致且静默。故此处固定
//! `INSERT … ON CONFLICT (owner_user_id, id) DO UPDATE`。
//!
//! # 读回一个 `Expert` 为什么不能「直接 new 出来」
//!
//! `Expert` 的字段是私有的，公开构造器只有 [`Expert::user_authored`] 与
//! [`Expert::builtin`]，两者都给出**固定初始状态**。而库里的一行可能是
//! 「自建 + 已停用 + 已软删」这样的组合。因此本实现按行状态**重放领域跃迁**
//! （先 `set_default_enabled`、再 `soft_delete`）把值还原出来，
//! 而不是绕过领域层直接造值 —— 后者会让「写库时的不变量」与「读库时的假设」
//! 各成一份真相源。
//!
//! # 领域投影没覆盖的列
//!
//! `version` / `license` / `tool_policy_json` / `tags_json` / `asset_hash` /
//! `persona_hash` / `skill_count` **不在** `Expert` 里，但表里是 `NOT NULL`。
//! 处理口径（逐条写明，便于后人接手时知道哪些是临时的）：
//!
//! | 列 | 本实现写入 | 理由 |
//! |---|---|---|
//! | `version` | `0.1.0` | 领域投影无版本概念；资源包导入（`/api/experts/import`）落地后应由它覆盖 |
//! | `license` | 空串 | 表上无 CHECK；空串表示「未声明」，由导入路径补齐 |
//! | `tool_policy_json` | `{}` | 满足 `length >= 2`；工具白名单由 `quill-ext-hub` 侧管理 |
//! | `tags_json` | `[]` | 同上 |
//! | `asset_hash` / `persona_hash` | `digest32(标签, id)` 派生占位 | 🔴 **已知缺口**：真实值应由资源管线按内容计算。本实现派生一个**稳定**值（不是随机），保证重复写入不会让摘要漂移 |
//! | `skill_count` | `0` | 领域投影无技能数 |

use std::collections::BTreeSet;
use std::sync::Arc;

use quill_adapters::{ExpertId, UserId};
use quill_agent::{AgentError, Expert, ExpertRegistry, ExpertRepository, Visibility};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::db::{col, digest32, now_ms, storage_error, user_id_from_blob, DbBridge};

/// 写入 `version` 列的常量值。
pub const EXPERT_VERSION: &str = "0.1.0";

/// 读列用的固定文案（错误里必须点名「读专家」而不是笼统的「查询失败」）。
const OP_GET: &str = "读取专家";
const OP_LIST: &str = "列出专家";
const OP_PUT: &str = "写入专家";
const OP_ROSTER: &str = "读取专家名册";

/// `experts` 的读列清单。
///
/// ⚠️ 显式列清单而不是 `SELECT *`：表一改列顺序，投影就静默错位
/// （比如把 `description` 读成 `role_summary`，编译期与运行期都不报错）。
const COLUMNS: &str = "id, owner_user_id, display_name, description, visibility, \
                       default_enabled, is_builtin, deleted_at";

/// `ExpertRepository` 的真库实现。
#[derive(Debug, Clone)]
pub struct SqlxExpertRepository {
    db: Arc<DbBridge>,
}

impl SqlxExpertRepository {
    /// 组装仓库。
    pub fn new(db: Arc<DbBridge>) -> Self {
        Self { db }
    }

    /// 便捷构造：直接给出注册表（HTTP 层的编排体）。
    pub fn registry(db: Arc<DbBridge>) -> ExpertRegistry<Self> {
        ExpertRegistry::new(Self::new(db))
    }
}

// ─────────────────────────── SQL ───────────────────────────

async fn sql_get(
    pool: &sqlx::SqlitePool,
    owner: UserId,
    id: String,
) -> Result<Option<Expert>, AgentError> {
    let sql = format!("SELECT {COLUMNS} FROM experts WHERE owner_user_id = ? AND id = ?");
    let row = sqlx::query(&sql)
        .bind(crate::db::blob_of(&owner))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| storage_error(OP_GET, e))?;
    match row {
        Some(r) => Ok(Some(row_into_expert(&r)?)),
        None => Ok(None),
    }
}

async fn sql_list_owned(pool: &sqlx::SqlitePool, owner: UserId) -> Result<Vec<Expert>, AgentError> {
    let sql = format!("SELECT {COLUMNS} FROM experts WHERE owner_user_id = ? ORDER BY id ASC");
    let rows = sqlx::query(&sql)
        .bind(crate::db::blob_of(&owner))
        .fetch_all(pool)
        .await
        .map_err(|e| storage_error(OP_LIST, e))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(row_into_expert(r)?);
    }
    Ok(out)
}

/// `put` 用的 upsert 语句。
///
/// ⚠️ `ON CONFLICT (owner_user_id, id)`：冲突目标必须与表的主键**逐字**一致。
/// DO UPDATE 段**刻意不更新** `is_builtin` / `owner_user_id` / `created_at`：
/// 前两者是交叉不变量的一边（改了就是越权），第三者是原始创建时间。
pub(crate) const PUT_SQL: &str = "INSERT INTO experts (\
     id, owner_user_id, display_name, version, description, role_summary, \
     visibility, tool_policy_json, tags_json, license, default_enabled, is_builtin, \
     asset_hash, persona_hash, skill_count, created_at, updated_at, deleted_at\
   ) VALUES (?, ?, ?, ?, ?, '', ?, '{}', '[]', '', ?, ?, ?, ?, 0, ?, ?, ?) \
   ON CONFLICT (owner_user_id, id) DO UPDATE SET \
     display_name = excluded.display_name, \
     description = excluded.description, \
     visibility = excluded.visibility, \
     default_enabled = excluded.default_enabled, \
     updated_at = excluded.updated_at, \
     deleted_at = excluded.deleted_at";

/// `roster` 用的可见性查询。
///
/// ⚠️ 可见性写进 SQL：三个共享可见性值 + 「自己的」。
/// 若改成「取全表在 Rust 里过滤」，别人的私有专家名就会进内存。
pub(crate) const ROSTER_SQL: &str = "SELECT id FROM experts \
     WHERE deleted_at IS NULL \
       AND (visibility IN ('default_visible', 'manual_enable', 'builtin_system') \
            OR owner_user_id = ?) \
     ORDER BY id ASC";

async fn sql_put(pool: &sqlx::SqlitePool, expert: Expert) -> Result<(), AgentError> {
    let owner = expert.owner();
    let id = expert.id().as_str().to_string();
    let now = now_ms();
    let deleted_at: Option<i64> = if expert.is_deleted() { Some(now) } else { None };
    let asset_hash = digest32("experts.asset_hash", &[id.as_bytes()]);
    let persona_hash = digest32("experts.persona_hash", &[id.as_bytes()]);

    sqlx::query(PUT_SQL)
        .bind(id)
        .bind(crate::db::blob_of(&owner))
        .bind(expert.display_name())
        .bind(EXPERT_VERSION)
        .bind(expert.description())
        .bind(expert.visibility().as_wire())
        .bind(i64::from(expert.default_enabled()))
        .bind(i64::from(expert.is_builtin()))
        .bind(asset_hash.to_vec())
        .bind(persona_hash.to_vec())
        .bind(now)
        .bind(now)
        .bind(deleted_at)
        .execute(pool)
        .await
        .map_err(|e| storage_error(OP_PUT, e))?;
    Ok(())
}

async fn sql_roster(
    pool: &sqlx::SqlitePool,
    viewer: UserId,
) -> Result<BTreeSet<ExpertId>, AgentError> {
    let rows = sqlx::query(ROSTER_SQL)
        .bind(crate::db::blob_of(&viewer))
        .fetch_all(pool)
        .await
        .map_err(|e| storage_error(OP_ROSTER, e))?;
    let mut out = BTreeSet::new();
    for r in &rows {
        let raw: String = r.try_get("id").map_err(|e| storage_error(OP_ROSTER, e))?;
        match ExpertId::parse(&raw) {
            Ok(id) => {
                out.insert(id);
            }
            Err(e) => {
                // ⚠️ 库里出现非法 slug 时**不静默跳过**：表上有 GLOB CHECK 兜底，
                //    走到这里说明 CHECK 被绕过（PRAGMA 或写入路径不对），必须报出来。
                return Err(crate::db::invariant_broken(format!(
                    "experts 表里存在非法专家标识 {raw:?}（{e}）：\
                     表上的 CHECK 约束应已拦住它，请检查写入路径是否绕过了 STRICT/GLOB 约束"
                )));
            }
        }
    }
    Ok(out)
}

// ─────────────────────────── 行 → 领域值 ───────────────────────────

fn row_into_expert(row: &SqliteRow) -> Result<Expert, AgentError> {
    let raw_id: String = col!(row, String, "id", OP_GET);
    let owner_raw: Vec<u8> = col!(row, Vec<u8>, "owner_user_id", OP_GET);
    let display_name: String = col!(row, String, "display_name", OP_GET);
    let description: String = col!(row, String, "description", OP_GET);
    let visibility_raw: String = col!(row, String, "visibility", OP_GET);
    let default_enabled: i64 = col!(row, i64, "default_enabled", OP_GET);
    let is_builtin: i64 = col!(row, i64, "is_builtin", OP_GET);
    let deleted_at: Option<i64> = col!(row, Option<i64>, "deleted_at", OP_GET);

    let id = ExpertId::parse(&raw_id).map_err(|e| {
        crate::db::invariant_broken(format!("experts.id = {raw_id:?} 不是合法 slug（{e}）"))
    })?;
    let owner = user_id_from_blob(&owner_raw).ok_or_else(|| {
        crate::db::invariant_broken(format!(
            "experts.owner_user_id 长度是 {} 字节，应为 16 字节",
            owner_raw.len()
        ))
    })?;
    let visibility = Visibility::from_wire(&visibility_raw).ok_or_else(|| {
        crate::db::invariant_broken(format!(
            "experts.visibility = {visibility_raw:?} 不在 schema 的 4 个合法值内"
        ))
    })?;

    let mut expert = if is_builtin == 1 {
        // ⚠️ 内置行的三个状态都必须与「内置专家不可停用、不可删」这条领域不变量一致。
        //    出现不一致说明有写入路径绕过了 `Expert::set_default_enabled`。
        if visibility != Visibility::BuiltinSystem {
            return Err(crate::db::invariant_broken(format!(
                "专家 {id} 是内置专家（is_builtin=1）但 visibility = {visibility}，\
                 内置专家只能是 builtin_system"
            )));
        }
        if default_enabled != 1 {
            return Err(crate::db::invariant_broken(format!(
                "专家 {id} 是内置专家但 default_enabled = 0：内置专家不可被停用"
            )));
        }
        if deleted_at.is_some() {
            return Err(crate::db::invariant_broken(format!(
                "专家 {id} 是内置专家但已被软删除：内置专家受保护，不可删除"
            )));
        }
        Expert::builtin(id.clone(), display_name, description)?
    } else {
        if visibility != Visibility::UserAuthored {
            return Err(crate::db::invariant_broken(format!(
                "专家 {id} 不是内置专家但 visibility = {visibility}：\
                 自建专家只能是 user_authored（共享可见性由资源分发写入，不由本端口写）"
            )));
        }
        let mut e = Expert::user_authored(owner, id.clone(), display_name, description)?;
        if default_enabled != 1 {
            // ⚠️ 用领域方法而不是直接改字段：这样「自建专家可被停用」这条不变量
            //    仍由 `Expert::set_default_enabled` 判定，读路径不会绕过它。
            e.set_default_enabled(&owner, false)?;
        }
        e
    };

    if deleted_at.is_some() {
        // ⚠️ 只有非内置行能走到这里（内置行已在上方判红）。
        //    `soft_delete` 幂等且要求 actor 是属主，actor 传 owner 天然满足。
        expert.soft_delete(&owner)?;
    }
    Ok(expert)
}

// ─────────────────────────── 端口实现 ───────────────────────────

impl ExpertRepository for SqlxExpertRepository {
    fn get(&self, owner: &UserId, id: &ExpertId) -> Result<Option<Expert>, AgentError> {
        let owner = *owner;
        let id = id.as_str().to_string();
        self.db
            .call(move |pool, _rt| Box::pin(async move { sql_get(&pool, owner, id).await }))
    }

    fn list_owned(&self, owner: &UserId) -> Result<Vec<Expert>, AgentError> {
        let owner = *owner;
        self.db
            .call(move |pool, _rt| Box::pin(async move { sql_list_owned(&pool, owner).await }))
    }

    fn put(&self, expert: &Expert) -> Result<(), AgentError> {
        let owned = expert.clone();
        self.db
            .call(move |pool, _rt| Box::pin(async move { sql_put(&pool, owned).await }))
    }

    fn roster(&self, viewer: &UserId) -> Result<BTreeSet<ExpertId>, AgentError> {
        let viewer = *viewer;
        self.db
            .call(move |pool, _rt| Box::pin(async move { sql_roster(&pool, viewer).await }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 断言**生产代码真正使用的那条语句**（`ROSTER_SQL` 常量本身），
    /// 而不是测试里另抄一份 —— 另抄一份的断言会在有人改实现时继续绿，
    /// 那正是 AGENTS.md 说的第 7 类失效（恒绿）。
    #[test]
    fn roster_sql_keeps_visibility_and_owner_in_the_where_clause() {
        assert!(
            ROSTER_SQL.contains("deleted_at IS NULL"),
            "软删行必须被排除：{ROSTER_SQL}"
        );
        assert!(
            ROSTER_SQL.contains("owner_user_id = ?"),
            "隔离谓词必须在 SQL 里：{ROSTER_SQL}"
        );
        assert!(
            !ROSTER_SQL.contains("SELECT *"),
            "必须显式列清单（SELECT * 会让列顺序变化静默错位）"
        );
    }

    /// 锁住 `put` 的两条关键形态：upsert 目标 + 不许覆盖的不变量列。
    #[test]
    fn put_sql_is_upsert_and_never_rewrites_the_builtin_invariant() {
        assert!(
            PUT_SQL.contains("ON CONFLICT (owner_user_id, id) DO UPDATE"),
            "软删后同名重建是同一行，必须 upsert：{PUT_SQL}"
        );
        let update_set = PUT_SQL
            .split("DO UPDATE SET")
            .nth(1)
            .expect("语句必须含 DO UPDATE SET");
        for forbidden in ["is_builtin", "owner_user_id =", "created_at ="] {
            assert!(
                !update_set.contains(forbidden),
                "DO UPDATE 段不得改写 {forbidden}（不变量 / 原始创建时间）：{update_set}"
            );
        }
        assert!(
            update_set.contains("deleted_at = excluded.deleted_at"),
            "必须能通过 upsert 复活软删行（deleted_at → NULL）：{update_set}"
        );
    }
}
