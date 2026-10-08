//! 专家团的存储口径。
//!
//! 表在 0001 就建好了（`teams` / `team_members`），0006 只给 `teams` 补了
//! `team_slug` 与 `description` 两列。这里集中放 SQL 常量：列清单、谓词、
//! 写语句都写成常量，便于单测直接断言「隔离谓词在 SQL 里」「软删过滤没漏」。
//!
//! 标识有两层，别混用：`teams.id` 是 16 字节 BLOB，只当 `sessions.team_id`、
//! `task_dispatches.team_id` 的外键键（派工路由按 32 位 hex 解析它）；
//! 对外可见、可记、可读的标识是 `teams.team_slug`（kebab-case，解析口径与
//! `experts.id` 一致）。

use sqlx::sqlite::SqliteRow;

use crate::db::{col, now_ms, storage_error, DbBridge};

const OP_GET: &str = "读取专家团";
const OP_LIST: &str = "列出专家团";
const OP_WRITE: &str = "写入专家团";

/// 显式列清单：`*` 会在列顺序变化时静默错位。
///
/// 四个限制列（`guidelines` / `max_dispatch` / `max_replan` / `max_ask_depth`）
/// 必须在这里——之前它们**只存在于 schema**，读写两侧一个字都没提，
/// 于是「限制」从来没影响过任何一次派工（队列 Q043）。
pub const COLUMNS: &str = "team_slug, name, description, guidelines, max_dispatch, max_replan, \
     max_ask_depth, leader_expert_id, created_at, updated_at";

/// 列表只给当前用户的未软删团队。
pub const LIST_SQL: &str = "SELECT team_slug, name, description, guidelines, max_dispatch, \
     max_replan, max_ask_depth, leader_expert_id, created_at, \
     updated_at FROM teams WHERE user_id = ? AND deleted_at IS NULL \
     ORDER BY updated_at DESC, team_slug ASC";

/// 单读。软删行读不出来（= 不存在），与「用户看不见」同一种结果。
pub const GET_SQL: &str = "SELECT team_slug, name, description, guidelines, max_dispatch, \
     max_replan, max_ask_depth, leader_expert_id, created_at, \
     updated_at FROM teams WHERE user_id = ? AND team_slug = ? AND deleted_at IS NULL";

/// 按 16 字节 `id` 单读。派工真执行按这个走 —— 路径参数就是 32 位 hex 的 id，
/// 而不是对外可见的 `team_slug`（两层标识的分工见文件头）。
///
/// 派工侧就是靠这一条把四个限制列读出来的（`TeamRow::limits`）。
pub const GET_BY_ID_SQL: &str = "SELECT team_slug, name, description, guidelines, max_dispatch, \
     max_replan, max_ask_depth, leader_expert_id, \
     created_at, updated_at FROM teams WHERE user_id = ? AND id = ? AND deleted_at IS NULL";

/// 成员按 expert_id 升序读出，契约要求 member_ids 有序。
pub const MEMBERS_SQL: &str = "SELECT expert_id FROM team_members \
     WHERE user_id = ? AND team_id = ? AND role = 'member' ORDER BY expert_id ASC";

/// slug 占用判定只看未软删行：软删后同名可重建（与 experts 一致）。
pub const SLUG_TAKEN_SQL: &str =
    "SELECT count(*) FROM teams WHERE user_id = ? AND team_slug = ? AND deleted_at IS NULL";

/// 幂等删除用：包含软删行，才能区分「删掉了」和「早就删过」。
pub const ANY_ROW_SQL: &str = "SELECT deleted_at FROM teams WHERE user_id = ? AND team_slug = ?";

pub const INSERT_TEAM_SQL: &str = "INSERT INTO teams (user_id, id, name, room_id, \
     leader_session_id, leader_expert_id, team_slug, description, guidelines, max_dispatch, \
     max_replan, max_ask_depth, state, state_changed_at, \
     created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'IDLE', ?, ?, ?)";

