//! 专家团（团队）的五个 CRUD 端点。
//!
//! 契约（字段名与形状冻结，勿改）：
//!   Team = { team_id, name, description, leader_id, member_ids, created_at, updated_at }
//!   member_ids 不含主持人，长度 2~8，按 id 升序。
//!
//! 错误映射（不引入 Octop 的错误码体系，quill 的 ApiError 是固定枚举）：
//!   - 团队不存在 → 404 entity_not_found
//!   - 成员数 / 重复 / 引用不存在的专家 / 把团队当成员 → 400 bad_request（detail 点名是哪一种）
//!   - team_id 已被占用 → 409 conflict
//!
//! 有意不实现两个 Octop 错误码：
//!   - TEAM_MEMBER_BUSY：quill 没有常驻 agent 进程，不存在「某个成员正在被占用」
//!     这个状态。写一个永远走不到的错误分支等于凭空造状态。
//!   - TEAM_NOT_SHAREABLE：quill 没有团队共享/发布功能，这个判断没有对象。
//!
//! 两者要等真出现「成员进程占用」与「团队分享」这两件事时再随功能一起加。

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use quill_adapters::ExpertId;
use quill_agent::ExpertRepository;
use quill_domain::{Team, TeamError, TeamId};

use crate::api_experts::{map_agent_error, only_keys};
use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::experts_repo::SqlxExpertRepository;
use crate::state::AppState;
use crate::teams_repo::{self, NewTeamRow, TeamRow};

pub async fn list(State(state): State<AppState>, user: AuthUser) -> Result<Response, ApiError> {
    let db = state.db()?;
    let rows = map_agent_error("列出团队", teams_repo::list(db, user.0.user_id).await)?;
    Ok(Json(json!({
        "teams": rows.iter().map(team_json).collect::<Vec<Value>>()
    }))
    .into_response())
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    only_keys(
        &body,
        &["team_id", "name", "description", "leader_id", "member_ids"],
        "POST /api/teams",
    )?;

    let team_id = parse_team_id(&need_str(&body, "team_id")?, "team_id")?;
    let name = need_str(&body, "name")?;
    let description = opt_str(&body, "description")?;
    let leader = parse_expert(&need_str(&body, "leader_id")?, "leader_id")?;
    let members = parse_member_ids(&body)?;
    let roster = roster_of(&state, user.0.user_id)?;

    let domain = build_domain(team_id.clone(), &name, &leader, &members, &roster)?;
    domain
        .validate()
        .map_err(|e| team_error_to_api("创建团队", e))?;

    let db = state.db()?;
    if map_agent_error(
        "查团队标识占用",
        teams_repo::slug_taken(db, user.0.user_id, team_id.as_str()).await,
    )? {
        return Err(ApiError::conflict(
            format!("团队标识 {} 已被占用。", team_id.as_str()),
            "软删除后同名可再次创建；改个新标识也一样可以。",
        ));
    }

    let slug = team_id.as_str().to_string();
    let leader_name = leader.as_str().to_string();
    let member_names = members
        .iter()
        .map(|m| m.as_str().to_string())
        .collect::<Vec<String>>();
    let row = TeamRow::new(
        slug.clone(),
        name.clone(),
        description.clone(),
        leader_name,
        member_names,
    );

    let team_uuid = new_uuid()?;
    let ids = NewTeamRow {
        team_uuid,
        leader_session: new_uuid()?,
        room_id: format!(
            "room-{}",
            &quill_adapters::ids::to_hex_lower(&team_uuid)[..12]
        ),
        workspace_path: format!(
            "ws/{}",
            &quill_adapters::ids::to_hex_lower(&team_uuid)[..12]
        ),
        provider_id: "local".to_string(),
        model: state
            .llm_config
            .read()
            .map_err(|_| ApiError::internal("llm_config 锁被毒化"))?
            .model
            .clone(),
    };

    let db = state.db()?;
    map_agent_error(
        "建团队",
        teams_repo::create(db, user.0.user_id, &row, &ids).await,
    )?;

    // 落库后回读一次再返回：回包必须是库里的样子，而不是请求体的复述。
    let stored = map_agent_error(
        "读取新建的团队",
        teams_repo::get(db, user.0.user_id, &slug).await,
    )?
    .ok_or_else(|| {
        ApiError::internal(format!(
            "团队 {slug} 写入后读不回来（已写入服务端日志）。\
             下一步：执行 `quill doctor --section=db` 导出诊断。"
        ))
    })?;

    Ok((StatusCode::CREATED, Json(team_json(&stored))).into_response())
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let team_id = parse_team_id(&id, "路径参数 {id}")?;
    let db = state.db()?;
    let team = map_agent_error(
        "读取团队",
        teams_repo::get(db, user.0.user_id, team_id.as_str()).await,
    )?
    .ok_or_else(|| team_not_found(team_id.as_str()))?;
    Ok(Json(team_json(&team)).into_response())
}

