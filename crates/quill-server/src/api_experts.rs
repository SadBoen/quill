use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use quill_adapters::ExpertId;
use quill_agent::{AgentError, Expert, ExpertRegistry, NewExpert};

use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::experts_repo::SqlxExpertRepository;
use crate::state::AppState;

pub async fn list(State(state): State<AppState>, user: AuthUser) -> Result<Json<Value>, ApiError> {
    let registry = registry(&state)?;
    let experts = map_agent_error("列出专家", registry.list_visible(&user.0.user_id))?;
    Ok(Json(json!({
        "experts": experts.iter().map(expert_json).collect::<Vec<Value>>()
    })))
}

pub async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    only_keys(
        &body,
        &["id", "display_name", "description"],
        "POST /api/experts",
    )?;
    let new = NewExpert {
        id: expert_id(&body, "id")?,
        display_name: need_str(&body, "display_name")?,
        description: need_str(&body, "description")?,
    };
    let registry = registry(&state)?;

    let expert = map_agent_error("创建专家", registry.create_user_expert(user.0.user_id, new))?;
    Ok((axum::http::StatusCode::CREATED, Json(expert_json(&expert))))
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let registry = registry(&state)?;
    let slug = parse_slug(&slug)?;
    let expert = map_agent_error("读取专家", registry.get_visible(&user.0.user_id, &slug))?;
    Ok(Json(expert_json(&expert)))
}

pub async fn patch(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    only_keys(
        &body,
        &["display_name", "description", "default_enabled"],
        "PATCH /api/experts/{slug}",
    )?;
    let id = parse_slug(&slug)?;
    let owner = user.0.user_id;
    let registry = registry(&state)?;

    let mut touched = false;
    if let Some(name) = opt_str(&body, "display_name")? {
        map_agent_error("改专家名", registry.rename(&owner, &id, name).map(|_| ()))?;
        touched = true;
    }
    if let Some(desc) = opt_str(&body, "description")? {
        map_agent_error(
            "改专家描述",
            registry.redescribe(&owner, &id, desc).map(|_| ()),
        )?;
        touched = true;
    }
    if let Some(on) = opt_bool(&body, "default_enabled")? {
        map_agent_error(
            "改默认启用开关",
            registry.set_default_enabled(&owner, &id, on).map(|_| ()),
        )?;
        touched = true;
    }
    if !touched {
        return Err(ApiError::bad_request(
            "请求体里没有任何可改字段。\
             可改字段：display_name（字符串）、description（字符串）、default_enabled（布尔）。"
                .to_string(),
        ));
    }
    let expert = map_agent_error("读取改后的专家", registry.get_visible(&owner, &id))?;
    Ok(Json(expert_json(&expert)))
}

pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let registry = registry(&state)?;
    let id = parse_slug(&slug)?;
    let deleted = map_agent_error("删除专家", registry.delete(&user.0.user_id, &id))?;
    Ok(Json(json!({
        "id": id.as_str(),
        "deleted": deleted,
        "note": if deleted {
            "已软删除：专家对所有用户（含属主）不可见，同名可再次创建。"
        } else {
            "该专家此前已被删除，本次为幂等重试（未重复删除）。"
        }
    })))
}

fn registry(state: &AppState) -> Result<ExpertRegistry<SqlxExpertRepository>, ApiError> {
    let db = state.db()?;
    Ok(SqlxExpertRepository::registry(std::sync::Arc::clone(db)))
}

fn expert_json(e: &Expert) -> Value {
    json!({
        "id": e.id().as_str(),
        "owner": e.owner().to_compact_hex(),
        "display_name": e.display_name(),
        "description": e.description(),
        "visibility": e.visibility().as_wire(),
        "default_enabled": e.default_enabled(),
        "is_builtin": e.is_builtin(),
    })
}

fn parse_slug(raw: &str) -> Result<ExpertId, ApiError> {
    ExpertId::parse(raw).map_err(|e| {
        ApiError::bad_request(format!(
            "专家标识 {raw:?} 非法（{e}）：只接受小写字母、数字与连字符组成的 kebab-case，\
             且长度 1~64。"
        ))
    })
}

fn expert_id(body: &Value, key: &str) -> Result<ExpertId, ApiError> {
    parse_slug(&need_str(body, key)?)
}

fn need_str(body: &Value, key: &str) -> Result<String, ApiError> {
    match body.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(ApiError::bad_request(format!(
            "字段 {key} 必须是字符串，实际收到 {}。",
            type_name(other)
        ))),
        None => Err(ApiError::bad_request(format!(
            "缺少必填字段 {key}。请求体示例：{{\"id\":\"cost-analyst\",\"display_name\":\"成本分析师\",\"description\":\"…\"}}"
        ))),
    }
}