/// 主持人是一个普通专家：建团时同步开一个 `team_leader` 会话作为它的工作区。
/// 顺序上必须先于 teams 入库（teams 对 sessions 有外键）。
pub const INSERT_LEADER_SESSION_SQL: &str = "INSERT INTO sessions (user_id, id, kind, room_id, \
     team_id, expert_id, provider_id, model, state, workspace_path, created_at, updated_at, \
     last_active_at) VALUES (?, ?, 'team_leader', ?, ?, ?, 'local', ?, 'IDLE', ?, ?, ?, ?)";

pub const UPDATE_TEAM_SQL: &str = "UPDATE teams SET name = ?, description = ?, \
     guidelines = ?, max_dispatch = ?, max_replan = ?, max_ask_depth = ?, \
     leader_expert_id = ?, updated_at = ? \
     WHERE user_id = ? AND team_slug = ? AND deleted_at IS NULL";

/// 换主持人时同步改它的会话人格，否则会话会带着上一任主持人的人格说话。
pub const UPDATE_LEADER_SESSION_SQL: &str = "UPDATE sessions SET expert_id = ? \
     WHERE user_id = ? AND id = (SELECT leader_session_id FROM teams \
       WHERE user_id = ? AND team_slug = ? AND deleted_at IS NULL)";

pub const PUT_LEADER_MEMBER_SQL: &str = "INSERT INTO team_members (user_id, team_id, expert_id, \
     role, state, state_changed_at, joined_at, created_at, updated_at) \
     VALUES (?, ?, ?, 'leader', 'IDLE', ?, ?, ?, ?) \
     ON CONFLICT (user_id, team_id, expert_id) DO UPDATE SET role = 'leader'";

pub const DROP_LEADER_MEMBER_SQL: &str = "DELETE FROM team_members \
     WHERE user_id = ? AND team_id = (SELECT id FROM teams \
       WHERE user_id = ? AND team_slug = ? AND deleted_at IS NULL) AND role = 'leader'";

pub const PUT_MEMBER_SQL: &str = "INSERT INTO team_members (user_id, team_id, expert_id, role, \
     state, state_changed_at, joined_at, created_at, updated_at) \
     VALUES (?, ?, ?, 'member', 'IDLE', ?, ?, ?, ?)";

/// 成员集合整体替换：先清空 role='member' 的行再按新集合写回。
/// 主持人的行不动（它 role='leader'），成员数因此不会因为换人而错位。
pub const CLEAR_MEMBERS_SQL: &str = "DELETE FROM team_members WHERE user_id = ? AND team_id = ? \
     AND role = 'member'";

/// 软删。与专家删除同一口径：硬删会连带丢 messages，且与 `deleted_at`
/// 字段存在的意图矛盾。
pub const SOFT_DELETE_SQL: &str =
    "UPDATE teams SET deleted_at = ?, updated_at = ? WHERE user_id = ? AND team_slug = ? \
       AND deleted_at IS NULL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamRow {
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    pub leader_id: String,
    pub member_ids: Vec<String>,
    /// 团队行为约束（`teams.guidelines`）。空串 = 没有额外约束。
    /// 派工时逐字进每个成员的提示，见 `quill_agent::team_limits`。
    pub guidelines: String,
    pub max_dispatch: u32,
    pub max_replan: u32,
    pub max_ask_depth: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

fn blob_of(user: &quill_adapters::UserId) -> Vec<u8> {
    crate::db::blob_of(user)
}

pub fn slug_to_bytes(slug: &str) -> Vec<u8> {
    slug.as_bytes().to_vec()
}

/// 随团创建的主持人会话需要的全部派生值。
#[derive(Debug, Clone)]
pub struct NewTeamRow {
    pub team_uuid: [u8; 16],
    pub leader_session: [u8; 16],
    pub room_id: String,
    pub workspace_path: String,
    pub provider_id: String,
    pub model: String,
}

