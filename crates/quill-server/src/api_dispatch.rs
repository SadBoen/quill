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
    //
    // 现在这里读的是整行（`get_by_id`）而不是只判存在性：记账前的那两道
    // 团队限制闸门要用同一行里的 `max_dispatch` / `max_replan`（Q043）。
    let row = crate::teams_repo::get_by_id(state.db()?, user.0.user_id, team_id)
        .await
        .map_err(|e| agent_error_to_api("读取团队", e))?
        .ok_or_else(|| {
            ApiError::entity_not_found(format!(
                "团队 {team} 不存在（已软删的团队同样算不存在）。\
                 下一步：先用 GET /api/teams 确认这个 id 还在，再往它派工。"
            ))
        })?;
    let limits = row.limits().map_err(limits_error_to_api)?;
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

    // queue Q025：先确保每个成员的会话都在（缺则按 `team_member` 建）。
    // `task_dispatches.member_session_id` 对 `sessions` 有外键，会话不在就记不进账。
    // 本路由只说「派给谁」，没有任务标题 —— 会话标题退回专家名。
    ensure_member_sessions(
        &state,
        user.0.user_id,
        team_id,
        leader,
        &room_id,
        &member_sessions,
        &BTreeMap::new(),
    )?;

    let ledger = SqlxDispatchLedger::new(
        std::sync::Arc::clone(state.db()?),
        DispatchScope::new(team_id, leader, member_sessions),
    );

    // 记账前的团队限制闸门（Q043）：超限**整轮拒绝**，一行台账都不写 ——
    // 记账后才发现超限的话，调用方要么得到一个「记了但跑不了」的半状态，
    // 要么得自己回滚，两条都比直接拒绝难用。
    limits
        .check_dispatch(members.len())
        .map_err(|e| dispatch_error_to_api("派工记账", e))?;
    limits
        .check_replan(round)
        .map_err(|e| dispatch_error_to_api("派工记账", e))?;

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
            "team_limits": limits_json(&limits),
            "guidelines_applied": false,
            "dispatches": items,
            "note": "本路由只记账不执行（真执行走 POST /api/teams/{id}/dispatch/run）。\
                     团队限制（max_dispatch / max_replan）已在记账前过闸：超限的一轮不会被记进来。\
                     guidelines 不在这里生效 —— 它在真执行时逐字拼进每个成员的提示。",
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
    // 这个团的四个限制列（Q043）。读不出合法值就 400 + 下一步，
    // 而不是带着一个坏限制去跑整轮模型。
    let limits = row.limits().map_err(limits_error_to_api)?;

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

    // queue Q025：确保每个成员的会话都在，并把「任务 + 产出」的写回素材先备好 ——
    // 下面 `tasks` / `member_sessions` / `limits` 都要 move 进派工线程，跑完就取不到了。
    let titles: BTreeMap<ExpertId, String> = tasks
        .iter()
        .map(|t| (t.expert.clone(), t.title.clone()))
        .collect();
    ensure_member_sessions(
        &state,
        user.0.user_id,
        team_id,
        leader,
        &room_id,
        &member_sessions,
        &titles,
    )?;
    let write_plan: Vec<MemberWritePlan> = tasks
        .iter()
        .filter_map(|t| {
            member_sessions.get(&t.expert).map(|sid| MemberWritePlan {
                member: t.member.clone(),
                session: *sid.as_bytes(),
                // 与成员实际收到的正文**是同一份**（都过 `apply_guidelines`）——
                // 写进会话的是「这一轮真的发生了什么」，不是事后复述。
                task_text: format!(
                    "任务：{}\n\n{}",
                    t.title,
                    limits.apply_guidelines(&t.instructions)
                ),
            })
        })
        .collect();

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

    // 团队限制（Q043）：`max_dispatch` / `max_replan` 在**写台账与起成员之前**过闸，
    // 超限整轮拒绝；`guidelines` 由 `Dispatcher::run_one` 逐字拼进每个成员的提示。
    // 这里先过一遍是为了把错误在「还没起任何东西」的时候就返回，同时也让
    // 派工线程里那一道成为同一条闸门的第二道保险（两条走同一份 TeamLimits）。
    limits
        .check_dispatch(tasks.len())
        .map_err(|e| dispatch_error_to_api("执行派工", e))?;
    limits
        .check_replan(round)
        .map_err(|e| dispatch_error_to_api("执行派工", e))?;
    let guidelines_chars = limits.guidelines_chars();
    let limits_state = limits_json(&limits);

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
            limits: &limits,
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
    .and_then(|r| r.map_err(|e| dispatch_error_to_api("执行派工", e)))?;

    // queue Q025：把每个交付成员的任务与产出写进各自的会话。
    // 写失败**不让整轮失败**（成员已经真跑完了、报告必须回给调用方），
    // 失败原因原样进响应 —— 见 `persist_member_outputs` 的注释。
    let (member_messages_written, member_persist_error) =
        persist_member_outputs(db, user.0.user_id, &write_plan, &report);

    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "room_id": report.room_id,
            "round": report.round,
            "executed": true,
            "team_limits": limits_state,
            "guidelines_applied": guidelines_chars > 0,
            "guidelines_chars": guidelines_chars,
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
            "member_messages_written": member_messages_written,
            "member_persist_error": member_persist_error,
        })),
    ))
}

