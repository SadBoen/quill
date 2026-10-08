//! 扩展配置的导出 / 导入（`/api/extensions/bundle/*`）—— 对应原始需求 4「多端同步」。
//!
//! **这两条是 quill 自加的路由**：octop 的 `api/routers/` 与 `dashboard/src/` 里
//! 都没有 `bundle`（逐目录核过），所以它们不属于「对齐 octop 契约」那一组，
//! 登记在 `EXTRA_ROUTES`。谁要是把它们挪回 `CONTRACT_ROUTES`，先给出 octop 的出处。
//!
//! **凭据只进不出**：导出**不带** MCP 的 `env` / `headers` 值 —— 那里面装的就是
//! API key 之类的秘密，而 bundle 的用途是拷到另一台机器 / 另一个账号。
//! 与 channels 的 `to_public`、备份的 admin-only 同一条纪律。
//! 响应里 `redacted` 逐条点名剔掉了什么；导入端拿到的是「有这台服务器、没有钥匙」，
//! 要用户自己补。
//!
//! **导入是全量替换 MCP、逐个 upsert 技能**：
//! - MCP 走 `mcp_repo::replace_all`（与 `POST /api/extensions/mcp` 同一条路），
//!   所以「导入的清单里没有的服务器」会被软删 —— 这是同步该有的语义。
//! - 技能正文写到磁盘、行写库，与 `POST /api/extensions/skills` 同一条路。

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::api_experts::{agent_error_to_api, only_keys};
use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::jsonx::need_str;
use crate::state::AppState;

/// 清单格式版本。**不兼容时直接拒**，不要「尽力解析」—— 猜错的清单会写出半套配置，
/// 而用户以为已经同步好了。
const BUNDLE_VERSION: i64 = 1;

const ROOT_FIELDS: &[&str] = &["bundle_version", "mcp_servers", "skills"];

/// 导出里的 MCP 字段。与 `POST /api/extensions/mcp` 认的字段**同源**（少 env/headers，
/// 那两项是凭据）。刻意不带 `enabled` 之外的时间戳 —— 那是本机的落库信息，不是配置。
const MCP_FIELDS: &[&str] = &[
    "name",
    "transport",
    "command",
    "args",
    "cwd",
    "url",
    "enabled",
    "timeout_ms",
    "description",
    "max_concurrent_calls",
    "enabled_capabilities",
];

const SKILL_FIELDS: &[&str] = &[
    "name",
    "version",
    "description",
    "enabled",
    "source",
    "source_ref",
    "tool_allowlist",
    "content",
];

const ALLOWED_TRANSPORTS: [&str; 3] = ["stdio", "streamable_http", "sse"];
const ALLOWED_SOURCES: [&str; 4] = ["builtin", "local", "bundle", "market"];

/// 与 `api_extensions::parse_server` 的默认值**同值**（那里写的是字面量 30_000）。
/// 两处都改成引用同一个常量更干净，但那要动既有路由的解析代码；先这样，
/// 并在两边都留了交叉引用。
const DEFAULT_MCP_TIMEOUT_MS: i64 = 30_000;
const MCP_TIMEOUT_RANGE: std::ops::RangeInclusive<i64> = 1_000..=600_000;