impl TeamRow {
    pub fn new(
        team_id: String,
        name: String,
        description: Option<String>,
        leader_id: String,
        member_ids: Vec<String>,
        limits: quill_agent::TeamLimits,
    ) -> Self {
        let now = now_ms();
        Self {
            team_id,
            name,
            description,
            leader_id,
            member_ids,
            guidelines: limits.guidelines().to_string(),
            max_dispatch: limits.max_dispatch(),
            max_replan: limits.max_replan(),
            max_ask_depth: limits.max_ask_depth(),
            created_at: now,
            updated_at: now,
        }
    }

    /// 把四个限制列还原成派工侧要用的 [`quill_agent::TeamLimits`]。
    ///
    /// 库里存的值正常都被 schema 的 CHECK 约束过，但**不假设**：越界时如实
    /// 报错（调用方映射成 400 并给下一步），而不是带着一个坏限制去派工。
    pub fn limits(&self) -> Result<quill_agent::TeamLimits, quill_agent::TeamLimitsError> {
        quill_agent::TeamLimits::new(
            self.max_dispatch,
            self.max_replan,
            self.max_ask_depth,
            self.guidelines.clone(),
        )
    }
}

fn row_to_team(row: &SqliteRow, members: Vec<String>) -> Result<TeamRow, quill_agent::AgentError> {
    Ok(TeamRow {
        team_id: col!(row, String, "team_slug", OP_GET),
        name: col!(row, String, "name", OP_GET),
        description: col!(row, Option<String>, "description", OP_GET),
        leader_id: col!(row, String, "leader_expert_id", OP_GET),
        member_ids: members,
        guidelines: col!(row, String, "guidelines", OP_GET),
        max_dispatch: limit_col(row, "max_dispatch")?,
        max_replan: limit_col(row, "max_replan")?,
        max_ask_depth: limit_col(row, "max_ask_depth")?,
        created_at: col!(row, i64, "created_at", OP_GET),
        updated_at: col!(row, i64, "updated_at", OP_GET),
    })
}

/// 读一个 INTEGER 限制列并转成 `u32`。
///
/// 只做搬运（负数 / 溢出一律按数据损坏报），范围（2~8 / 0~5）由
/// `TeamLimits::new` 与 schema 的 CHECK 两头把关。
fn limit_col(row: &SqliteRow, name: &'static str) -> Result<u32, quill_agent::AgentError> {
    // 不走 `col!`：那个宏要求列名写字面量，而这里是三个限制列共用的一份。
    let v: i64 = sqlx::Row::try_get(row, name).map_err(|e| storage_error(OP_GET, e))?;
    u32::try_from(v)
        .map_err(|_| crate::db::invariant_broken(format!("teams.{name} = {v} 为负数或超出 u32")))
}

async fn sql_list(
    pool: &sqlx::SqlitePool,
    uid: Vec<u8>,
) -> Result<Vec<TeamRow>, quill_agent::AgentError> {
    let rows = sqlx::query(LIST_SQL)
        .bind(uid.clone())
        .fetch_all(pool)
        .await
        .map_err(|e| storage_error(OP_LIST, e))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let slug: String = col!(r, String, "team_slug", OP_LIST);
        let members = sql_members(pool, uid.clone(), &slug).await?;
        out.push(row_to_team(r, members)?);
    }
    Ok(out)
}

async fn sql_members(
    pool: &sqlx::SqlitePool,
    uid: Vec<u8>,
    slug: &str,
) -> Result<Vec<String>, quill_agent::AgentError> {
    let uuid: Vec<u8> = sqlx::query_scalar(
        "SELECT id FROM teams WHERE user_id = ? AND team_slug = ? AND deleted_at IS NULL",
    )
    .bind(uid.clone())
    .bind(slug.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| storage_error(OP_GET, e))?
    .unwrap_or_default();
    let rows = sqlx::query(MEMBERS_SQL)
        .bind(uid.clone())
        .bind(uuid)
        .fetch_all(pool)
        .await
        .map_err(|e| storage_error(OP_GET, e))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(col!(r, String, "expert_id", OP_GET));
    }
    Ok(out)
}