/// 把 `AgentError` 映射成 HTTP 错误，**先把团队限制超限那一类摘出来**。
///
/// `TeamLimitsExceeded` 是调用方的错（这一轮太大 / 轮次太靠后）：要 400，
/// 而且要把 `TeamLimits` 里那句中文的字段级「下一步」原样带出去。让它掉进
/// `agent_error_to_api` 的兜底分支会变成 500「真实原因已写入日志」——
/// 用户看到的是一句无从下手的话，而其实他只要把成员数减一个就行。
fn dispatch_error_to_api(op: &str, e: quill_agent::AgentError) -> ApiError {
    match e {
        quill_agent::AgentError::TeamLimitsExceeded {
            limit,
            got,
            max,
            next_step,
        } => ApiError::bad_request(format!(
            "超出团队限制「{limit}」：本次 {got}，上限 {max}。下一步：{next_step}"
        )),
        other => agent_error_to_api(op, other),
    }
}

/// 限制列本身存坏了（越过 API 直写、或 schema 被改过）：400 + 下一步，
/// 而不是 500 —— 用户能自己 PATCH 把这一行修好。
fn limits_error_to_api(e: quill_agent::TeamLimitsError) -> ApiError {
    ApiError::bad_request(format!(
        "团队的限制列不合法：{e}。下一步：{}",
        e.next_step()
    ))
}

