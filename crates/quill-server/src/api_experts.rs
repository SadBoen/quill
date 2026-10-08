use std::collections::BTreeSet;

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use quill_adapters::{ExpertId, UserId};
use quill_agent::{AgentError, Expert, ExpertRegistry, ExpertRepository, NewExpert};

use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::experts_repo::SqlxExpertRepository;
use crate::jsonx::{need_str, opt_str, type_name};
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
    // 先保证「通用专家」存在，再列。
    // 不这么做的话，一个从没建过专家的用户打开界面只会看到
    // 「还没有「默认启用」的角色」——而对话页恰恰要求**至少有一个角色可选**。
    let _ = crate::general_expert::ensure(&state, user.0.user_id)?;
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
        id: expert_id(&body, "id", "专家请求")?,
        display_name: need_str(&body, "display_name", "专家请求")?,
        description: need_str(&body, "description", "专家请求")?,
        instructions: opt_str(&body, "instructions", "专家请求")?.unwrap_or_default(),
        model: opt_str(&body, "model", "专家请求")?,
        // 不做存在性校验：模板库是前端静态 vendor 进来的，后端没有模板表可查。
        source_template: opt_str(&body, "source_template", "专家请求")?,
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
    if let Some(name) = opt_str(&body, "display_name", "专家请求")? {
        map_agent_error("改专家名", registry.rename(&owner, &id, name).map(|_| ()))?;
        touched = true;
    }
    if let Some(desc) = opt_str(&body, "description", "专家请求")? {
        map_agent_error(
            "改专家描述",
            registry.redescribe(&owner, &id, desc).map(|_| ()),
        )?;
        touched = true;
    }
    if let Some(text) = opt_str(&body, "instructions", "专家请求")? {
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

// ---------------------------------------------------------------------------
// 专家清单导出 / 导入（Q031 / Q032）
//
// 形状与纪律照 `api_bundle`（扩展配置的导出 / 导入）：
// - 带版本字段，不兼容直接拒，不「尽力解析」；
// - 导出不带内部列，并用 `redacted` 逐条点名；
// - 导入逐条给结果，不静默成功也不静默失败。
//
// 与 `api_bundle` 的两处**有意不同**（下面各自的 doc 注释里也写了）：
// 1. 导入是逐条 upsert，**不是全量替换** —— 专家是用户内容，一次导入不该
//    因为「清单里没有」就删掉别的专家；
// 2. 导入**接受**导出产出的 `redacted` / `note` 说明字段 —— 导出的东西必须
//    能原样喂回导入，这才叫对称（`api_bundle` 把这两个键当未知字段拒掉，
//    是那边的一处不对称，不照抄）。
// ---------------------------------------------------------------------------

/// 专家清单格式版本。**不兼容时直接拒**，不要「尽力解析」—— 猜错的清单会
/// 写出半套专家，而用户以为已经同步好了。（口径与 `api_bundle::BUNDLE_VERSION` 同。）
const EXPERTS_BUNDLE_VERSION: i64 = 1;

/// 清单根字段。`redacted` / `note` 是导出产的说明字段，导入端原样接受。
const BUNDLE_ROOT_FIELDS: &[&str] = &["bundle_version", "experts", "redacted", "note"];

/// 清单里一条专家的全部字段 —— 导出写出哪些，导入就只认哪些。
/// 单元测试 `exported_shape_and_accepted_fields_are_the_same_set` 钉住这个集合
/// 与 `expert_bundle_json` 实际产出的键**完全相同**（对称判据的机器可跑版本）。
const EXPERT_FIELDS: &[&str] = &[
    "id",
    "display_name",
    "description",
    "instructions",
    "model",
    "source_template",
    "default_enabled",
];

/// 导出刻意**不带**的表内列（`experts` 表里真实存在，但不是能搬走的配置）：
/// - `owner_user_id`：导入时归属**调用方**自己，清单不携带归属；
/// - `asset_hash` / `persona_hash`：内部指纹列（`sql_put` 里只对 id 取摘要，
///   全仓库没有读取方），既不该外流也无处可用；
/// - `visibility` / `is_builtin` / `deleted_at`：由 schema 与写入路径保证的
///   不变量（自建专家恒为 user_authored / is_builtin=0 / deleted_at=NULL），
///   清单无从覆盖；
/// - `version` / `created_at` / `updated_at`：本机的落库信息，不是配置
///   （与 `api_bundle` 刻意不带 MCP 时间戳同一条纪律）。
const EXPERT_REDACTED: &[&str] = &[
    "experts[].owner_user_id",
    "experts[].visibility",
    "experts[].is_builtin",
    "experts[].deleted_at",
    "experts[].version",
    "experts[].asset_hash",
    "experts[].persona_hash",
    "experts[].created_at",
    "experts[].updated_at",
];

const EXPORT_NOTE: &str = "只带当前用户自己的专家：内置系统专家与已软删的专家都不在清单里。\
     归属（owner_user_id）不在清单里 —— 导入到哪个账号就归哪个账号。";

const IMPORT_NOTE: &str = "逐条 upsert，**不是全量替换**：清单里有的一次性写入\
     （同名覆盖、软删复活），清单里没有的专家不动（与 MCP 设置包的全量替换语义不同）。\
     status 四值：created / updated / skipped（与库中逐字段相同，未重写）/ \
     failed（只有这一条没进去，后面的条目继续处理）。只要有一项 failed，ok=false。";

/// `GET /api/experts/export`
///
/// 只导出**当前用户自己的、未软删的**专家：
/// - 内置系统专家的 `owner_user_id` 是 `SYSTEM_OWNER`（全零 16 字节），
///   `list_owned(uid)` 按属主隔离，天然不带它们；这里再显式滤一遍
///   `is_builtin`，免得将来有人把内置专家挂到别的属主名下时悄悄外流。
/// - 软删行是墓碑，不是配置：带出去只会在导入端生成一批不可见专家。
pub async fn export(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let registry = registry(&state)?;
    let owned = map_agent_error("导出专家", registry.repo().list_owned(&user.0.user_id))?;
    let experts: Vec<Value> = owned
        .iter()
        .filter(|e| !e.is_deleted() && !e.is_builtin())
        .map(expert_bundle_json)
        .collect();
    Ok(Json(json!({
        "bundle_version": EXPERTS_BUNDLE_VERSION,
        "experts": experts,
        "redacted": EXPERT_REDACTED,
        "note": EXPORT_NOTE,
    })))
}

/// `POST /api/experts/import`
///
/// 逐条 upsert：清单里有的一次性写入（同名覆盖、软删复活），清单里没有的**不动**。
/// 写入复用 `ExpertRepository::put`（即 `experts_repo::PUT_SQL` 那条 upsert，
/// `ON CONFLICT (owner_user_id, id) DO UPDATE`），**不另写一条 SQL**：
/// 「覆盖谁、保什么」的口径全仓库只有这一处。
pub async fn import(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    only_keys(&body, BUNDLE_ROOT_FIELDS, "POST /api/experts/import")?;
    let version = body
        .get("bundle_version")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "缺少 bundle_version（应为 {EXPERTS_BUNDLE_VERSION}）。\
                 下一步：用 GET /api/experts/export 导出一份再改，不要手拼清单。"
            ))
        })?;
    if version != EXPERTS_BUNDLE_VERSION {
        return Err(ApiError::bad_request(format!(
            "bundle_version = {version} 不认识（本版只认 {EXPERTS_BUNDLE_VERSION}）。\
             下一步：用**同一版** quill 导出，或先升级这个实例。\
             拒绝「尽力解析」是有意的：猜错的清单会写出半套专家。"
        )));
    }
    let items = body
        .get("experts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ApiError::bad_request(
                "缺少 experts 数组（形状必须与 GET /api/experts/export 的 experts 一致）。\
                 下一步：用导出的清单，不要手拼；空清单也要显式写 \"experts\": []。"
                    .to_string(),
            )
        })?;

    let registry = registry(&state)?;
    let owner = user.0.user_id;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut results: Vec<Value> = Vec::with_capacity(items.len());
    let mut summary = ItemsSummary::default();
    for (i, item) in items.iter().enumerate() {
        let where_ = format!("experts[{i}]");
        let outcome = match import_one(&registry, owner, item, &where_, &mut seen) {
            Ok(o) => o,
            Err(ItemError::Rejected(reason)) => ItemOutcome {
                id: None,
                status: ItemStatus::Failed,
                reason,
            },
            // 存储层 / 不变量故障不是一个条目的问题：整批以 5xx 结束，不逐条
            // 假装「只是这一条没导进去」。已写入的条目不回滚，但重试是幂等的
            // （逐条 upsert），照常重试即可。
            Err(ItemError::Fatal(e)) => return Err(e),
        };
        summary.record(outcome.status);
        results.push(json!({
            "index": i,
            "id": outcome.id,
            "status": outcome.status.as_wire(),
            "reason": outcome.reason,
        }));
    }

    Ok(Json(json!({
        "bundle_version": EXPERTS_BUNDLE_VERSION,
        "ok": summary.failed == 0,
        "results": results,
        "summary": summary.to_json(items.len()),
        "redacted": EXPERT_REDACTED,
        "note": IMPORT_NOTE,
    })))
}