pub async fn patch(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    only_keys(
        &body,
        &["name", "description", "leader_id", "member_ids"],
        "PATCH /api/teams/{id}",
    )?;
    let team_id = parse_team_id(&id, "路径参数 {id}")?;
    let owner = user.0.user_id;
    let db = state.db()?;
    let current = map_agent_error(
        "读取待改团队",
        teams_repo::get(db, owner, team_id.as_str()).await,
    )?
    .ok_or_else(|| team_not_found(team_id.as_str()))?;

    let name = match opt_str(&body, "name")? {
        Some(n) => n,
        None => current.name.clone(),
    };
    // description 的省略 / null 语义与 model 字段一致：省略 = 沿用，null = 清除。
    let description = if body.get("description").is_some() {
        opt_str(&body, "description")?
    } else {
        current.description.clone()
    };
    let leader = match opt_str(&body, "leader_id")? {
        Some(raw) => parse_expert(&raw, "leader_id")?,
        None => parse_expert(&current.leader_id, "leader_id")?,
    };
    let leader_changed = leader.as_str() != current.leader_id;

    let members = match body.get("member_ids") {
        Some(v) => parse_member_list(v)?,
        None => current
            .member_ids
            .iter()
            .map(|m| ExpertId::parse(m))
            .collect::<Result<Vec<ExpertId>, _>>()
            .map_err(|e| {
                ApiError::internal(format!(
                    "库里 teams {}/{}/team_members 的专家标识非法（已写入服务端日志）：{e}",
                    owner.to_compact_hex(),
                    team_id
                ))
            })?,
    };

    let roster = roster_of(&state, owner)?;
    if !roster.contains(&leader) {
        return Err(ApiError::bad_request(format!(
            "主持人 {} 不在你的专家名册里：主持人必须是一个真实存在的普通专家。\
             下一步：先 POST /api/experts 建出这个专家，或把 leader_id 改成名册里的专家。",
            leader.as_str()
        )));
    }
    let domain = build_domain(team_id.clone(), &name, &leader, &members, &roster)?;
    domain
        .validate()
        .map_err(|e| team_error_to_api("修改团队", e))?;

    let row = TeamRow {
        team_id: team_id.as_str().to_string(),
        name,
        description,
        leader_id: leader.as_str().to_string(),
        member_ids: members
            .iter()
            .map(|m| m.as_str().to_string())
            .collect::<Vec<String>>(),
        created_at: current.created_at,
        updated_at: crate::db::now_ms(),
    };
    map_agent_error(
        "改团队",
        teams_repo::update(db, owner, &row, leader_changed).await,
    )?;

    let stored = map_agent_error(
        "读取改后的团队",
        teams_repo::get(db, owner, team_id.as_str()).await,
    )?
    .ok_or_else(|| team_not_found(team_id.as_str()))?;
    Ok(Json(team_json(&stored)).into_response())
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let team_id = parse_team_id(&id, "路径参数 {id}")?;
    let db = state.db()?;
    // 先判存在性（含软删行）：不存在 = 404；已软删 = 幂等 200。
    if !map_agent_error(
        "查团队是否存在",
        teams_repo::exists_including_deleted(db, user.0.user_id, team_id.as_str()).await,
    )? {
        return Err(team_not_found(team_id.as_str()));
    }
    let deleted = map_agent_error(
        "删除团队",
        teams_repo::soft_delete(db, user.0.user_id, team_id.as_str()).await,
    )?;
    Ok(Json(json!({
        "team_id": team_id.as_str(),
        "deleted": deleted,
        "note": if deleted {
            "已软删除：团队对所有用户（含属主）不可见，同名 team_id 可再次创建。成员与会话记录保留，便于回溯派工历史。"
        } else {
            "该团队此前已被删除，本次为幂等重试（未重复删除）。"
        }
    }))
    .into_response())
}