/// 响应里如实回出这个团生效中的限制：派工被拒时，用户能对着它看到是哪一条卡住了
/// （只报「被拒」不报「上限是多少」，等于让人去猜）。
fn limits_json(l: &quill_agent::TeamLimits) -> Value {
    json!({
        "max_dispatch": l.max_dispatch(),
        "max_replan": l.max_replan(),
        "max_ask_depth": l.max_ask_depth(),
        "guidelines_chars": l.guidelines_chars(),
    })
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

// -------------------------------------------- Q025：成员跑在哪张会话里、产出写回哪

/// 一轮派工里「要写进哪个会话、写什么」的素材。
///
/// 在 `tasks` / `member_sessions` / `limits` 被 move 进派工线程**之前**拼好 ——
/// 落库发生在那一轮跑完之后，那时这些值已经不在手里了。
struct MemberWritePlan {
    member: MemberId,
    session: [u8; 16],
    task_text: String,
}

/// 确认这一轮每个成员的会话都在（queue Q025）。
///
/// **缺就建**：`task_dispatches.member_session_id` 对 `sessions` 有外键，会话不在
/// 台账就写不进去；而生产里没有任何别的地方会建成员会话，所以「给成员各开独立会话」
/// 这件事落在派工这一层。建出来的行满足 schema 对 `team_member` 的三项硬要求
/// （`team_id` / `parent_session_id` / `expert_id`，见 `0001_init.sql:284`）。
///
/// **已有就验**：那个 id 被一条**别的类型**的会话占用时 400 —— 把成员的消息写进
/// 用户的聊天会话是数据污染，不是「顺手复用」。
fn ensure_member_sessions(
    state: &AppState,
    uid: quill_adapters::UserId,
    team_id: [u8; 16],
    leader: SessionId,
    room_id: &str,
    members: &BTreeMap<ExpertId, SessionId>,
    titles: &BTreeMap<ExpertId, String>,
) -> Result<(), ApiError> {
    let db = state.db()?;
    let model = state
        .llm_config
        .read()
        .map_err(|_| ApiError::internal("llm_config 锁被毒化"))?
        .model
        .clone();
    for (expert, sid) in members {
        let sid_bytes = *sid.as_bytes();
        match crate::chat_repo::session_kind(db, uid, sid_bytes)
            .map_err(|e| agent_error_to_api("查成员会话", e))?
        {
            Some(kind) if kind == "team_member" => {}
            Some(kind) => {
                return Err(ApiError::bad_request(format!(
                    "members[] 里给专家 {expert} 的 member_session_id 指向一条 {kind:?} 类型的会话，\
                     不是团队成员的会话（团队成员的会话由派工自己建）。\
                     下一步：把这一项的 member_session_id 换成一个没被占用的 32 位十六进制 id，\
                     或删掉这一项。"
                )));
            }
            None => {
                let short = &quill_adapters::ids::to_hex_lower(&sid_bytes)[..12];
                let title: String = titles
                    .get(expert)
                    .cloned()
                    .unwrap_or_else(|| expert.as_str().to_string())
                    .chars()
                    .take(crate::api_chat::TITLE_MAX_CHARS)
                    .collect();
                crate::chat_repo::insert_member_session(
                    db,
                    uid,
                    crate::chat_repo::NewMemberSession {
                        id: sid_bytes,
                        room_id: room_id.to_string(),
                        title,
                        team_id,
                        parent_session_id: *leader.as_bytes(),
                        expert_id: expert.as_str().to_string(),
                        model: model.clone(),
                        workspace_path: format!("ws/{short}"),
                        now: crate::db::now_ms(),
                    },
                    "建成员会话",
                )
                .map_err(|e| agent_error_to_api("建成员会话", e))?;
            }
        }
    }
    Ok(())
}

/// 把这一轮每个交付成员的「任务 + 产出」写进各自的会话（queue Q025）。
///
/// 返回（写成了几个成员，第一条失败原因）。**故意不让它把整轮判失败**：成员已经
/// 真跑完了（钱花了），报告必须回给调用方；落库失败如实放进响应
/// （`member_persist_error`），不静默吞掉 —— 同 Q018 压缩失败那条取舍。
fn persist_member_outputs(
    db: &crate::db::DbBridge,
    uid: quill_adapters::UserId,
    plan: &[MemberWritePlan],
    report: &quill_agent::DispatchReport,
) -> (usize, Option<String>) {
    let mut written = 0usize;
    let mut first_error: Option<String> = None;
    for result in &report.results {
        let MemberResult::Delivered { member, outcome } = result else {
            // 失败 / 取消的成员没有「产出」可写（自报状态与原因在台账与响应里）。
            continue;
        };
        let Some(entry) = plan.iter().find(|p| p.member == *member) else {
            continue;
        };
        match write_member_turn(db, uid, entry.session, &entry.task_text, outcome.output()) {
            Ok(()) => written += 1,
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(format!("成员 {member} 的产出没能写进它的会话：{e}"));
                }
            }
        }
    }
    (written, first_error)
}

/// 一次成员回合写两条消息（user = 任务、assistant = 产出），并推进会话计数。
///
/// **用量一律不写**：成员执行器现在不带 token 用量（那是 Q026）—— 不编数，
/// 宁可让用量页看不到这一份，也不写真值不明的数字。
fn write_member_turn(
    db: &crate::db::DbBridge,
    uid: quill_adapters::UserId,
    sid: [u8; 16],
    task_text: &str,
    output: &str,
) -> Result<(), String> {
    let now = crate::db::now_ms();
    let seq = crate::chat_repo::next_seq(db, uid, sid).map_err(|e| e.to_string())?;
    for (seq, role, content) in [
        (seq, "user", task_text.to_string()),
        (seq + 1, "assistant", output.to_string()),
    ] {
        crate::chat_repo::insert_message(
            db,
            uid,
            sid,
            crate::chat_repo::NewMessage {
                id: crate::api_chat::new_id().map_err(|e| e.to_string())?,
                seq,
                role: role.to_string(),
                status: "complete".to_string(),
                content,
                reasoning: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: None,
                cache_write_tokens: None,
                turn_ms: None,
                created_at: now,
            },
        )
        .map_err(|e| e.to_string())?;
    }
    crate::chat_repo::touch_session(db, uid, sid, seq + 2, None, None, now)
        .map_err(|e| e.to_string())
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