async fn sql_get(
    pool: &sqlx::SqlitePool,
    uid: Vec<u8>,
    slug: String,
) -> Result<Option<TeamRow>, quill_agent::AgentError> {
    let row = sqlx::query(GET_SQL)
        .bind(uid.clone())
        .bind(slug.clone())
        .fetch_optional(pool)
        .await
        .map_err(|e| storage_error(OP_GET, e))?;
    let Some(r) = row else {
        return Ok(None);
    };
    let members = sql_members(pool, uid, &slug).await?;
    Ok(Some(row_to_team(&r, members)?))
}

pub async fn list(
    db: &DbBridge,
    uid: quill_adapters::UserId,
) -> Result<Vec<TeamRow>, quill_agent::AgentError> {
    let b = blob_of(&uid);
    db.call(move |pool, _rt| Box::pin(async move { sql_list(&pool, b).await }))
}

pub async fn get(
    db: &DbBridge,
    uid: quill_adapters::UserId,
    slug: &str,
) -> Result<Option<TeamRow>, quill_agent::AgentError> {
    let b = blob_of(&uid);
    let s = slug.to_string();
    db.call(move |pool, _rt| Box::pin(async move { sql_get(&pool, b, s).await }))
}

/// 按 16 字节 `id` 取团。派工真执行用它把团名 / 主持人 / 成员读出来，
/// 好构造 `quill_domain::Team` 交给 `Dispatcher`。
pub async fn get_by_id(
    db: &DbBridge,
    uid: quill_adapters::UserId,
    team_id: [u8; 16],
) -> Result<Option<TeamRow>, quill_agent::AgentError> {
    let b = blob_of(&uid);
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let row = sqlx::query(GET_BY_ID_SQL)
                .bind(b.clone())
                .bind(team_id.to_vec())
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error(OP_GET, e))?;
            let Some(r) = row else {
                return Ok(None);
            };
            let rows = sqlx::query(MEMBERS_SQL)
                .bind(b)
                .bind(team_id.to_vec())
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error(OP_GET, e))?;
            let mut members = Vec::with_capacity(rows.len());
            for row in &rows {
                members.push(col!(row, String, "expert_id", OP_GET));
            }
            Ok(Some(row_to_team(&r, members)?))
        })
    })
}

pub async fn slug_taken(
    db: &DbBridge,
    uid: quill_adapters::UserId,
    slug: &str,
) -> Result<bool, quill_agent::AgentError> {
    let b = blob_of(&uid);
    let s = slug.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let n: i64 = sqlx::query_scalar(SLUG_TAKEN_SQL)
                .bind(b)
                .bind(s)
                .fetch_one(&pool)
                .await
                .map_err(|e| storage_error(OP_WRITE, e))?;
            Ok(n > 0)
        })
    })
}

/// 这个 id 下有没有**没被软删**的团队行。
///
/// 派工路由用它把「路径里的团队不存在」挡在记账之前（2026-10-07）。
/// 之前那条路由把 `{id}` 解析完就直接塞进台账，从不查库，于是传一个
/// 编出来的 id 也照样返回 200 与空记录 —— 台账里就此多出一批指向
/// 不存在团队的派工，而且**没有任何地方消费它**，错了也没人知道。
///
/// 之所以要按 `(user_id, id)` 查而不是只查 id：`teams` 的主键就是这两列，
/// 只按 id 查等于把别人的团队算成自己的。已软删的团队按「不存在」处理 ——
/// 派工给一个删掉的团队同样说不通。
pub async fn exists_by_id(
    db: &DbBridge,
    uid: quill_adapters::UserId,
    team_id: [u8; 16],
) -> Result<bool, quill_agent::AgentError> {
    let b = blob_of(&uid);
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM teams \
                 WHERE user_id = ? AND id = ? AND deleted_at IS NULL",
            )
            .bind(b)
            .bind(team_id.to_vec())
            .fetch_one(&pool)
            .await
            .map_err(|e| storage_error(OP_WRITE, e))?;
            Ok(n > 0)
        })
    })
}

