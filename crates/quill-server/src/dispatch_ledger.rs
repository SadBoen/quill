use std::collections::BTreeMap;
use std::sync::Arc;

use quill_adapters::{ExpertId, MemberId, MemberOutcome, MemberStatus, SessionId, UserId};
use quill_agent::{
    AgentError, BeginOutcome, DispatchKey, DispatchLedger, DispatchRecord, DispatchState,
    RoundPrefix,
};
use sqlx::sqlite::SqliteRow;

use crate::db::{col, digest16, now_ms, storage_error, DbBridge};

const OP_BEGIN: &str = "派工记账";
const OP_PUT: &str = "更新派工记录";
const OP_GET: &str = "读取派工记录";
const OP_INFLIGHT: &str = "列出在途派工";
const OP_ROUND: &str = "列出某轮派工";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchScope {
    pub team_id: [u8; 16],

    pub leader_session_id: SessionId,

    pub member_sessions: BTreeMap<ExpertId, SessionId>,
}

impl DispatchScope {
    pub fn new(
        team_id: [u8; 16],
        leader_session_id: SessionId,
        member_sessions: BTreeMap<ExpertId, SessionId>,
    ) -> Self {
        Self {
            team_id,
            leader_session_id,
            member_sessions,
        }
    }

    fn member_session(&self, expert: &ExpertId, room_id: &str) -> Result<&SessionId, AgentError> {
        self.member_sessions
            .get(expert)
            .ok_or_else(|| AgentError::DispatchRequestInvalid {
                reason: format!(
                    "派工 {room_id} 缺少专家 {expert} 的成员会话（member_sessions 里没有它）。\
                     成员会话由会话创建流程分配；下一步：先用 `quill doctor --section=teams` \
                     检查该团队的成员会话是否已建立，再重试派工。"
                ),
            })
    }
}

const COLUMNS: &str = "id, user_id, room_id, \"round\", member_expert_id, member_session_id, \
                       task_digest, state, ask_depth, result_digest, result_bytes, \
                       error_code, error_message";

const ROW_ID_LABEL: &str = "task_dispatches.id";

fn begin_sql() -> String {
    format!(
        "INSERT INTO task_dispatches (\
           user_id, id, room_id, team_id, \"round\", leader_session_id, member_session_id, \
           member_expert_id, task_digest, state, ask_depth, result_digest, result_bytes, \
           dispatched_at, started_at, settled_at, created_at, updated_at\
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 0, ?, ?, NULL, ?, ?) \
         ON CONFLICT DO NOTHING \
         RETURNING {COLUMNS}"
    )
}

pub(crate) const PUT_SQL: &str = "INSERT INTO task_dispatches (\
     user_id, id, room_id, team_id, \"round\", leader_session_id, member_session_id, \
     member_expert_id, task_digest, state, ask_depth, result_digest, result_bytes, \
     error_code, error_message, dispatched_at, started_at, settled_at, created_at, updated_at\
   ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
   ON CONFLICT (user_id, id) DO UPDATE SET \
     state = excluded.state, \
     ask_depth = excluded.ask_depth, \
     result_digest = excluded.result_digest, \
     result_bytes = excluded.result_bytes, \
     error_code = excluded.error_code, \
     error_message = excluded.error_message, \
     leader_session_id = excluded.leader_session_id, \
     member_session_id = excluded.member_session_id, \
     started_at = COALESCE(task_dispatches.started_at, excluded.started_at), \
     settled_at = excluded.settled_at, \
     updated_at = excluded.updated_at";

#[derive(Debug, Clone)]
pub struct SqlxDispatchLedger {
    db: Arc<DbBridge>,
    scope: Arc<DispatchScope>,
}

impl SqlxDispatchLedger {
    pub fn new(db: Arc<DbBridge>, scope: DispatchScope) -> Self {
        Self {
            db,
            scope: Arc::new(scope),
        }
    }

    pub fn scope(&self) -> &DispatchScope {
        &self.scope
    }
}

fn task_envelope(record: &DispatchRecord) -> Vec<u8> {
    let v = serde_json::json!({ "member": record.member().as_str() });
    v.to_string().into_bytes()
}

