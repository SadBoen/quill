use std::collections::BTreeMap;

use axum::extract::{Path, RawQuery, State};
use axum::Json;
use serde_json::{json, Value};

use quill_adapters::{ExpertId, MemberId, SessionId};
use quill_agent::{BeginOutcome, DispatchKey, DispatchLedger, DispatchRecord, RoundPrefix};

use crate::api_experts::{agent_error_to_api, map_agent_error, only_keys};
use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::dispatch_ledger::{DispatchScope, SqlxDispatchLedger};
use crate::error::ApiError;
use crate::state::AppState;

pub async fn list_round(
    State(state): State<AppState>,
    user: AuthUser,
    Path(_team): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Json<Value>, ApiError> {
    let (room_id, round) = parse_room_round(query.as_deref())?;
    let ledger = read_only_ledger(&state)?;
    let prefix = RoundPrefix {
        owner: user.0.user_id,
        room_id: room_id.clone(),
        round,
    };
    let records = map_agent_error("列出某轮派工", ledger.list_round(&prefix))?;
    Ok(Json(json!({
        "room_id": room_id,
        "round": round,
        "count": records.len(),
        "dispatches": records.iter().map(record_json).collect::<Vec<Value>>(),
    })))
}

pub async fn book(
    State(state): State<AppState>,
    user: AuthUser,
    Path(team): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    only_keys(
        &body,
        &["room_id", "round", "leader_session_id", "members"],
        "POST /api/teams/{id}/dispatch",
    )?;
    let team_id = parse_hex16(&team, "路径参数 {id}（团队标识）")?;
    let room_id = need_str(&body, "room_id")?;
    let round = need_round(&body)?;
    let leader = SessionId::parse(&need_str(&body, "leader_session_id")?).map_err(|e| {
        ApiError::bad_request(format!(
            "leader_session_id 非法（{e}）：应为 32 位十六进制。"
        ))
    })?;

    let members = body
        .get("members")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ApiError::bad_request(
                "缺少字段 members（数组）。每项形如 \
                 {\"expert\":\"cost-analyst\",\"member\":\"cost-analyst-1\",\"member_session_id\":\"<32 位十六进制>\"}。"
                    .to_string(),
            )
        })?;
    if members.is_empty() {
        return Err(ApiError::bad_request(
            "members 为空：没有任何成员要派，记账不会产生任何记录。".to_string(),
        ));
    }

    let mut member_sessions: BTreeMap<ExpertId, SessionId> = BTreeMap::new();
    for (i, m) in members.iter().enumerate() {
        only_keys(m, &["expert", "member", "member_session_id"], "members[]")?;
        let expert = ExpertId::parse(&need_str(m, "expert")?)
            .map_err(|e| ApiError::bad_request(format!("members[{i}].expert 非法（{e}）。")))?;
        let member = MemberId::parse(&need_str(m, "member")?)
            .map_err(|e| ApiError::bad_request(format!("members[{i}].member 非法（{e}）。")))?;

        if !member.as_str().starts_with(&format!("{expert}-")) {
            return Err(ApiError::bad_request(format!(
                "members[{i}].member = {:?} 必须以 \"{expert}-\" 开头（成员实例标识由专家名派生）。",
                member.as_str()
            )));
        }
        let sid = SessionId::parse(&need_str(m, "member_session_id")?).map_err(|e| {
            ApiError::bad_request(format!("members[{i}].member_session_id 非法（{e}）。"))
        })?;
        if member_sessions.insert(expert.clone(), sid).is_some() {
            return Err(ApiError::bad_request(format!(
                "members 里出现了两次专家 {expert}：同一轮同一专家只允许一条派工（ux_dispatch_once）。"
            )));
        }
    }

    let ledger = SqlxDispatchLedger::new(
        std::sync::Arc::clone(state.db()?),
        DispatchScope::new(team_id, leader, member_sessions),
    );

    let mut items = Vec::new();
    for (i, m) in members.iter().enumerate() {
        let expert = ExpertId::parse(&need_str(m, "expert")?)
            .map_err(|e| ApiError::bad_request(format!("members[{i}].expert 非法（{e}）。")))?;
        let member = MemberId::parse(&need_str(m, "member")?)
            .map_err(|e| ApiError::bad_request(format!("members[{i}].member 非法（{e}）。")))?;
        let key = DispatchKey::new(user.0.user_id, room_id.clone(), round, expert.clone())
            .map_err(|e| agent_error_to_api("组装派工键", e))?;
        let outcome = map_agent_error(
            "派工记账",
            ledger.begin(&DispatchRecord::pending(key, member.clone())),
        )?;
        let (verdict, record) = match &outcome {
            BeginOutcome::Created(r) => ("created", r),
            BeginOutcome::Existed(r) => ("existed", r),
        };
        items.push(json!({
            "expert": expert.as_str(),
            "member": member.as_str(),
            "idempotency": verdict,
            "state": record.state().as_wire(),
        }));
    }

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({
            "room_id": room_id,
            "round": round,
            "executed": false,
            "dispatches": items,
            "note": "本版本只记账不执行：成员执行器（MemberExecutor 生产实现）尚未落地。\
                     PENDING 记录在崩溃恢复口径下属于「可安全重派」，执行器就绪后重放同一轮即会真正执行。",
        })),
    ))
}

