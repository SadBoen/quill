use std::collections::BTreeMap;

use axum::extract::{Path, RawQuery, State};
use axum::Json;
use serde_json::{json, Value};

use quill_adapters::{ExpertId, MemberId, SessionId};
use quill_agent::{
    BeginOutcome, DispatchKey, DispatchLedger, DispatchRecord, DispatchTask, Dispatcher,
    MemberResult, RoundPrefix, RoundRequest,
};

use crate::api_experts::{agent_error_to_api, map_agent_error, only_keys};
use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::dispatch_ledger::{DispatchScope, SqlxDispatchLedger};
use crate::error::ApiError;
use crate::jsonx::need_str;
use crate::state::AppState;

pub async fn list_round(
    State(state): State<AppState>,
    user: AuthUser,
    Path(team): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Json<Value>, ApiError> {
    // 团队必须存在，否则 404 —— 与 POST 那条同一把尺（见下面 book 的注释）。
    //
    // 这条路由原先是 `Path(_team)`，**下划线前缀、直接丢弃**：于是问一个
    // 编出来的团队 id 也能拿到 200 与一份空记录，读起来像「这个团队没有
    // 派工历史」。那是把「没有这个东西」说成「它没有记录」，是假回答。
    let team_id = parse_hex16(&team, "路径参数 {id}（团队标识）")?;
    let exists = crate::teams_repo::exists_by_id(state.db()?, user.0.user_id, team_id)
        .await
        .map_err(|e| agent_error_to_api("核对团队是否存在", e))?;
    if !exists {
        return Err(ApiError::entity_not_found(format!(
            "团队 {team} 不存在（已软删的团队同样算不存在）。\
             下一步：先用 GET /api/teams 确认这个 id 还在，再看它的派工记录。"
        )));
    }
    let (room_id, round) = parse_room_round(query.as_deref())?;
    let ledger = read_only_ledger(&state)?;
    let prefix = RoundPrefix {
        owner: user.0.user_id,
        room_id: room_id.clone(),
        round,
    };
    let records = map_agent_error("列出某轮派工", ledger.list_round(&prefix))?;
    Ok(Json(json!({
        // team_id 一并回给客户端：路径里的 {id} 现在**真的参与了判定**，
        // 调用方能确认自己问的是哪个团队，不必靠猜。
        "team_id": team,
        "room_id": room_id,
        "round": round,
        "count": records.len(),
        "dispatches": records.iter().map(record_json).collect::<Vec<Value>>(),
        // 说清一件事：这个列表按 room_id + round 过滤，**不是**按 team_id ——
        // 台账的键就是 (owner, room_id, round)。所以「团队存在」与
        // 「返回的记录属于这个团队」是两回事，改成按团队过滤要动键，
        // 那是产品契约变更，见 BACKLOG B2-1。
        "filtered_by": "room_id+round",
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
    // 团队必须真的存在（且属于当前用户、没被软删），否则 404。
    //
    // 这一步原先是缺的：路径里的 id 解析完就直接进了台账，于是编一个
    // 不存在的团队 id 也能拿到 200 与一份空记录。它把 B2-1 从「派工记录
    // 没有消费者」推进到「派工记录也不会凭空记」—— 注意**只是**不再
    // 凭空记，真正派子 agent 仍然是 M2 未完成的部分。
    let exists = crate::teams_repo::exists_by_id(state.db()?, user.0.user_id, team_id)
        .await
        .map_err(|e| agent_error_to_api("核对团队是否存在", e))?;
    if !exists {
        return Err(ApiError::entity_not_found(format!(
            "团队 {team} 不存在（已软删的团队同样算不存在）。\
             下一步：先用 GET /api/teams 确认这个 id 还在，再往它派工。"
        )));
    }
    let room_id = need_str(&body, "room_id", "派工请求")?;
    let round = need_round(&body)?;
    let leader =
        SessionId::parse(&need_str(&body, "leader_session_id", "派工请求")?).map_err(|e| {
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
        let where_ = format!("members[{i}]");
        only_keys(m, &["expert", "member", "member_session_id"], "members[]")?;
        let expert = ExpertId::parse(&need_str(m, "expert", &where_)?)
            .map_err(|e| ApiError::bad_request(format!("members[{i}].expert 非法（{e}）。")))?;
        let member = MemberId::parse(&need_str(m, "member", &where_)?)
            .map_err(|e| ApiError::bad_request(format!("members[{i}].member 非法（{e}）。")))?;

        if !member.as_str().starts_with(&format!("{expert}-")) {
            return Err(ApiError::bad_request(format!(
                "members[{i}].member = {:?} 必须以 \"{expert}-\" 开头（成员实例标识由专家名派生）。",
                member.as_str()
            )));
        }
        let sid = SessionId::parse(&need_str(m, "member_session_id", &where_)?).map_err(|e| {
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
        let where_ = format!("members[{i}]");
        let expert = ExpertId::parse(&need_str(m, "expert", &where_)?)
            .map_err(|e| ApiError::bad_request(format!("members[{i}].expert 非法（{e}）。")))?;
        let member = MemberId::parse(&need_str(m, "member", &where_)?)
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

/// `POST /api/teams/{id}/dispatch/run` —— **真的把这一轮跑起来**（M2）。
///
/// 与 `book` 的分工，说清楚免得混：
///
/// - `book`：只记账。响应里 `executed:false`，是给「先把轮次登记下来、稍后再跑」
///   这类调用方的，语义一个字没变。
/// - `run`（本条）：**真执行**。每个成员各调一次模型，产出收进 `results`。
///   幂等闸门仍然是那张台账 —— `Dispatcher::dispatch_round` 对已终态的成员直接跳过，
///   所以重复提交同一轮是安全的（`skipped_as_duplicate` 会如实报出来）。
///
/// **为什么另起一条路由而不是改 `book`**：`book` 的响应形状（`executed:false`）
/// 是对外契约的一部分；把执行混进去会让它的语义随请求内容而变，老调用方要跟着改。
/// 新增一条是纯增量。
///
/// 与 `book` 唯一的请求字段差别：每个成员**必须多带 `title` 与 `instructions`** ——
/// 光知道「派给谁」执行不了，还得知道「让它干什么」。
pub async fn run(
    State(state): State<AppState>,
    user: AuthUser,
    Path(team): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    only_keys(
        &body,
        &["room_id", "round", "leader_session_id", "members"],
        "POST /api/teams/{id}/dispatch/run",
    )?;
    let team_id = parse_hex16(&team, "路径参数 {id}（团队标识）")?;
    let db = state.db()?;
    let row = crate::teams_repo::get_by_id(db, user.0.user_id, team_id)
        .await
        .map_err(|e| agent_error_to_api("读取团队", e))?
        .ok_or_else(|| {
            ApiError::entity_not_found(format!(
                "团队 {team} 不存在（已软删的团队同样算不存在）。\
                 下一步：先用 GET /api/teams 确认这个 id 还在，再往它派工。"
            ))
        })?;

    let room_id = need_str(&body, "room_id", "派工请求")?;
    let round = need_round(&body)?;
    let leader =
        SessionId::parse(&need_str(&body, "leader_session_id", "派工请求")?).map_err(|e| {
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
                 {\"expert\":\"cost-analyst\",\"member\":\"cost-analyst-1\",\
                 \"title\":\"分析成本\",\"instructions\":\"给出三点结论\",\
                 \"member_session_id\":\"<32 位十六进制>\"}。"
                    .to_string(),
            )
        })?;
    if members.is_empty() {
        return Err(ApiError::bad_request(
            "members 为空：这一轮没有任何成员要派。".to_string(),
        ));
    }

    let mut member_sessions: BTreeMap<ExpertId, SessionId> = BTreeMap::new();
    let mut tasks = Vec::with_capacity(members.len());
    for (i, m) in members.iter().enumerate() {
        only_keys(
            m,
            &[
                "expert",
                "member",
                "title",
                "instructions",
                "member_session_id",
            ],
            "members[]",
        )?;
        let where_ = format!("members[{i}]");
        let expert = ExpertId::parse(&need_str(m, "expert", &where_)?)
            .map_err(|e| ApiError::bad_request(format!("members[{i}].expert 非法（{e}）。")))?;
        let member = MemberId::parse(&need_str(m, "member", &where_)?)
            .map_err(|e| ApiError::bad_request(format!("members[{i}].member 非法（{e}）。")))?;
        if !member.as_str().starts_with(&format!("{expert}-")) {
            return Err(ApiError::bad_request(format!(
                "members[{i}].member = {:?} 必须以 \"{expert}-\" 开头（成员实例标识由专家名派生）。",
                member.as_str()
            )));
        }
        // 与 `book` 的区别就在这两行：真执行要知道「这一项叫什么、要它做什么」。
        let title = need_str(m, "title", &where_)?;
        let instructions = need_str(m, "instructions", &where_)?;
        let sid = SessionId::parse(&need_str(m, "member_session_id", &where_)?).map_err(|e| {
            ApiError::bad_request(format!("members[{i}].member_session_id 非法（{e}）。"))
        })?;
        if member_sessions.insert(expert.clone(), sid).is_some() {
            return Err(ApiError::bad_request(format!(
                "members 里出现了两次专家 {expert}：同一轮同一专家只允许一条派工（ux_dispatch_once）。"
            )));
        }
        tasks.push(
            DispatchTask::new(expert.clone(), member, title, instructions)
                .map_err(|e| agent_error_to_api("组装派工任务", e))?,
        );
    }

    // 把存储里的团还原成领域 `Team`：`dispatch_round` 用它校验「派给的这个专家
    // 到底是不是本团成员」。**名册必须来自库里存的那份，不能来自本次请求** ——
    // 拿请求里的专家去建名册，等于把这道校验架空：派给谁都算「是本团成员」。
    let mut domain_team = quill_domain::team::team_of(&row.team_id, &row.name, &row.leader_id)
        .map_err(|e| ApiError::internal(format!("团队 {team} 的行读出来不合法：{e}")))?;
    let stored: Vec<&str> = row.member_ids.iter().map(String::as_str).collect();
    let roster = quill_domain::team::roster(&stored);
    for id in &row.member_ids {
        let expert = ExpertId::parse(id)
            .map_err(|e| ApiError::internal(format!("团队 {team} 存的成员 {id:?} 不合法：{e}")))?;
        // 名册本身就是从这一列读出来的，所以这里「不在名册」不可能发生；
        // 真发生了是数据损坏，如实报 500 而不是悄悄跳过。
        domain_team
            .add_member(expert, &roster)
            .map_err(|e| ApiError::internal(format!("团队 {team} 的成员名册在库里不合法：{e}")))?;
    }

    let ledger = SqlxDispatchLedger::new(
        std::sync::Arc::clone(db),
        DispatchScope::new(team_id, leader, member_sessions),
    );
    let executor = crate::member_executor::ProviderMemberExecutor::new(
        state.llm()?,
        state.llm_config_snapshot(),
        std::sync::Arc::clone(db),
    );
    let dispatcher = Dispatcher::new(executor, ledger);
    let owner = user.0.user_id;

    // `dispatch_round` 是同步的，而且会阻塞整轮（成员数 × 模型延迟）；
    // 直接在这里调会占住一个 tokio worker 直到最后一个成员答完。
    let report = tokio::task::spawn_blocking(move || {
        let req = RoundRequest {
            owner,
            session: leader,
            team: &domain_team,
            room_id: &room_id,
            round,
            tasks: &tasks,
            chain: &[],
        };
        dispatcher.dispatch_round(&req)
    })
    .await
    .map_err(|e| {
        ApiError::internal(format!(
            "派工线程没能返回结果（可能是 panic）：{e}。\
             下一步：看服务端日志里这一轮之前的 [chat] / 派工记录。"
        ))
    })
    .and_then(|r| r.map_err(|e| agent_error_to_api("执行派工", e)))?;

    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "room_id": report.room_id,
            "round": report.round,
            "executed": true,
            "delivered": report.delivered_count(),
            "failed": report.failed_count(),
            "skipped_as_duplicate": report
                .skipped_as_duplicate
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<&str>>(),
            "recovered_for_retry": report.recovered_for_retry.len(),
            "results": report.results.iter().map(result_json).collect::<Vec<Value>>(),
            "summary": report.summary(),
        })),
    ))
}

fn result_json(r: &MemberResult) -> Value {
    let mut v = json!({ "member": r.member().as_str() });
    match r {
        MemberResult::Delivered { outcome, .. } => {
            v["status"] = json!("delivered");
            v["scope"] = json!(outcome.completed_scope());
            v["output"] = json!(outcome.output());
        }
        MemberResult::Failed { error, .. } => {
            v["status"] = json!("failed");
            v["error_code"] = json!(error.code());
            v["error"] = json!(error.to_string());
        }
    }
    v
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