fn parse_member(raw: &[u8], key: &DispatchKey) -> Result<MemberId, AgentError> {
    let text = std::str::from_utf8(raw).map_err(|e| {
        crate::db::invariant_broken(format!("task_digest 不是 UTF-8（{e}）：数据已损坏"))
    })?;
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| {
        crate::db::invariant_broken(format!("task_digest 不是合法 JSON（{e}）：数据已损坏"))
    })?;
    let member = v.get("member").and_then(|m| m.as_str()).ok_or_else(|| {
        crate::db::invariant_broken(format!(
            "task_digest 缺少 member 字段：派工 {key} 的成员名无法恢复"
        ))
    })?;
    MemberId::parse(member).map_err(|e| {
        crate::db::invariant_broken(format!(
            "task_digest.member = {member:?} 不是合法标识（{e}）"
        ))
    })
}

fn parse_result(
    raw: Option<Vec<u8>>,
    state: DispatchState,
    error_code: Option<String>,
    error_message: Option<String>,
    member: &MemberId,
) -> Result<Option<Result<MemberOutcome, AgentError>>, AgentError> {
    match state {
        DispatchState::Done => {
            let raw = raw.ok_or_else(|| {
                crate::db::invariant_broken(
                    "DONE 派工没有 result_digest：成员产出正文无法恢复（数据已损坏）",
                )
            })?;
            let v = parse_json(&raw, "result_digest")?;
            let status = match v.get("status").and_then(|s| s.as_str()) {
                Some("done") => MemberStatus::Done,
                Some("partial") => MemberStatus::Partial,
                other => {
                    return Err(crate::db::invariant_broken(format!(
                        "result_digest.status = {other:?} 不是 done/partial"
                    )))
                }
            };
            let scope = v
                .get("scope")
                .and_then(|s| s.as_str())
                .ok_or_else(|| {
                    crate::db::invariant_broken("result_digest 缺少 scope（已完成范围）字段")
                })?
                .to_string();
            let output = v
                .get("output")
                .and_then(|s| s.as_str())
                .ok_or_else(|| {
                    crate::db::invariant_broken("result_digest 缺少 output（产出正文）字段")
                })?
                .to_string();
            let outcome =
                MemberOutcome::new(member.clone(), status, scope, output).map_err(|e| {
                    crate::db::invariant_broken(format!("结算结果与成员状态不自洽：{e}"))
                })?;
            Ok(Some(Ok(outcome)))
        }
        DispatchState::Failed | DispatchState::Cancelled => {
            let code = error_code.ok_or_else(|| {
                crate::db::invariant_broken(
                    "终态派工没有 error_code：无法恢复失败原因（错误码是前端分支依据）",
                )
            })?;
            let kind = kind_from_wire(&code).ok_or_else(|| {
                crate::db::invariant_broken(format!(
                    "error_code = {code:?} 不在可重建的成员失败类别内（member_rejected / \
                     member_unauthorized / member_reported_failure / member_cancelled）：\
                     该类错误无法从冻结的 AgentError 变体无损重建"
                ))
            })?;
            let retryable = raw
                .as_deref()
                .map(|b| parse_json(b, "result_digest"))
                .transpose()?
                .and_then(|v| v.get("retryable").and_then(|b| b.as_bool()))
                .unwrap_or(false);
            let detail = error_message.unwrap_or_else(|| "（原始错误正文缺失）".to_string());
            Ok(Some(Err(AgentError::MemberRejected {
                member: member.clone(),
                kind,
                detail,
                retryable,
            })))
        }
        _ => Ok(None),
    }
}

fn parse_json(raw: &[u8], column: &str) -> Result<serde_json::Value, AgentError> {
    let text = std::str::from_utf8(raw).map_err(|e| {
        crate::db::invariant_broken(format!("{column} 不是 UTF-8（{e}）：数据已损坏"))
    })?;
    serde_json::from_str(text)
        .map_err(|e| crate::db::invariant_broken(format!("{column} 不是合法 JSON（{e}）")))
}

fn kind_from_wire(code: &str) -> Option<quill_agent::MemberRejectKind> {
    use quill_agent::MemberRejectKind as K;

    Some(match code {
        "member_rejected" => K::Rejected,
        "member_unauthorized" => K::Unauthorized,
        "member_reported_failure" => K::SelfReportedFailure,
        "member_cancelled" => K::Cancelled,
        _ => return None,
    })
}