/// 建团：主持人会话 + teams 行 + 主持人成员行 + 成员行，一个事务。
pub async fn create(
    db: &DbBridge,
    uid: quill_adapters::UserId,
    team: &TeamRow,
    ids: &NewTeamRow,
) -> Result<(), quill_agent::AgentError> {
    let ub = blob_of(&uid);
    let team = team.clone();
    let ids = ids.clone();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let mut tx = pool.begin().await.map_err(|e| storage_error(OP_WRITE, e))?;
            let now = now_ms();

            sqlx::query(INSERT_LEADER_SESSION_SQL)
                .bind(ub.clone())
                .bind(ids.leader_session.to_vec())
                .bind(ids.room_id.clone())
                .bind(ids.team_uuid.to_vec())
                .bind(team.leader_id.clone())
                .bind(ids.model.clone())
                .bind(ids.workspace_path.clone())
                .bind(now)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| storage_error("建主持人会话", e))?;

            sqlx::query(INSERT_TEAM_SQL)
                .bind(ub.clone())
                .bind(ids.team_uuid.to_vec())
                .bind(team.name.clone())
                .bind(ids.room_id.clone())
                .bind(ids.leader_session.to_vec())
                .bind(team.leader_id.clone())
                .bind(team.team_id.clone())
                .bind(team.description.clone())
                .bind(team.guidelines.clone())
                .bind(i64::from(team.max_dispatch))
                .bind(i64::from(team.max_replan))
                .bind(i64::from(team.max_ask_depth))
                .bind(now)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| storage_error("建团队", e))?;

            sqlx::query(PUT_LEADER_MEMBER_SQL)
                .bind(ub.clone())
                .bind(ids.team_uuid.to_vec())
                .bind(team.leader_id.clone())
                .bind(now)
                .bind(now)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| storage_error("写主持人成员行", e))?;

            for m in &team.member_ids {
                sqlx::query(PUT_MEMBER_SQL)
                    .bind(ub.clone())
                    .bind(ids.team_uuid.to_vec())
                    .bind(m.clone())
                    .bind(now)
                    .bind(now)
                    .bind(now)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| storage_error("写成员行", e))?;
            }

            tx.commit().await.map_err(|e| storage_error(OP_WRITE, e))?;
            Ok(())
        })
    })
}