pub async fn inflight(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let ledger = read_only_ledger(&state)?;
    let records = map_agent_error("列出在途派工", ledger.inflight(&user.0.user_id))?;
    Ok(Json(json!({
        "count": records.len(),
        "dispatches": records.iter().map(record_json).collect::<Vec<Value>>(),
        "note": "「可安全重派 / 需人工确认」的分档由领域层 Dispatcher::recover 给出，\
                 本路由不复制该口径（避免第二份真相源）。",
    })))
}

fn read_only_ledger(state: &AppState) -> Result<SqlxDispatchLedger, ApiError> {
    Ok(SqlxDispatchLedger::new(
        std::sync::Arc::clone(state.db()?),
        DispatchScope::new([0u8; 16], SessionId::from_bytes([0u8; 16]), BTreeMap::new()),
    ))
}

fn record_json(r: &DispatchRecord) -> Value {
    let mut v = json!({
        "room_id": r.key().room_id(),
        "round": r.key().round(),
        "expert": r.key().member_expert().as_str(),
        "member": r.member().as_str(),
        "state": r.state().as_wire(),
        "ask_depth": r.ask_depth(),
    });
    if let Some(o) = r.outcome() {
        v["status"] = json!(o.status().as_wire());
        v["completed_scope"] = json!(o.completed_scope());
        v["result_bytes"] = json!(o.output().len());
    }
    if let Some(e) = r.error() {
        v["error_code"] = json!(e.code());
        v["error_detail"] = json!(e.to_string());
        v["retryable"] = json!(e.is_retryable());
    }
    v
}

fn parse_room_round(query: Option<&str>) -> Result<(String, u32), ApiError> {
    let mut room: Option<String> = None;
    let mut round: Option<u32> = None;
    for pair in query
        .unwrap_or_default()
        .split('&')
        .filter(|p| !p.is_empty())
    {
        let (k, v) = pair.split_once('=').ok_or_else(|| {
            ApiError::bad_request(format!(
                "查询串 {pair:?} 缺少 `=`（应为 room_id=…&round=…）。"
            ))
        })?;

        let v = v.replace('+', " ");
        match k {
            "room_id" => room = Some(v),
            "round" => {
                round = Some(v.parse::<u32>().map_err(|e| {
                    ApiError::bad_request(format!("round={v:?} 不是非负整数（{e}）。"))
                })?)
            }
            other => {
                return Err(ApiError::bad_request(format!(
                    "未知查询参数 {other:?}。本路由只接受 room_id 与 round。"
                )))
            }
        }
    }
    let room_id = room.ok_or_else(|| {
        ApiError::bad_request(
            "缺少查询参数 room_id。示例：GET /api/teams/{id}/dispatch?room_id=room-1&round=0"
                .to_string(),
        )
    })?;
    if room_id.trim().is_empty() {
        return Err(ApiError::bad_request("room_id 不能为空。".to_string()));
    }
    let round = round
        .ok_or_else(|| ApiError::bad_request("缺少查询参数 round（从 0 起）。".to_string()))?;
    Ok((room_id, round))
}

fn need_str(body: &Value, key: &str) -> Result<String, ApiError> {
    match body.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        Some(Value::String(_)) => Err(ApiError::bad_request(format!("字段 {key} 不能为空。"))),
        Some(_) => Err(ApiError::bad_request(format!("字段 {key} 必须是字符串。"))),
        None => Err(ApiError::bad_request(format!("缺少必填字段 {key}。"))),
    }
}

fn need_round(body: &Value) -> Result<u32, ApiError> {
    match body.get("round") {
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| ApiError::bad_request("round 必须是 0 ~ 4294967295 的整数。")),
        Some(_) => Err(ApiError::bad_request("round 必须是数字。")),
        None => Err(ApiError::bad_request("缺少必填字段 round（从 0 起）。")),
    }
}

fn parse_hex16(raw: &str, what: &str) -> Result<[u8; 16], ApiError> {
    let parsed = quill_adapters::UserId::parse(raw.trim()).map_err(|e| {
        ApiError::bad_request(format!(
            "{what} = {raw:?} 非法（{e}）：应为 32 位十六进制。"
        ))
    })?;
    Ok(*parsed.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_parsing_rejects_missing_and_unknown_params() {
        assert!(parse_room_round(Some("room_id=a&round=2")).is_ok());
        assert_eq!(
            parse_room_round(Some("room_id=a&round=2")).expect("合法"),
            ("a".to_string(), 2u32)
        );
        assert!(
            parse_room_round(Some("room_id=a")).is_err(),
            "缺 round 必须判红"
        );
        assert!(
            parse_room_round(Some("round=1")).is_err(),
            "缺 room_id 必须判红"
        );
        assert!(
            parse_room_round(Some("room_id=a&round=1&team=x")).is_err(),
            "未知参数必须判红（否则拼错的参数被静默忽略）"
        );
        assert!(
            parse_room_round(Some("room_id=a&round=-1")).is_err(),
            "负轮次必须判红"
        );
    }
}