fn result_payload(
    record: &DispatchRecord,
) -> (Option<Vec<u8>>, i64, Option<String>, Option<String>) {
    match (record.state(), record.outcome(), record.error()) {
        (DispatchState::Done, Some(outcome), _) => {
            let v = serde_json::json!({
                "status": outcome.status().as_wire(),
                "scope": outcome.completed_scope(),
                "output": outcome.output(),
            });
            let bytes = v.to_string().into_bytes();
            let n = bytes.len() as i64;
            (Some(bytes), n, None, None)
        }
        (_, _, Some(err)) => {
            let v = serde_json::json!({ "retryable": err.is_retryable() });
            let bytes = v.to_string().into_bytes();
            let n = bytes.len() as i64;
            (
                Some(bytes),
                n,
                Some(err.code().to_string()),
                Some(err.to_string()),
            )
        }
        _ => (None, 0, None, None),
    }
}

fn row_into_record(row: &SqliteRow) -> Result<DispatchRecord, AgentError> {
    let id_raw: Vec<u8> = col!(row, Vec<u8>, "id", OP_GET);
    let owner_raw: Vec<u8> = col!(row, Vec<u8>, "user_id", OP_GET);
    let room_id: String = col!(row, String, "room_id", OP_GET);
    let round: i64 = col!(row, i64, "round", OP_GET);
    let expert_raw: String = col!(row, String, "member_expert_id", OP_GET);
    let task_digest: Vec<u8> = col!(row, Vec<u8>, "task_digest", OP_GET);
    let state_raw: String = col!(row, String, "state", OP_GET);
    let ask_depth: i64 = col!(row, i64, "ask_depth", OP_GET);
    let result_digest: Option<Vec<u8>> = col!(row, Option<Vec<u8>>, "result_digest", OP_GET);
    let error_code: Option<String> = col!(row, Option<String>, "error_code", OP_GET);
    let error_message: Option<String> = col!(row, Option<String>, "error_message", OP_GET);

    let owner = crate::db::user_id_from_blob(&owner_raw).ok_or_else(|| {
        crate::db::invariant_broken(format!(
            "task_dispatches.user_id 长度是 {} 字节，应为 16 字节",
            owner_raw.len()
        ))
    })?;
    let expert = ExpertId::parse(&expert_raw).map_err(|e| {
        crate::db::invariant_broken(format!("member_expert_id = {expert_raw:?} 非法（{e}）"))
    })?;
    let state = DispatchState::from_wire(&state_raw).ok_or_else(|| {
        crate::db::invariant_broken(format!("task_dispatches.state = {state_raw:?} 不合法"))
    })?;

    let expected_id = row_id_for(&owner, &room_id, round, &expert);
    if id_raw.as_slice() != expected_id.as_slice() {
        return Err(crate::db::invariant_broken(format!(
            "派工 {room_id}/round-{round}/{expert} 的行 id 与键的派生值不一致：\
             数据已损坏或被绕过本端口写入"
        )));
    }
    let round = u32::try_from(round)
        .map_err(|_| crate::db::invariant_broken("task_dispatches.round 为负数"))?;

    let key = DispatchKey::new(owner, room_id, round, expert)?;
    let member = parse_member(&task_digest, &key)?;

    let mut record = DispatchRecord::pending(key, member.clone());
    match parse_result(result_digest, state, error_code, error_message, &member)? {
        Some(Ok(outcome)) => record.settle_done(outcome)?,
        Some(Err(err)) => record.settle_failed(err)?,
        None => match state {
            DispatchState::Pending => {}
            DispatchState::Running => record.mark_running()?,
            DispatchState::Asking => {
                let depth = u32::try_from(ask_depth).map_err(|_| {
                    crate::db::invariant_broken(format!("ask_depth = {ask_depth} 为负数"))
                })?;
                if depth == 0 {
                    return Err(crate::db::invariant_broken(
                        "ASKING 派工的 ask_depth 必须 > 0（schema 有对应 CHECK）",
                    ));
                }
                record.mark_running()?;
                record.mark_asking(depth)?;
            }
            DispatchState::Done | DispatchState::Failed | DispatchState::Cancelled => {
                return Err(crate::db::invariant_broken(
                    "终态派工缺少结算数据：无法恢复为领域记录（数据已损坏）",
                ))
            }
        },
    }
    Ok(record)
}

fn row_id_for(owner: &UserId, room_id: &str, round: i64, expert: &ExpertId) -> [u8; 16] {
    let round_bytes = round.to_be_bytes();
    digest16(
        ROW_ID_LABEL,
        &[
            owner.as_bytes().as_slice(),
            room_id.as_bytes(),
            round_bytes.as_slice(),
            expert.as_str().as_bytes(),
        ],
    )
}