fn team_json(t: &TeamRow) -> Value {
    json!({
        "team_id": t.team_id,
        "name": t.name,
        // 显式 null 而不是省略键：前端靠 `"description" in team` 判断字段是否被支持。
        "description": t.description,
        "leader_id": t.leader_id,
        "member_ids": t.member_ids,
        "created_at": t.created_at,
        "updated_at": t.updated_at,
    })
}

fn team_not_found(id: &str) -> ApiError {
    ApiError::entity_not_found(format!(
        "团队 {id} 不存在，或不属于当前用户。\
         下一步：用 GET /api/teams 列出自己的团队，或先 POST /api/teams 建一个。"
    ))
}

/// 用领域模型做全部成员规则校验：名册、重复、上限、嵌套、下限。
/// 领域层通过之后才允许写库 —— 校验与持久化分成两步，避免半写状态。
fn build_domain(
    team_id: TeamId,
    name: &str,
    leader: &ExpertId,
    members: &[ExpertId],
    roster: &BTreeSet<ExpertId>,
) -> Result<Team, ApiError> {
    // 主持人是一个普通专家：名册里没有它就等于凭空捏造了一个主持人。
    if !roster.contains(leader) {
        return Err(ApiError::bad_request(format!(
            "主持人 {} 不在你的专家名册里：主持人必须是一个真实存在的普通专家。\
             下一步：先 POST /api/experts 建出这个专家，或把 leader_id 改成名册里的专家。",
            leader.as_str()
        )));
    }
    // 主持人不计入成员数，因此也不能同时出现在 member_ids 里：
    // 否则库里会同时有 role='leader' 与 role='member' 两行同一个人。
    if members.iter().any(|m| m == leader) {
        return Err(ApiError::bad_request(format!(
            "主持人 {} 不能同时出现在 member_ids 里：主持人不计入成员数，\
             他有自己的工作区与记忆。\n下一步：把 {} 从 member_ids 里去掉，member_ids 只放其他成员。",
            leader.as_str(),
            leader.as_str()
        )));
    }
    let mut team = Team::new(team_id, name, leader.clone()).map_err(|e| {
        ApiError::bad_request(format!(
            "团队不合法：{e}。\
             下一步：team_id 用小写字母、数字与连字符（最长 64 个字符），name 不能为空。"
        ))
    })?;
    for m in members {
        team.add_member(m.clone(), roster)
            .map_err(|e| team_error_to_api("校验团队成员", e))?;
    }
    Ok(team)
}