/// `GET /api/extensions/bundle/export`
pub async fn export(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let db = state.db()?;
    let dir = crate::api_extensions::skill_dir(&state.config);

    let mcp = crate::mcp_repo::list(db, user.0.user_id)
        .await
        .map_err(|e| agent_error_to_api("导出 MCP 配置", e))?;
    let skills = crate::skills_repo::list(db, user.0.user_id)
        .await
        .map_err(|e| agent_error_to_api("导出 SKILL", e))?;

    let servers: Vec<Value> = mcp
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "transport": r.transport,
                "command": r.command,
                "args": r.args,
                "cwd": r.cwd,
                "url": r.url,
                "enabled": r.enabled,
                "timeout_ms": r.timeout_ms,
                "description": r.description,
                "max_concurrent_calls": r.max_concurrent_calls,
                "enabled_capabilities": r.enabled_capabilities,
            })
        })
        .collect();

    let mut skill_items = Vec::with_capacity(skills.len());
    for s in &skills {
        // 正文读不到就当空串带上，并在 note 里说清楚 —— 静默省略会让导入端以为
        // 「本来就没正文」。
        let content = crate::api_extensions::skill_file(&dir, &s.name)
            .map(|p| crate::api_extensions::read_skill_body(&p))
            .unwrap_or_default();
        skill_items.push(json!({
            "name": s.name,
            "version": s.version,
            "description": s.description,
            "enabled": s.enabled,
            "source": s.source,
            "source_ref": s.source_ref,
            "content": content,
        }));
    }

    Ok(Json(json!({
        "bundle_version": BUNDLE_VERSION,
        "mcp_servers": servers,
        "skills": skill_items,
        "redacted": ["mcp_servers[].env", "mcp_servers[].headers"],
        "note": "MCP 的 env / headers **不在导出里**（凭据只进不出）。\
                 导到新机器后要在那里重新填这两项，否则那台服务器连不上。",
    })))
}

/// `POST /api/extensions/bundle/import`
pub async fn import(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    only_keys(&body, ROOT_FIELDS, "POST /api/extensions/bundle/import")?;

    let version = body
        .get("bundle_version")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "缺少 bundle_version（应为 {BUNDLE_VERSION}）。\
                 下一步：用 GET /api/extensions/bundle/export 导出一份再改，不要手拼清单。"
            ))
        })?;
    if version != BUNDLE_VERSION {
        return Err(ApiError::bad_request(format!(
            "bundle_version = {version} 不认识（本版只认 {BUNDLE_VERSION}）。\
             下一步：用**同一版** quill 导出，或先升级这个实例。\
             拒绝「尽力解析」是有意的：猜错的清单会写出半套配置。"
        )));
    }

    let servers = body
        .get("mcp_servers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let skills = body
        .get("skills")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let db = state.db()?;

    // —— MCP：全量替换（与 POST /api/extensions/mcp 同一条路）——
    let mut parsed = Vec::with_capacity(servers.len());
    for (i, s) in servers.iter().enumerate() {
        only_keys(s, MCP_FIELDS, &format!("mcp_servers[{i}]"))?;
        parsed.push(mcp_row_from(s, i)?);
    }
    let mcp_after = crate::mcp_repo::replace_all(db, user.0.user_id, parsed)
        .await
        .map_err(|e| agent_error_to_api("导入 MCP 配置", e))?;

    // —— 技能：正文落盘 + 行 upsert ——
    let dir = crate::api_extensions::skill_dir(&state.config);
    std::fs::create_dir_all(&dir).map_err(|e| {
        ApiError::internal(format!(
            "创建技能目录 {} 失败：{e}。下一步：确认该目录可写，\
             或用 QUILL_SKILL_DIR 指向一个可写目录。",
            dir.display()
        ))
    })?;
    let mut skill_results = Vec::with_capacity(skills.len());
    for (i, s) in skills.iter().enumerate() {
        only_keys(s, SKILL_FIELDS, &format!("skills[{i}]"))?;
        let name = need_str(s, "name", &format!("skills[{i}]"))?;
        let content = s
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let chars = content.chars().count();
        if chars > crate::skills_repo::MAX_SKILL_CHARS {
            return Err(ApiError::bad_request(format!(
                "skills[{i}]（{name}）正文太长（{chars} 字符，上限 {}）。\
                 下一步：在**导出那台机器**上把它拆短，再重新导出。",
                crate::skills_repo::MAX_SKILL_CHARS
            )));
        }
        let path = crate::api_extensions::skill_file(&dir, &name)?;
        crate::api_extensions::write_skill_body(&path, &content)?;

        let source = s
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("bundle")
            .to_string();
        if !ALLOWED_SOURCES.contains(&source.as_str()) {
            return Err(ApiError::bad_request(format!(
                "skills[{i}].source = {source:?} 不在枚举里（builtin / local / bundle / market）。"
            )));
        }
        let row = crate::skills_repo::SkillRow {
            name: name.clone(),
            version: s
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or("0.1.0")
                .to_string(),
            source,
            source_ref: s
                .get("source_ref")
                .and_then(Value::as_str)
                .map(str::to_string),
            description: s
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            enabled: s.get("enabled").and_then(Value::as_bool).unwrap_or(false),
            install_path: path.display().to_string(),
            // 老 bundle 里的 `tool_allowlist` **接受并忽略**：这一列 2026-10-09 已删
            // （Q102），而它从来只被写、没被读过 —— 忽略它，行为与删之前**一模一样**，
            // 所以老 bundle 不会因为多一个键就导不进来。
            created_at: crate::db::now_ms(),
            updated_at: crate::db::now_ms(),
        };
        let hash = crate::skills_repo::content_hash(&content);
        let saved = crate::skills_repo::upsert(db, user.0.user_id, row, hash)
            .await
            .map_err(|e| agent_error_to_api("导入 SKILL", e))?;
        skill_results.push(json!({ "name": saved.name, "chars": chars }));
    }

    Ok(Json(json!({
        "bundle_version": BUNDLE_VERSION,
        "mcp_servers": mcp_after.iter().map(crate::mcp_repo::to_json).collect::<Vec<Value>>(),
        "skills": skill_results,
        "note": "MCP 是全量替换：清单里没有的服务器已被软删。\
                 技能逐条 upsert。凭据（env / headers）不在清单里，需在本机重新填。",
    })))
}