async fn select_by_key(
    pool: &sqlx::SqlitePool,
    owner: UserId,
    room_id: &str,
    round: i64,
    expert: &str,
) -> Result<Option<SqliteRow>, AgentError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM task_dispatches \
         WHERE user_id = ? AND room_id = ? AND \"round\" = ? AND member_expert_id = ?"
    );
    sqlx::query(&sql)
        .bind(crate::db::blob_of(&owner))
        .bind(room_id)
        .bind(round)
        .bind(expert)
        .fetch_optional(pool)
        .await
        .map_err(|e| storage_error(OP_GET, e))
}

async fn sql_begin(
    pool: &sqlx::SqlitePool,
    scope: &DispatchScope,
    record: DispatchRecord,
) -> Result<BeginOutcome, AgentError> {
    let key = record.key().clone();
    let owner = key.owner();
    let round = i64::from(key.round());
    let member_session = scope
        .member_session(key.member_expert(), key.room_id())?
        .as_bytes()
        .to_vec();
    let id = row_id_for(&owner, key.room_id(), round, key.member_expert());
    let now = now_ms();
    let started_at = if record.state() == DispatchState::Pending {
        None
    } else {
        Some(now)
    };

    let inserted = sqlx::query(&begin_sql())
        .bind(crate::db::blob_of(&owner))
        .bind(id.to_vec())
        .bind(key.room_id())
        .bind(scope.team_id.to_vec())
        .bind(round)
        .bind(scope.leader_session_id.as_bytes().to_vec())
        .bind(member_session)
        .bind(key.member_expert().as_str())
        .bind(task_envelope(&record))
        .bind(record.state().as_wire())
        .bind(i64::from(record.ask_depth()))
        .bind(now)
        .bind(started_at)
        .bind(now)
        .bind(now)
        .fetch_optional(pool)
        .await
        .map_err(|e| storage_error(OP_BEGIN, e))?;

    if let Some(row) = inserted {
        return Ok(BeginOutcome::Created(row_into_record(&row)?));
    }

    match select_by_key(
        pool,
        owner,
        key.room_id(),
        round,
        key.member_expert().as_str(),
    )
    .await?
    {
        Some(row) => Ok(BeginOutcome::Existed(row_into_record(&row)?)),
        None => Err(crate::db::invariant_broken(format!(
            "派工 {key} 的插入被数据库判为冲突，但按幂等键查不到既有行。\
             可能原因：派生行 id 与另一条派工撞车（`task_dispatches.id` 的哈希碰撞）。\
             下一步：用 `quill doctor --section=db` 导出该房间的派工记录并人工核对，\
             不要把这次记账当成成功。"
        ))),
    }
}

async fn sql_put(
    pool: &sqlx::SqlitePool,
    scope: &DispatchScope,
    record: DispatchRecord,
) -> Result<(), AgentError> {
    let key = record.key().clone();
    let owner = key.owner();
    let round = i64::from(key.round());
    let member_session = scope
        .member_session(key.member_expert(), key.room_id())?
        .as_bytes()
        .to_vec();
    let id = row_id_for(&owner, key.room_id(), round, key.member_expert());
    let now = now_ms();
    let started_at = if record.state() == DispatchState::Pending {
        None
    } else {
        Some(now)
    };
    let settled_at = if record.state().is_terminal() {
        Some(now)
    } else {
        None
    };
    let (result_digest, result_bytes, error_code, error_message) = result_payload(&record);

    sqlx::query(PUT_SQL)
        .bind(crate::db::blob_of(&owner))
        .bind(id.to_vec())
        .bind(key.room_id())
        .bind(scope.team_id.to_vec())
        .bind(round)
        .bind(scope.leader_session_id.as_bytes().to_vec())
        .bind(member_session)
        .bind(key.member_expert().as_str())
        .bind(task_envelope(&record))
        .bind(record.state().as_wire())
        .bind(i64::from(record.ask_depth()))
        .bind(result_digest)
        .bind(result_bytes)
        .bind(error_code)
        .bind(error_message)
        .bind(now)
        .bind(started_at)
        .bind(settled_at)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| storage_error(OP_PUT, e))?;
    Ok(())
}

