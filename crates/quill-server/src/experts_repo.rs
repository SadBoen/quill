use std::collections::BTreeSet;
use std::sync::Arc;

use quill_adapters::{ExpertId, UserId};
use quill_agent::{AgentError, Expert, ExpertRegistry, ExpertRepository, Visibility};
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use crate::db::{col, digest32, now_ms, storage_error, user_id_from_blob, DbBridge};

pub const EXPERT_VERSION: &str = "0.1.0";

const OP_GET: &str = "读取专家";
const OP_LIST: &str = "列出专家";
const OP_PUT: &str = "写入专家";
const OP_ROSTER: &str = "读取专家名册";

const COLUMNS: &str = "id, owner_user_id, display_name, description, instructions, model, \
                       source_template, visibility, default_enabled, is_builtin, deleted_at";

#[derive(Debug, Clone)]
pub struct SqlxExpertRepository {
    db: Arc<DbBridge>,
}

impl SqlxExpertRepository {
    pub fn new(db: Arc<DbBridge>) -> Self {
        Self { db }
    }

    pub fn registry(db: Arc<DbBridge>) -> ExpertRegistry<Self> {
        ExpertRegistry::new(Self::new(db))
    }
}

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

pub(crate) const PUT_SQL: &str = "INSERT INTO experts (\
     id, owner_user_id, display_name, version, description, role_summary, \
     visibility, tool_policy_json, tags_json, license, default_enabled, is_builtin, \
     asset_hash, persona_hash, skill_count, created_at, updated_at, deleted_at, \
     instructions, model, source_template\
   ) VALUES (?, ?, ?, ?, ?, '', ?, '{}', '[]', '', ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?) \
   ON CONFLICT (owner_user_id, id) DO UPDATE SET \
     display_name = excluded.display_name, \
     description = excluded.description, \
     visibility = excluded.visibility, \
     default_enabled = excluded.default_enabled, \
     updated_at = excluded.updated_at, \
     deleted_at = excluded.deleted_at, \
     instructions = excluded.instructions, \
     model = excluded.model, \
     source_template = excluded.source_template";

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
    // persona_hash 口径：仍只对 id 取摘要，**不把 instructions 算进去**。
    // 实测踩过的坑：把人格正文混进摘要后，每次改人格都会改写这一列，而当前
    // 全仓库没有任何读取方消费它——只会在旧库上凭空产生一次无意义的行重写。
    // 等真有「人格指纹对账」需求时，再用一条新迁移把它切到内容摘要。
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
        .bind(expert.instructions())
        .bind(expert.model())
        .bind(expert.source_template())
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
                return Err(crate::db::invariant_broken(format!(
                    "experts 表里存在非法专家标识 {raw:?}（{e}）：\
                     表上的 CHECK 约束应已拦住它，请检查写入路径是否绕过了 STRICT/GLOB 约束"
                )));
            }
        }
    }
    Ok(out)
}

fn row_into_expert(row: &SqliteRow) -> Result<Expert, AgentError> {
    let raw_id: String = col!(row, String, "id", OP_GET);
    let owner_raw: Vec<u8> = col!(row, Vec<u8>, "owner_user_id", OP_GET);
    let display_name: String = col!(row, String, "display_name", OP_GET);
    let description: String = col!(row, String, "description", OP_GET);
    let instructions: String = col!(row, String, "instructions", OP_GET);
    let model: Option<String> = col!(row, Option<String>, "model", OP_GET);
    let source_template: Option<String> = col!(row, Option<String>, "source_template", OP_GET);
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
        Expert::builtin_with_source(id.clone(), display_name, description, instructions, model, source_template)?
    } else {
        if visibility != Visibility::UserAuthored {
            return Err(crate::db::invariant_broken(format!(
                "专家 {id} 不是内置专家但 visibility = {visibility}：\
                 自建专家只能是 user_authored（共享可见性由资源分发写入，不由本端口写）"
            )));
        }
        let mut e = Expert::user_authored_with_source(
            owner,
            id.clone(),
            display_name,
            description,
            instructions,
            model,
            source_template,
        )?;
        if default_enabled != 1 {
            e.set_default_enabled(&owner, false)?;
        }
        e
    };

    if deleted_at.is_some() {
        expert.soft_delete(&owner)?;
    }
    Ok(expert)
}

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

    #[test]
    fn put_sql_writes_and_rewrites_both_persona_columns() {
        let column_list = PUT_SQL
            .split(") VALUES")
            .next()
            .expect("语句必须含列清单段");
        for c in ["instructions", "model", "source_template"] {
            assert!(
                column_list.contains(c),
                "INSERT 必须显式写出 {c}（靠默认值插入会让人格静默丢失）：{PUT_SQL}"
            );
        }
        let update_set = PUT_SQL
            .split("DO UPDATE SET")
            .nth(1)
            .expect("语句必须含 DO UPDATE SET");
        for c in ["instructions", "model", "source_template"] {
            assert!(
                update_set.contains(&format!("{c} = excluded.{c}")),
                "PATCH 改人格必须真的落库，DO UPDATE 段缺 {c}：{update_set}"
            );
        }
        assert!(
            COLUMNS.contains("instructions")
                && COLUMNS.contains("model")
                && COLUMNS.contains("source_template"),
            "SELECT 必须读出三列：{COLUMNS}"
        );
    }

    /// persona_hash 的口径不能被顺手扩成「人格正文 + 来源模板一起摘要」：
    /// 那样每次 PATCH 都会重写这一列，而全仓库没有读取方消费它。
    #[test]
    fn persona_hash_still_only_digests_the_expert_id() {
        let update_set = PUT_SQL
            .split("DO UPDATE SET")
            .nth(1)
            .expect("语句必须含 DO UPDATE SET");
        assert!(
            !update_set.contains("persona_hash"),
            "摘要列不得在 upsert 里被改写（它只对 id 取摘要）：{update_set}"
        );
        assert!(
            update_set.contains("source_template = excluded.source_template"),
            "来源模板作为普通列更新：{update_set}"
        );
    }
}