/// Octop 的 TEAM_MEMBER_BUSY / TEAM_NOT_SHAREABLE 在这里**不映射**：
/// quill 没有常驻 agent 进程（没人会「忙」），也没有团队共享功能。
/// 没有触发条件就别造错误分支，见文件头说明。
fn team_error_to_api(op: &str, e: TeamError) -> ApiError {
    let detail = match &e {
        TeamError::TooFewMembers { got, min } => format!(
            "团队只有 {got} 个成员，少于下限 {min}（主持人不计入成员数）。\
             下一步：把 member_ids 补到至少 {min} 个普通专家，或换用单人会话（POST /api/sessions）。"
        ),
        TeamError::TooManyMembers { got, max } => format!(
            "团队有 {got} 个成员，超过上限 {max}。\
             下一步：把 member_ids 减到 {max} 个以内，或拆成两个团队。"
        ),
        TeamError::DuplicateMember(m) => format!(
            "member_ids 里出现了两次 {m}：同一专家在一个团队里只能出现一次。\
             下一步：去掉重复项（成员按 expert_id 去重）。"
        ),
        TeamError::MemberIsTeam { member, team } => format!(
            "{member} 是一个团队，不能作为 {team} 的成员：quill 不支持嵌套团队。\
             下一步：把 member_ids 里的团队换成普通专家。"
        ),
        TeamError::UnknownExpert(m) => format!(
            "专家 {m} 不在你的名册里。\
             下一步：先 POST /api/experts 建出这个专家，或把 member_ids 改成名册里已有的专家。"
        ),
        TeamError::IdEmpty | TeamError::IdInvalid(_) => format!(
            "团队标识非法：{e}。\
             下一步：team_id 只接受小写字母、数字与连字符组成的 kebab-case，最长 64 个字符。"
        ),
        TeamError::EmptyName => "团队名称为空。下一步：name 传一个非空字符串。".to_string(),
    };
    eprintln!("[teams] {op} 失败：{e}");
    ApiError::bad_request(detail)
}

fn roster_of(
    state: &AppState,
    uid: quill_adapters::UserId,
) -> Result<BTreeSet<ExpertId>, ApiError> {
    let repo = SqlxExpertRepository::new(Arc::clone(state.db()?));
    map_agent_error("读取专家名册", repo.roster(&uid))
}

fn parse_member_ids(body: &Value) -> Result<Vec<ExpertId>, ApiError> {
    let Some(raw) = body.get("member_ids") else {
        return Err(ApiError::bad_request(
            "缺少必填字段 member_ids（数组）。\
             请求体示例：{\"team_id\":\"growth-squad\",\"name\":\"增长小队\",\
             \"leader_id\":\"cost-analyst\",\"member_ids\":[\"growth-analyst\",\"risk-reviewer\"]}"
                .to_string(),
        ));
    };
    parse_member_list(raw)
}

fn parse_member_list(raw: &Value) -> Result<Vec<ExpertId>, ApiError> {
    let Some(items) = raw.as_array() else {
        return Err(ApiError::bad_request(format!(
            "member_ids 必须是字符串数组，实际收到 {}。",
            type_name(raw)
        )));
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, v) in items.iter().enumerate() {
        let Some(s) = v.as_str() else {
            return Err(ApiError::bad_request(format!(
                "member_ids[{i}] 必须是字符串，实际收到 {}。",
                type_name(v)
            )));
        };
        out.push(parse_expert(s, &format!("member_ids[{i}]"))?);
    }
    Ok(out)
}

fn parse_team_id(raw: &str, field: &str) -> Result<TeamId, ApiError> {
    TeamId::parse(raw.trim()).map_err(|e| {
        ApiError::bad_request(format!(
            "字段 {field} = {raw:?} 非法（{e}）：只接受小写字母、数字与连字符组成的 kebab-case，\
             长度 1~64。\n下一步：例如 \"growth-squad\"。"
        ))
    })
}

fn parse_expert(raw: &str, field: &str) -> Result<ExpertId, ApiError> {
    ExpertId::parse(raw.trim()).map_err(|e| {
        ApiError::bad_request(format!(
            "字段 {field} = {raw:?} 非法（{e}）：专家标识只接受小写字母、数字与连字符，\
             长度 1~64。\n下一步：传 GET /api/experts 列出的真实专家 id。"
        ))
    })
}

fn need_str(body: &Value, key: &str) -> Result<String, ApiError> {
    match body.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(ApiError::bad_request(format!(
            "字段 {key} 必须是字符串，实际收到 {}。",
            type_name(other)
        ))),
        None => Err(ApiError::bad_request(format!("缺少必填字段 {key}。"))),
    }
}

fn opt_str(body: &Value, key: &str) -> Result<Option<String>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(ApiError::bad_request(format!(
            "字段 {key} 必须是字符串或 null，实际收到 {}。",
            type_name(other)
        ))),
    }
}

use crate::jsonx::type_name;

fn new_uuid() -> Result<[u8; 16], ApiError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b)
        .map_err(|e| ApiError::internal(format!("生成标识失败（系统随机源不可用）：{e}")))?;
    if b == [0u8; 16] {
        b[0] = 1;
    }
    Ok(b)
}