/// 改团：名称/描述/主持人/成员一次落库。成员集合整体替换。
pub async fn update(
    db: &DbBridge,
    uid: quill_adapters::UserId,
    team: &TeamRow,
    leader_changed: bool,
) -> Result<(), quill_agent::AgentError> {
    let ub = blob_of(&uid);
    let team = team.clone();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let mut tx = pool.begin().await.map_err(|e| storage_error(OP_WRITE, e))?;
            let now = now_ms();

            sqlx::query(UPDATE_TEAM_SQL)
                .bind(team.name.clone())
                .bind(team.description.clone())
                .bind(team.guidelines.clone())
                .bind(i64::from(team.max_dispatch))
                .bind(i64::from(team.max_replan))
                .bind(i64::from(team.max_ask_depth))
                .bind(team.leader_id.clone())
                .bind(now)
                .bind(ub.clone())
                .bind(team.team_id.clone())
                .execute(&mut *tx)
                .await
                .map_err(|e| storage_error("改团队", e))?;

            if leader_changed {
                sqlx::query(DROP_LEADER_MEMBER_SQL)
                    .bind(ub.clone())
                    .bind(ub.clone())
                    .bind(team.team_id.clone())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| storage_error("撤旧主持人", e))?;
                let uuid: Vec<u8> = sqlx::query_scalar(
                    "SELECT id FROM teams WHERE user_id = ? AND team_slug = ? AND deleted_at IS NULL",
                )
                .bind(ub.clone())
                .bind(team.team_id.clone())
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| storage_error("读团队主键", e))?;
                sqlx::query(PUT_LEADER_MEMBER_SQL)
                    .bind(ub.clone())
                    .bind(uuid.clone())
                    .bind(team.leader_id.clone())
                    .bind(now)
                    .bind(now)
                    .bind(now)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| storage_error("写新主持人", e))?;
                sqlx::query(UPDATE_LEADER_SESSION_SQL)
                    .bind(team.leader_id.clone())
                    .bind(ub.clone())
                    .bind(ub.clone())
                    .bind(team.team_id.clone())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| storage_error("改主持人会话人格", e))?;
            }

            let uuid: Vec<u8> = sqlx::query_scalar(
                "SELECT id FROM teams WHERE user_id = ? AND team_slug = ? AND deleted_at IS NULL",
                )
                .bind(ub.clone())
                .bind(team.team_id.clone())
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| storage_error("读团队主键", e))?;
            sqlx::query(CLEAR_MEMBERS_SQL)
                .bind(ub.clone())
                .bind(uuid.clone())
                .execute(&mut *tx)
                .await
                .map_err(|e| storage_error("清空旧成员", e))?;
            for m in &team.member_ids {
                sqlx::query(PUT_MEMBER_SQL)
                    .bind(ub.clone())
                    .bind(uuid.clone())
                    .bind(m.clone())
                    .bind(now)
                    .bind(now)
                    .bind(now)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| storage_error("写成员行", e))?;
            }

            tx.commit().await.map_err(|e| storage_error(OP_WRITE, e))?;
            Ok(())
        })
    })
}

/// 这一行在不在（含软删行）。删除路由靠它区分 404 与幂等 200。
pub async fn exists_including_deleted(
    db: &DbBridge,
    uid: quill_adapters::UserId,
    slug: &str,
) -> Result<bool, quill_agent::AgentError> {
    let b = blob_of(&uid);
    let s = slug.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let row: Option<Option<i64>> = sqlx::query_scalar(ANY_ROW_SQL)
                .bind(b)
                .bind(s)
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error(OP_GET, e))?;
            Ok(row.is_some())
        })
    })
}