async fn sql_get(
    pool: &sqlx::SqlitePool,
    key: &DispatchKey,
) -> Result<Option<DispatchRecord>, AgentError> {
    let row = select_by_key(
        pool,
        key.owner(),
        key.room_id(),
        i64::from(key.round()),
        key.member_expert().as_str(),
    )
    .await?;
    match row {
        Some(r) => Ok(Some(row_into_record(&r)?)),
        None => Ok(None),
    }
}

async fn sql_inflight(
    pool: &sqlx::SqlitePool,
    owner: UserId,
) -> Result<Vec<DispatchRecord>, AgentError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM task_dispatches \
         WHERE user_id = ? AND state IN ('PENDING', 'RUNNING', 'ASKING') \
         ORDER BY dispatched_at ASC, member_expert_id ASC"
    );
    let rows = sqlx::query(&sql)
        .bind(crate::db::blob_of(&owner))
        .fetch_all(pool)
        .await
        .map_err(|e| storage_error(OP_INFLIGHT, e))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(row_into_record(r)?);
    }
    Ok(out)
}

async fn sql_list_round(
    pool: &sqlx::SqlitePool,
    prefix: &RoundPrefix,
) -> Result<Vec<DispatchRecord>, AgentError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM task_dispatches \
         WHERE user_id = ? AND room_id = ? AND \"round\" = ? \
         ORDER BY member_expert_id ASC"
    );
    let rows = sqlx::query(&sql)
        .bind(crate::db::blob_of(&prefix.owner))
        .bind(&prefix.room_id)
        .bind(i64::from(prefix.round))
        .fetch_all(pool)
        .await
        .map_err(|e| storage_error(OP_ROUND, e))?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(row_into_record(r)?);
    }
    Ok(out)
}

impl DispatchLedger for SqlxDispatchLedger {
    fn begin(&self, record: &DispatchRecord) -> Result<BeginOutcome, AgentError> {
        let scope = Arc::clone(&self.scope);
        let record = record.clone();
        self.db
            .call(move |pool, _rt| Box::pin(async move { sql_begin(&pool, &scope, record).await }))
    }

    fn put(&self, record: &DispatchRecord) -> Result<(), AgentError> {
        let scope = Arc::clone(&self.scope);
        let record = record.clone();
        self.db
            .call(move |pool, _rt| Box::pin(async move { sql_put(&pool, &scope, record).await }))
    }

    fn get(&self, key: &DispatchKey) -> Result<Option<DispatchRecord>, AgentError> {
        let key = key.clone();
        self.db
            .call(move |pool, _rt| Box::pin(async move { sql_get(&pool, &key).await }))
    }

    fn inflight(&self, owner: &UserId) -> Result<Vec<DispatchRecord>, AgentError> {
        let owner = *owner;
        self.db
            .call(move |pool, _rt| Box::pin(async move { sql_inflight(&pool, owner).await }))
    }

    fn list_round(&self, key_prefix: &RoundPrefix) -> Result<Vec<DispatchRecord>, AgentError> {
        let prefix = key_prefix.clone();
        self.db
            .call(move |pool, _rt| Box::pin(async move { sql_list_round(&pool, &prefix).await }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_statement_conflicts_on_any_unique_constraint() {
        let sql = begin_sql();
        assert!(
            sql.contains("ON CONFLICT DO NOTHING"),
            "必须是不带冲突目标的 DO NOTHING：{sql}"
        );
        assert!(
            sql.contains("RETURNING"),
            "判别必须来自 RETURNING，不能靠先查后插：{sql}"
        );
    }

    #[test]
    fn row_id_is_a_pure_function_of_the_idempotency_key() {
        let owner = UserId::from_bytes([1u8; 16]);
        let expert = ExpertId::parse("cost-analyst").expect("合法");
        let a = row_id_for(&owner, "room", 0, &expert);
        let b = row_id_for(&owner, "room", 0, &expert);
        let other_round = row_id_for(&owner, "room", 1, &expert);
        let other_user = row_id_for(&UserId::from_bytes([2u8; 16]), "room", 0, &expert);
        assert_eq!(a, b, "同键必须同 id（否则 upsert 每次都插新行）");
        assert_ne!(a, other_round, "轮次不同必须不同 id");
        assert_ne!(a, other_user, "🔴 跨用户：不同用户必须不同 id");
    }

    #[test]
    fn unknown_error_code_is_reported_not_guessed() {
        assert!(kind_from_wire("member_cancelled").is_some());
        assert!(
            kind_from_wire("storage_error").is_none(),
            "storage_error 无法从 MemberRejected 无损重建，必须判红"
        );
    }
}