#[allow(unused_imports)]
use crate::api_experts::only_keys as _only_keys_reexport;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_experts::agent_error_to_api;

    fn body(v: Value) -> Value {
        v
    }

    #[test]
    fn member_ids_must_be_an_array_of_strings_and_name_the_bad_index() {
        let err = parse_member_list(&body(json!("cost-analyst"))).expect_err("非数组必须判红");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert!(err.detail().contains("member_ids"), "{}", err.detail());

        let err = parse_member_list(&body(json!([1]))).expect_err("非字符串项必须判红");
        assert!(err.detail().contains("member_ids[0]"), "{}", err.detail());
    }

    #[test]
    fn every_team_error_maps_to_400_with_a_chinese_fix_direction() {
        let leader = ExpertId::parse("cost-analyst").expect("应合法");
        let roster = ["cost-analyst", "growth-analyst", "risk-reviewer"]
            .into_iter()
            .map(|s| ExpertId::parse(s).expect("应合法"))
            .collect::<BTreeSet<ExpertId>>();

        let cases = [
            (
                TeamError::TooFewMembers { got: 1, min: 2 },
                vec!["1", "2", "下一步"],
            ),
            (
                TeamError::TooManyMembers { got: 9, max: 8 },
                vec!["9", "8", "下一步"],
            ),
            (
                TeamError::DuplicateMember(leader.clone()),
                vec!["cost-analyst", "下一步"],
            ),
            (
                TeamError::UnknownExpert(ExpertId::parse("ghost").expect("应合法")),
                vec!["ghost", "下一步"],
            ),
            (
                TeamError::MemberIsTeam {
                    member: ExpertId::parse("growth-squad").expect("应合法"),
                    team: TeamId::parse("growth-squad").expect("应合法"),
                },
                vec!["嵌套", "下一步"],
            ),
        ];
        for (e, wants) in cases {
            let err = team_error_to_api("建团队", e);
            assert_eq!(
                err.status(),
                StatusCode::BAD_REQUEST,
                "团队规则错误必须是 400"
            );
            let text = format!("{}{}", err.detail(), err.next_step());
            for w in wants {
                assert!(text.contains(w), "错误文案必须含 {w:?}：{text}");
            }
        }
        let _ = roster;
    }

    #[test]
    fn team_json_always_emits_the_description_key_even_when_null() {
        let row = TeamRow {
            team_id: "growth-squad".into(),
            name: "增长小队".into(),
            description: None,
            leader_id: "cost-analyst".into(),
            member_ids: vec!["growth-analyst".into(), "risk-reviewer".into()],
            created_at: 1,
            updated_at: 2,
        };
        let v = team_json(&row);
        let obj = v.as_object().expect("必须是对象");
        for k in [
            "team_id",
            "name",
            "description",
            "leader_id",
            "member_ids",
            "created_at",
            "updated_at",
        ] {
            assert!(obj.contains_key(k), "契约字段 {k} 必须出现：{v}");
        }
        assert!(v["description"].is_null(), "未填描述必须是 JSON null");
        assert_eq!(v["member_ids"], json!(["growth-analyst", "risk-reviewer"]));
    }

    #[test]
    fn a_team_id_or_expert_id_that_is_not_kebab_case_is_400() {
        let err = parse_team_id("Growth Squad", "team_id").expect_err("大写与空格必须判红");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert!(err.detail().contains("team_id"));
        assert!(parse_team_id("growth-squad", "team_id").is_ok());
    }

    /// 代理用的错误映射不能把存储细节泄漏出去。
    #[test]
    fn storage_failures_never_leak_sql_text() {
        let err = agent_error_to_api(
            "建团队",
            quill_agent::AgentError::Storage {
                detail: "no such column: team_slug".to_string(),
            },
        );
        let text = format!("{}{}", err.detail(), err.next_step());
        assert!(
            !text.contains("no such column"),
            "内部 SQL 串泄漏了：{text}"
        );
    }
}
