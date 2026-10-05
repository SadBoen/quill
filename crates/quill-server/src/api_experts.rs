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

/// 工具层用：按同一套可见性规则列出专家。
///
/// 与 `list` 处理器**必须走同一条路径**（`registry` + `list_visible`），
/// 否则工具就成了绕过专家可见性的后门 —— 模型能列出用户界面上看不到的专家。
pub fn list_for_tools(
    state: &crate::state::AppState,
    uid: quill_adapters::UserId,
) -> Result<Vec<Expert>, String> {
    let reg = registry(state).map_err(|e| e.to_string())?;
    reg.list_visible(&uid).map_err(|e| e.to_string())
}

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
        &[
            "id",
            "display_name",
            "description",
            "instructions",
            "model",
            "source_template",
        ],
        "POST /api/experts",
    )?;
    let new = NewExpert {
        id: expert_id(&body, "id")?,
        display_name: need_str(&body, "display_name")?,
        description: need_str(&body, "description")?,
        instructions: opt_str(&body, "instructions")?.unwrap_or_default(),
        model: opt_str(&body, "model")?,
        // 不做存在性校验：模板库是前端静态 vendor 进来的，后端没有模板表可查。
        source_template: opt_str(&body, "source_template")?,
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
        &[
            "display_name",
            "description",
            "instructions",
            "model",
            "source_template",
            "default_enabled",
        ],
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
    if let Some(text) = opt_str(&body, "instructions")? {
        map_agent_error(
            "改专家人格正文",
            registry.set_instructions(&owner, &id, text).map(|_| ()),
        )?;
        touched = true;
    }
    // `model` 必须是「键是否存在」而不是「值是否为 null」来决定改不改：
    // 显式 null = 清除偏好模型回落到实例默认模型，缺省 = 沿用现值。
    if let Some(raw) = body.get("model") {
        let next = match raw {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => {
                return Err(ApiError::bad_request(format!(
                    "字段 model 必须是字符串或 null，实际收到 {}。\
                     下一步：传模型名表示该专家固定用它，传 null 表示跟随实例默认模型。",
                    type_name(other)
                )))
            }
        };
        map_agent_error(
            "改专家偏好模型",
            registry.set_model(&owner, &id, next).map(|_| ()),
        )?;
        touched = true;
    }
    // source_template 同样是「键是否存在」：省略 = 不改，显式 null = 清除来源模板。
    if let Some(raw) = body.get("source_template") {
        let next = match raw {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => {
                return Err(ApiError::bad_request(format!(
                    "字段 source_template 必须是字符串或 null，实际收到 {}。\
                     下一步：传模板 id（如 ai-coding-coach）表示这个专家派生自该模板，\
                     传 null 表示它不来自任何模板。",
                    type_name(other)
                )))
            }
        };
        map_agent_error(
            "改专家来源模板",
            registry.set_source_template(&owner, &id, next).map(|_| ()),
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
             可改字段：display_name（字符串）、description（字符串）、instructions（字符串）、\
             model（字符串或 null）、source_template（字符串或 null）、default_enabled（布尔）。"
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
        "instructions": e.instructions(),
        // 显式 null 而不是省略键：前端靠 `"model" in expert` 判断字段是否被支持，
        // 省略键会被误当成「后端还没做这个字段」。
        "model": e.model(),
        "source_template": e.source_template(),
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
    only_keys_at(body, allowed, &format!("路由 {route}"))
}

/// `label` 是「这里是什么」的自然说法：可以是路由，也可以是
/// 「第 2 项的 MCP 服务器」这种容器内的位置。
pub(crate) fn only_keys_at(
    body: &Value,
    allowed: &[&str],
    label: &str,
) -> Result<(), ApiError> {
    let Some(map) = body.as_object() else {
        return Err(ApiError::bad_request(
            "请求体必须是 JSON 对象。\n\
             下一步：确认发来的是 `{{\"键\": 值}}` 这样的对象，而不是数组或字符串。"
                .to_string(),
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
            "{label} 不接受字段 {:?}。可接受字段：{}。\
             字段名拼错如果被静默忽略，用户会以为自己配了、其实没生效。\
             下一步：删掉这些字段，或改用上面列出的名字。",
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
        AgentError::ExpertInstructionsInvalid { reason, .. } => ApiError::bad_request(format!(
            "人格正文不合法：{reason}。\
             下一步：把 instructions 压到 20000 个字符以内（删掉重复的例句、示例输出），\
             或把长文沉淀成技能文件、在 instructions 里只留指向它的说明。"
        )),
        AgentError::ExpertModelInvalid { reason, .. } => ApiError::bad_request(format!(
            "偏好模型不合法：{reason}。\
             下一步：要么填模型服务 /v1/models 里列出的真实模型名，要么传 null / 省略 model \
             让它跟随实例默认模型。"
        )),
        AgentError::ExpertSourceTemplateInvalid { raw, reason } => ApiError::bad_request(format!(
            "来源模板不合法：{reason}。你填的是「{raw}」。\
             下一步：传模板库里的模板 id（小写字母、数字与连字符组成，最长 64 个字符，\
             例如 ai-coding-coach），或传 null / 省略 source_template 表示这个专家不来自模板。"
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
    fn not_found_is_404_and_builtin_is_403() {        let nf = agent_error_to_api(
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

    #[test]
    fn persona_validation_errors_are_400_with_a_chinese_fix_direction() {
        for (e, want) in [
            (
                AgentError::ExpertInstructionsInvalid {
                    raw: "人".repeat(20_001),
                    reason: "超过 20000 个字符",
                },
                "instructions",
            ),
            (
                AgentError::ExpertModelInvalid {
                    raw: "  ".into(),
                    reason: "trim 后为空串",
                },
                "model",
            ),
            (
                AgentError::ExpertSourceTemplateInvalid {
                    raw: "AI Coding".into(),
                    reason: "含非法字符",
                },
                "source_template",
            ),
        ] {
            let err = agent_error_to_api("改专家人格", e);
            assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
            let body = format!("{}{}", err.detail(), err.next_step());
            assert!(
                body.contains("下一步："),
                "人格类错误必须带修复方向：{body}"
            );
            assert!(body.contains(want), "应点名出错的字段 {want}：{body}");
        }
    }

    #[test]
    fn expert_json_always_emits_model_key_even_when_it_is_null() {
        let e = Expert::user_authored_with_persona(
            quill_adapters::UserId::from_bytes([1u8; 16]),
            ExpertId::parse("cost-analyst").expect("合法"),
            "成本分析师",
            "算清成本",
            "先问口径",
            None,
        )
        .expect("应可构造");
        let v = expert_json(&e);
        let obj = v.as_object().expect("必须是对象");
        assert!(obj.contains_key("model"), "model 为 None 时也必须出现该键");
        assert!(v["model"].is_null(), "None 必须序列化成 JSON null");
        assert_eq!(v["instructions"], serde_json::json!("先问口径"));
        assert!(
            obj.contains_key("source_template"),
            "source_template 为 None 时也必须出现该键（否则前端会以为后端没做这字段）"
        );
        assert!(v["source_template"].is_null());
    }

    #[test]
    fn expert_json_carries_the_source_template_through() {
        let e = Expert::user_authored_with_source(
            quill_adapters::UserId::from_bytes([1u8; 16]),
            ExpertId::parse("prog-1").expect("合法"),
            "程序员1号",
            "从模板派生",
            "",
            None,
            Some("ai-coding-coach".to_string()),
        )
        .expect("应可构造");
        assert_eq!(
            expert_json(&e)["source_template"],
            serde_json::json!("ai-coding-coach")
        );
    }
}