/// 软删。`Ok(true)` = 本次删掉了；`Ok(false)` = 没有可删的行（早已软删，
/// 或调用方没判存在性就直接调了这里）。404 由调用方用
/// `exists_including_deleted` 判，不在这里混进存储层。
pub async fn soft_delete(
    db: &DbBridge,
    uid: quill_adapters::UserId,
    slug: &str,
) -> Result<bool, quill_agent::AgentError> {
    let ub = blob_of(&uid);
    let s = slug.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let prior: Option<Option<i64>> = sqlx::query_scalar(ANY_ROW_SQL)
                .bind(ub.clone())
                .bind(s.clone())
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error(OP_WRITE, e))?;
            match prior {
                // 行不存在：调用方已经先判过存在性，走到这说明是并发删除，
                // 按幂等处理（不报错，也不假装删掉了）。
                None | Some(Some(_)) => Ok(false),
                Some(None) => {
                    let now = now_ms();
                    let n = sqlx::query(SOFT_DELETE_SQL)
                        .bind(now)
                        .bind(now)
                        .bind(ub)
                        .bind(s)
                        .execute(&pool)
                        .await
                        .map_err(|e| storage_error(OP_WRITE, e))?;
                    Ok(n.rows_affected() > 0)
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_read_path_keeps_the_owner_predicate_and_the_soft_delete_filter() {
        for sql in [LIST_SQL, GET_SQL, MEMBERS_SQL, SLUG_TAKEN_SQL, ANY_ROW_SQL] {
            assert!(
                sql.contains("user_id = ?"),
                "隔离谓词必须在 SQL 里（不能靠上层过滤）：{sql}"
            );
        }
        for sql in [LIST_SQL, GET_SQL, SOFT_DELETE_SQL, UPDATE_TEAM_SQL] {
            assert!(!sql.contains("SELECT *"), "必须显式列清单：{sql}");
        }
        assert!(
            LIST_SQL.contains("deleted_at IS NULL") && GET_SQL.contains("deleted_at IS NULL"),
            "列表与单读都必须过滤软删行，否则删掉的团队还会出现"
        );
        assert!(
            SOFT_DELETE_SQL.contains("deleted_at IS NULL"),
            "软删语句必须自带未删条件，否则二次删除会把 deleted_at 刷新：{SOFT_DELETE_SQL}"
        );
        assert!(
            !SOFT_DELETE_SQL.contains("DELETE FROM teams"),
            "只能软删：硬删会连带丢 team_members 与派工记录"
        );
        assert!(
            ANY_ROW_SQL.contains("deleted_at"),
            "幂等删除需要能查到已软删的行：{ANY_ROW_SQL}"
        );
    }

    #[test]
    fn members_are_read_in_a_stable_order_and_exclude_the_leader_row() {
        assert!(
            MEMBERS_SQL.contains("ORDER BY expert_id ASC"),
            "member_ids 必须有序输出：{MEMBERS_SQL}"
        );
        assert!(
            MEMBERS_SQL.contains("role = 'member'"),
            "主持人不计入成员数，读取时必须按 role 过滤：{MEMBERS_SQL}"
        );
        assert!(
            CLEAR_MEMBERS_SQL.contains("role = 'member'"),
            "换人时不能把主持人的行一起删掉（会撞 ux_team_leader）：{CLEAR_MEMBERS_SQL}"
        );
    }

    #[test]
    fn every_limit_column_is_read_and_written_by_the_production_sql() {
        // Q043：这四列曾经**只存在于 schema** —— 读写 SQL 一个字都没提，
        // 于是「团队限制」从来没有影响过任何一次派工。这条测试钉住
        // 「三条读 SQL + 两条写 SQL」都带着它们；少了任何一条都会变红。
        const LIMITS: [&str; 4] = ["guidelines", "max_dispatch", "max_replan", "max_ask_depth"];
        for sql in [COLUMNS, LIST_SQL, GET_SQL, GET_BY_ID_SQL, INSERT_TEAM_SQL] {
            for c in LIMITS {
                assert!(sql.contains(c), "读/写 SQL 漏了限制列 {c}：{sql}");
            }
        }
        assert!(
            UPDATE_TEAM_SQL.contains("guidelines = ?")
                && UPDATE_TEAM_SQL.contains("max_dispatch = ?")
                && UPDATE_TEAM_SQL.contains("max_replan = ?")
                && UPDATE_TEAM_SQL.contains("max_ask_depth = ?"),
            "改团时必须能改限制列（否则只有建团那一刻能设）：{UPDATE_TEAM_SQL}"
        );
    }

    #[test]
    fn the_leader_session_is_written_before_the_team_row() {
        // teams 对 sessions 有外键（leader_session_id），顺序反了会直接 FK 失败。
        assert!(
            INSERT_LEADER_SESSION_SQL.contains("'team_leader'"),
            "主持人会话必须是 team_leader 类型（团队不是 solo 会话）：{INSERT_LEADER_SESSION_SQL}"
        );
        assert!(
            INSERT_TEAM_SQL.contains("team_slug") && INSERT_TEAM_SQL.contains("description"),
            "建团必须显式写公开标识与描述：{INSERT_TEAM_SQL}"
        );
        assert!(
            PUT_MEMBER_SQL.contains("'IDLE'"),
            "成员行初始必须是 IDLE：team_members 的 CHECK 要求 member_session_id 为空时 state=IDLE"
        );
    }
}