fn str_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn mcp_row_from(v: &Value, i: usize) -> Result<crate::mcp_repo::McpServerRow, ApiError> {
    let where_ = format!("mcp_servers[{i}]");
    let name = need_str(v, "name", &where_)?;
    let transport = v
        .get("transport")
        .and_then(Value::as_str)
        .unwrap_or("stdio")
        .to_string();
    if !ALLOWED_TRANSPORTS.contains(&transport.as_str()) {
        return Err(ApiError::bad_request(format!(
            "{where_}.transport = {transport:?} 不在枚举里（stdio / streamable_http / sse）。"
        )));
    }
    let now = crate::db::now_ms();
    let timeout_ms = match v.get("timeout_ms") {
        Some(Value::Null) | None => DEFAULT_MCP_TIMEOUT_MS,
        Some(raw) => {
            let n = raw.as_i64().ok_or_else(|| {
                ApiError::bad_request(format!(
                    "{where_}.timeout_ms 必须是整数。下一步：填 1000~600000 之间的毫秒数，\
                     或省略用默认的 {DEFAULT_MCP_TIMEOUT_MS}。"
                ))
            })?;
            if !MCP_TIMEOUT_RANGE.contains(&n) {
                return Err(ApiError::bad_request(format!(
                    "{where_}.timeout_ms={n} 超出范围（1000..600000 毫秒）。"
                )));
            }
            n
        }
    };
    Ok(crate::mcp_repo::McpServerRow {
        name,
        transport,
        command: v.get("command").and_then(Value::as_str).map(str::to_string),
        args: str_list(v, "args"),
        // 凭据永远是空的：导出不带、导入也不接受。
        env: Vec::new(),
        url: v.get("url").and_then(Value::as_str).map(str::to_string),
        headers: Vec::new(),
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        timeout_ms,
        description: v
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        cwd: v.get("cwd").and_then(Value::as_str).map(str::to_string),
        max_concurrent_calls: v.get("max_concurrent_calls").and_then(Value::as_i64),
        enabled_capabilities: v
            .get("enabled_capabilities")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            }),
        created_at: now,
        updated_at: now,
    })
}