fn opt_str(body: &Value, key: &str) -> Result<Option<String>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(ApiError::bad_request(format!(
            "字段 {key} 必须是字符串，实际收到 {}。",
            type_name(other)
        ))),
    }
}

fn opt_bool(body: &Value, key: &str) -> Result<Option<bool>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(other) => Err(ApiError::bad_request(format!(
            "字段 {key} 必须是布尔值（true/false），实际收到 {}。",
            type_name(other)
        ))),
    }
}

pub(crate) fn only_keys(body: &Value, allowed: &[&str], route: &str) -> Result<(), ApiError> {
    let Some(map) = body.as_object() else {
        return Err(ApiError::bad_request(
            "请求体必须是 JSON 对象。".to_string(),
        ));
    };
    let unknown: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|k| !allowed.contains(k))
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "路由 {route} 不接受字段 {:?}。可接受字段：{}。\
             （字段名拼错会被静默忽略并返回成功，所以这里直接判红。）",
            unknown,
            allowed.join(" / ")
        )))
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "布尔值",
        Value::Number(_) => "数字",
        Value::String(_) => "字符串",
        Value::Array(_) => "数组",
        Value::Object(_) => "对象",
    }
}

pub fn map_agent_error<T>(op: &str, r: Result<T, AgentError>) -> Result<T, ApiError> {
    r.map_err(|e| agent_error_to_api(op, e))
}

pub fn agent_error_to_api(op: &str, e: AgentError) -> ApiError {
    eprintln!("[api] {op} 失败（错误码 {}）：{e}", e.code());
    match e {
        AgentError::ExpertNotFound { id } | AgentError::ExpertDeleted { id } => {
            ApiError::entity_not_found(format!("专家 {id} 不存在，或对你不可见。"))
        }
        AgentError::ExpertExists { id } => {
            ApiError::conflict(format!("专家 {id} 已存在。软删除后同名可再次创建。"))
        }
        AgentError::ExpertBuiltinProtected { id } => ApiError::forbidden(format!(
            "专家 {id} 是内置系统专家：内置专家随版本发布，不可修改、不可删除。"
        )),
        AgentError::ExpertNotModifiable { id } => {
            ApiError::forbidden(format!("专家 {id} 不是你创建的，无法修改。"))
        }
        AgentError::ExpertDisplayNameInvalid { raw, reason } => ApiError::bad_request(format!(
            "显示名 {raw:?} 不合法：{reason}（长度上限 64 个**字符**，中文按字符计）。"
        )),
        AgentError::DispatchRequestInvalid { reason } => ApiError::bad_request(reason),
        AgentError::Storage { .. } => ApiError::internal(
            "存储层操作失败，真实原因已写入服务端日志（响应体不含内部细节）。\
             下一步：用同一请求重试一次；若持续失败，执行 `quill doctor --section=db`。"
                .to_string(),
        ),
        AgentError::InvariantBroken { .. } => ApiError::internal(format!(
            "{op}时发现数据不自洽（已写入服务端日志）。\
             下一步：执行 `quill doctor --section=db` 导出诊断后人工核对。"
        )),
        other => ApiError::internal(format!(
            "{op}失败（错误码 {}）。真实原因已写入服务端日志。",
            other.code()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_field_is_rejected_rather_than_silently_ignored() {
        let body = json!({ "displayName": "成本分析师" });
        let err = only_keys(&body, &["display_name"], "PATCH /api/experts/{slug}")
            .expect_err("驼峰拼写必须判红");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(err.detail().contains("displayName"));
    }

    #[test]
    fn storage_error_never_leaks_the_underlying_sql_text() {
        let internal = "no such table: experts (code 1)";
        let err = agent_error_to_api(
            "列出专家",
            AgentError::Storage {
                detail: internal.to_string(),
            },
        );
        let body = format!("{}{}", err.detail(), err.next_step());
        assert!(!body.contains("no such table"), "内部 SQL 串泄漏了：{body}");
        assert!(err.detail().contains("存储层操作失败"));
        assert!(err.next_step().contains("quill doctor"));
    }

    #[test]
    fn not_found_is_404_and_builtin_is_403() {
        let nf = agent_error_to_api(
            "读取专家",
            AgentError::ExpertNotFound {
                id: ExpertId::parse("ghost").expect("合法"),
            },
        );
        assert_eq!(nf.status(), axum::http::StatusCode::NOT_FOUND);
        let builtin = agent_error_to_api(
            "删除专家",
            AgentError::ExpertBuiltinProtected {
                id: ExpertId::parse("helper").expect("合法"),
            },
        );
        assert_eq!(builtin.status(), axum::http::StatusCode::FORBIDDEN);
    }
}