/// 清单里一条专家的形状。**键集合与 `EXPERT_FIELDS` 逐字对应**（单元测试钉住），
/// 显式 null 而不是省略键（与 `expert_json` 同一理由：省略键会被当成
/// 「后端还没做这个字段」）。
pub(crate) fn expert_bundle_json(e: &Expert) -> Value {
    json!({
        "id": e.id().as_str(),
        "display_name": e.display_name(),
        "description": e.description(),
        "instructions": e.instructions(),
        "model": e.model(),
        "source_template": e.source_template(),
        "default_enabled": e.default_enabled(),
    })
}

/// 一条专家的导入结果。
struct ItemOutcome {
    /// 解析出来的专家 id；条目连 id 都不合法时为 None。
    id: Option<String>,
    status: ItemStatus,
    /// 人话：为什么是这个结果。
    reason: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemStatus {
    Created,
    Updated,
    Skipped,
    Failed,
}

impl ItemStatus {
    fn as_wire(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

/// 逐条导入错误。分两类是**有意的**：
/// - `Rejected`：这一条自己有问题（字段缺失 / 非法、清单内重复），记成 failed
///   并继续处理后面的条目 —— 一批里一条坏不该让其余全废；
/// - `Fatal`：存储层 / 数据不变量故障，`agent_error_to_api` 已把内部细节写进
///   日志并换成中文 5xx 文案，整批返回它。
enum ItemError {
    Rejected(String),
    Fatal(ApiError),
}

#[derive(Default)]
struct ItemsSummary {
    created: u32,
    updated: u32,
    skipped: u32,
    failed: u32,
}

impl ItemsSummary {
    fn record(&mut self, status: ItemStatus) {
        match status {
            ItemStatus::Created => self.created += 1,
            ItemStatus::Updated => self.updated += 1,
            ItemStatus::Skipped => self.skipped += 1,
            ItemStatus::Failed => self.failed += 1,
        }
    }

    fn to_json(&self, total: usize) -> Value {
        json!({
            "total": total,
            "created": self.created,
            "updated": self.updated,
            "skipped": self.skipped,
            "failed": self.failed,
        })
    }
}

/// 导入一条。任何返回 `Ok` 的结果都已经**真的落库**（skipped 除外 ——
/// 它明确表示「与库中逐字段相同，没有写」）。
fn import_one(
    registry: &ExpertRegistry<SqlxExpertRepository>,
    owner: UserId,
    item: &Value,
    where_: &str,
    seen: &mut BTreeSet<String>,
) -> Result<ItemOutcome, ItemError> {
    only_keys_at(item, EXPERT_FIELDS, where_).map_err(|e| ItemError::Rejected(e.detail()))?;
    let raw_id = field_str(item, "id", where_)?;
    let id = ExpertId::parse(&raw_id).map_err(|e| {
        ItemError::Rejected(format!(
            "{where_}.id = {raw_id:?} 不是合法专家标识（{e}）：\
             只接受小写字母、数字与连字符组成的 kebab-case，长度 1~64。"
        ))
    })?;
    if !seen.insert(raw_id.clone()) {
        return Err(ItemError::Rejected(format!(
            "{where_}.id = {raw_id:?} 在同一清单里重复出现；只有第一次出现的那条被处理。\
             下一步：删掉重复项再导 —— 两份不同的同名专家谁覆盖谁，导入端不该猜。"
        )));
    }

    let mut expert = Expert::user_authored_with_source(
        owner,
        id.clone(),
        field_str(item, "display_name", where_)?,
        field_str(item, "description", where_)?,
        field_str(item, "instructions", where_)?,
        field_opt_str(item, "model", where_)?,
        field_opt_str(item, "source_template", where_)?,
    )
    .map_err(|e| ItemError::Rejected(agent_error_to_api("导入专家", e).detail()))?;
    if !field_bool(item, "default_enabled", where_)? {
        expert
            .set_default_enabled(&owner, false)
            .map_err(|e| ItemError::Rejected(agent_error_to_api("导入专家", e).detail()))?;
    }
    expert
        .check_cross_invariant()
        .map_err(|e| ItemError::Rejected(agent_error_to_api("导入专家", e).detail()))?;

    // 先看库中现状，再决定 written / skipped —— 复用 put 那条 upsert。
    let existing = registry
        .repo()
        .get(&owner, &id)
        .map_err(|e| ItemError::Fatal(agent_error_to_api("导入专家（读取现状）", e)))?;
    let (status, reason) = match existing {
        // 全字段相等（id / owner / 可见性 / 内置标记 / deleted 也天然相等）→
        // 不白写一次 `updated_at`。skipped 只有这一个含义，且逐条如实上报。
        Some(e) if !e.is_deleted() && e == expert => (
            ItemStatus::Skipped,
            "与库中现有专家逐字段相同，未重写（updated_at 不变）。".to_string(),
        ),
        Some(e) if e.is_deleted() => (
            ItemStatus::Updated,
            "已把此前软删的同名专家按清单恢复（deleted_at → NULL）。".to_string(),
        ),
        Some(_) => (ItemStatus::Updated, "已按清单覆盖同名专家。".to_string()),
        None => (ItemStatus::Created, "已新建。".to_string()),
    };
    if status != ItemStatus::Skipped {
        registry
            .repo()
            .put(&expert)
            .map_err(|e| ItemError::Fatal(agent_error_to_api("导入专家（写入）", e)))?;
    }
    Ok(ItemOutcome {
        id: Some(id.as_str().to_string()),
        status,
        reason,
    })
}

/// 清单逐条字段的取法：**必须显式出现**（导出恒写出全部字段），字符串
/// **原样保留、不 trim** —— 往返判据是「逐字段等价」，导入端擅自 trim 会让
/// 带首尾空白的 description 在往返后变样。
fn field_str(item: &Value, key: &str, where_: &str) -> Result<String, ItemError> {
    match item.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(ItemError::Rejected(format!(
            "{where_}.{key} 必须是字符串，实际收到 {}。",
            type_name(other)
        ))),
        None => Err(ItemError::Rejected(format!(
            "缺少字段 {where_}.{key}：导入只接受导出产出的形状，每个字段都要在\
             （没有值就显式写 null）。"
        ))),
    }
}

fn field_opt_str(item: &Value, key: &str, where_: &str) -> Result<Option<String>, ItemError> {
    match item.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(ItemError::Rejected(format!(
            "{where_}.{key} 必须是字符串或 null，实际收到 {}。",
            type_name(other)
        ))),
        None => Err(ItemError::Rejected(format!(
            "缺少字段 {where_}.{key}：没有值就显式写 null，不要省略键 —— \
             省略会被当成「这版还没支持这个字段」。"
        ))),
    }
}

fn field_bool(item: &Value, key: &str, where_: &str) -> Result<bool, ItemError> {
    match item.get(key) {
        Some(Value::Bool(b)) => Ok(*b),
        Some(other) => Err(ItemError::Rejected(format!(
            "{where_}.{key} 必须是布尔值（true/false），实际收到 {}。",
            type_name(other)
        ))),
        None => Err(ItemError::Rejected(format!(
            "缺少字段 {where_}.{key}：导出清单里这一项恒在；手拼时要显式写 true 或 false。"
        ))),
    }
}

pub(crate) fn registry(state: &AppState) -> Result<ExpertRegistry<SqlxExpertRepository>, ApiError> {
    let db = state.db()?;
    Ok(SqlxExpertRepository::registry(std::sync::Arc::clone(db)))
}

pub(crate) fn expert_json(e: &Expert) -> Value {
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
        // 「哪一个是通用专家」由服务端说了算，前端不许自己认 id ——
        // 把 `general` 硬编码在前端的话，后端换 id 时界面会跟着说谎，
        // 而且没人会发现。显式布尔字段，前端只负责显示。
        "is_general": e.id().as_str() == crate::general_expert::GENERAL_EXPERT_ID,
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

fn expert_id(body: &Value, key: &str, where_: &str) -> Result<ExpertId, ApiError> {
    parse_slug(&need_str(body, key, where_)?)
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
pub(crate) fn only_keys_at(body: &Value, allowed: &[&str], label: &str) -> Result<(), ApiError> {
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

pub fn map_agent_error<T>(op: &str, r: Result<T, AgentError>) -> Result<T, ApiError> {
    r.map_err(|e| agent_error_to_api(op, e))
}

pub fn agent_error_to_api(op: &str, e: AgentError) -> ApiError {
    eprintln!("[api] {op} 失败（错误码 {}）：{e}", e.code());
    match e {
        AgentError::ExpertNotFound { id } | AgentError::ExpertDeleted { id } => {
            ApiError::entity_not_found(format!("专家 {id} 不存在，或对你不可见。"))
        }
        AgentError::ExpertExists { id } => ApiError::conflict(
            format!("专家 {id} 已存在。"),
            "先 `GET` 该专家确认它是否已存在；\
             若要替换请先 `DELETE` 再创建（专家名删除后可复用）。",
        ),
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
        // 「派给一个不在名册里的专家」是**调用方**的错，不是服务端故障。
        // 报 500 会把「你写错了专家名」说成「我们坏了」，而且 detail 里把真实
        // 原因藏进日志、调用方无从下手（2026-10-08 写派工真执行时实测到）。
        AgentError::TeamInvalid(t) => ApiError::bad_request(format!(
            "专家团的数据不合法：{t}。\
             下一步：用 `GET /api/teams/{{id}}` 看这个团的名册，只按名册里的专家派工。"
        )),
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
    fn a_dispatch_to_a_non_member_expert_is_a_client_error_not_a_500() {
        // 调用方写错专家名是**客户端**错误。2026-10-08 写派工真执行时实测到：
        // 原来它落到 `other =>` 兜底变 500，detail 只说「真实原因已写入服务端日志」，
        // 调用方既不知道自己错了、也不知道错在哪。
        let err = agent_error_to_api(
            "执行派工",
            AgentError::TeamInvalid(quill_domain::TeamError::UnknownExpert(
                quill_adapters::ExpertId::parse("ghost-analyst").expect("测试专家名合法"),
            )),
        );
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(
            err.detail().contains("ghost-analyst"),
            "要点名是哪个专家不在名册里：{}",
            err.detail()
        );
        // `bad_request` 没有 advice 槽位，指引写在 detail 里（与其它 bad_request 分支一致）。
        assert!(
            err.detail().contains("GET /api/teams"),
            "要告诉调用方怎么自查名册：{}",
            err.detail()
        );
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

    // ---------------------------------------------------------------
    // Q031 / Q032：清单形状的对称性（判据本身的单元版本）
    // ---------------------------------------------------------------

    fn bundle_fixture() -> Expert {
        Expert::user_authored_with_source(
            quill_adapters::UserId::from_bytes([1u8; 16]),
            ExpertId::parse("cost-analyst").expect("合法"),
            "成本分析师",
            "算清成本口径",
            "先问口径再算",
            Some("qwen3.5".to_string()),
            Some("ai-coding-coach".to_string()),
        )
        .expect("应可构造")
    }

    /// 对称判据的机器可跑版本：`EXPERT_FIELDS`（导入只认这些字段）必须与
    /// `expert_bundle_json` 实际写出的键集合**完全相同**。
    /// 把导出少写一个键、或把导入的 allowed 列表加/减一项，这条立刻红。
    #[test]
    fn exported_shape_and_accepted_fields_are_the_same_set() {
        let v = expert_bundle_json(&bundle_fixture());
        let keys: std::collections::BTreeSet<&str> = v
            .as_object()
            .expect("清单条目必须是对象")
            .keys()
            .map(String::as_str)
            .collect();
        let accepted: std::collections::BTreeSet<&str> = EXPERT_FIELDS.iter().copied().collect();
        assert_eq!(
            keys, accepted,
            "导出形状与导入接受的字段必须是同一集合（对称判据）"
        );
    }

    /// 内部指纹列（asset_hash / persona_hash）与归属、不变量列**不随清单外流**。
    /// 给 `expert_bundle_json` 加回任何一列，这条变红。
    #[test]
    fn the_bundle_never_carries_internal_columns_or_fingerprints() {
        let text = expert_bundle_json(&bundle_fixture()).to_string();
        for forbidden in [
            "asset_hash",
            "persona_hash",
            "owner_user_id",
            "visibility",
            "is_builtin",
            "deleted_at",
            "version",
            "created_at",
            "updated_at",
        ] {
            assert!(
                !text.contains(forbidden),
                "清单条目里出现了内部列 {forbidden}：{text}"
            );
        }
    }

    /// `redacted` 清单必须点名两个指纹列，且与 `EXPERT_FIELDS` 不重叠
    /// （重叠意味着「一边说有、一边收」，说明清单在骗人）。
    #[test]
    fn the_redacted_list_names_the_fingerprints_and_does_not_overlap_the_fields() {
        let leaf = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
        let redacted: Vec<String> = EXPERT_REDACTED.iter().map(|r| leaf(r)).collect();
        for want in ["asset_hash", "persona_hash", "owner_user_id"] {
            assert!(
                redacted.iter().any(|r| r == want),
                "redacted 要点名 {want}：{redacted:?}"
            );
        }
        for f in EXPERT_FIELDS {
            assert!(
                !redacted.iter().any(|r| r == f),
                "字段 {f} 既在导出清单里又在 redacted 里，自相矛盾：{redacted:?}"
            );
        }
    }

    /// 版本常量与 `api_bundle` 的口径一致：只认整数版本，不认字符串。
    /// （导入判定用 `Value::as_i64`，这里钉住「版本是数字」这个契约。）
    #[test]
    fn the_bundle_version_is_an_integer_that_only_the_current_one_is_accepted() {
        assert_eq!(EXPERTS_BUNDLE_VERSION, 1);
        assert_eq!(
            serde_json::json!(EXPERTS_BUNDLE_VERSION).as_i64(),
            Some(EXPERTS_BUNDLE_VERSION),
            "版本必须以整数出现在清单里"
        );
    }
}
