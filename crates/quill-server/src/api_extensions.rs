//! `/api/extensions/*` —— MCP 服务器与 SKILL 的管理入口。
//!
//! **这一层只做配置管理**：把 `mcp_servers` / `skills` 两张表接上真实现，
//! 让界面上那些「点��必失败」的按钮变成能存能读。
//!
//! 刻意**不做**的事：这里不发起任何到 MCP 服务器的连接。理由是保存配置与
//! 连通性是两件事 —— 服务器没起、地址写错、token 过期，都会让「保存」失败，
//! 而用户恰恰需要先把配置登记进去才能去排查。所以响应里带 `connected: false`
//! 与说明，如实告知「已保存、尚未连接验证」，不假装通了。
//!
//! 契约抄前端（`ui/web/src/devices/api.ts`，它抄的是 Octop）：全量提交
//! `{ servers: [...] }`，没有的即软删。

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::mcp_repo::{self, McpServerRow};
use crate::skills_repo;
use crate::state::AppState;

fn map_err(op: &str, e: quill_agent::AgentError) -> ApiError {
    ApiError::internal(format!("{op}失败（详情见服务端日志）：{e}"))
}

/// `GET /api/extensions/mcp` —— 列出当前用户的 MCP 服务器配置。
pub async fn list_mcp(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Response, ApiError> {
    let db = state.db()?;
    let rows = mcp_repo::list(db, user.0.user_id)
        .await
        .map_err(|e| map_err("列出 MCP 服务器", e))?;
    Ok(Json(json!({
        "servers": rows.iter().map(mcp_repo::to_json).collect::<Vec<Value>>(),
        "connected": false,
        "note": "已保存配置，尚未连接验证。\
                 下一步：接 rmcp 协议层后，这里会变成真实的 tools/list 结果。",
    }))
    .into_response())
}

/// `POST /api/extensions/mcp` —— 全量覆盖。
pub async fn save_mcp(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    crate::api_experts::only_keys(&body, &["servers"], "POST /api/extensions/mcp")?;
    let servers = body
        .get("servers")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ApiError::bad_request(
                "servers 必须是数组。下一步：{\"servers\":[{\"name\":\"filesystem\",\"transport\":\"stdio\",\"command\":\"npx\"}]}".to_string(),
            )
        })?
        .clone();

    let mut parsed = Vec::with_capacity(servers.len());
    for (i, item) in servers.iter().enumerate() {
        parsed.push(parse_server(item, i)?);
    }
    // 同一次提交里出现重名会让「差分软删」失去意义：到底留哪个？
    let mut seen = std::collections::BTreeSet::new();
    for s in &parsed {
        let n = s.name.to_lowercase();
        if !seen.insert(n.clone()) {
            return Err(ApiError::bad_request(format!(
                "第 {} 项之后又出现名为「{n}」的服务器。\
                 同一次提交里名字必须唯一 —— 名字是主键的一部分，重复会让 \
                 「哪一条被删」无法判定。\
                 下一步：改掉其中一个名字，或拆成两次提交。",
                parsed.iter().position(|x| x.name.to_lowercase() == n).map(|x| x + 1).unwrap_or(0)
            )));
        }
    }

    let db = state.db()?;
    let rows = mcp_repo::replace_all(db, user.0.user_id, parsed)
        .await
        .map_err(|e| map_err("保存 MCP 服务器", e))?;
    Ok(Json(json!({
        "servers": rows.iter().map(mcp_repo::to_json).collect::<Vec<Value>>(),
        "connected": false,
        "note": "配置已保存。连接验证尚未接入（rmcp 协议层还没落地），\
                 所以这里的 connected 恒为 false —— 这不是失败，是还没做。",
    }))
    .into_response())
}

/// `DELETE /api/extensions/mcp/{name}` —— 软删单个。
pub async fn delete_mcp(
    State(state): State<AppState>,
    user: AuthUser,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let norm = mcp_repo::normalize_name(&name).map_err(ApiError::bad_request)?;
    let db = state.db()?;
    // 走「全量覆盖」实现软删：读出现有、去掉目标、写回。
    let rows = mcp_repo::list(db, user.0.user_id)
        .await
        .map_err(|e| map_err("删除前读取", e))?;
    // 老实说删没删掉：目标本来就不存在时返回 false。报 true 等于凭空
    // 告诉用户「刚删掉了一台服务器」，而实际上什么都没发生。
    let existed = rows.iter().any(|r| r.name == norm);
    let kept: Vec<McpServerRow> = rows.into_iter().filter(|r| r.name != norm).collect();
    let after = mcp_repo::replace_all(db, user.0.user_id, kept)
        .await
        .map_err(|e| map_err("删除 MCP 服务器", e))?;
    Ok(Json(json!({
        "deleted": existed,
        "name": norm,
        "servers": after.iter().map(mcp_repo::to_json).collect::<Vec<Value>>(),
    }))
    .into_response())
}

const ALLOWED_TRANSPORTS: [&str; 3] = ["stdio", "streamable_http", "sse"];

/// 单个 MCP 服务器对象接受的字段。与前端 `McpServerConfig` 一一对应。
///
/// 逐个字段白名单而不是「挑认识的读」：多写一个不认识的键就静默返回 200，
/// 用户会以为那个配置生效了 —— 这类失败要到连不上服务器时才暴露，
/// 而那时候没人想得起来是字段名拼错了。
const SERVER_FIELDS: &[&str] = &[
    "name",
    "transport",
    "command",
    "args",
    "cwd",
    "env",
    "url",
    "headers",
    "enabled",
    "timeout_ms",
    "description",
    "max_concurrent_calls",
    "enabled_capabilities",
];

fn parse_server(v: &Value, idx: usize) -> Result<McpServerRow, ApiError> {
    // 报哪一项的哪个字段，第几项就写进消息里 —— 错误里带定位，
    // 用户不用自己数第几个对象写错了。
    let bad = ApiError::bad_request;

    let obj = v.as_object().ok_or_else(|| {
        bad(format!(
            "第 {idx} 项必须是对象。\
             下一步：写成 {{\"name\":\"…\",\"transport\":\"stdio\",\"command\":\"npx\"}} 这样的对象。"
        ))
    })?;
    crate::api_experts::only_keys_at(v, SERVER_FIELDS, &format!("第 {idx} 项的 MCP 服务器"))?;

    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            bad(format!(
                "第 {idx} 项的 name 必须是字符串。\
                 下一步：给它一个名字，例如 \"filesystem\" —— 名字是主键的一部分。"
            ))
        })?
        .to_string();
    let name = mcp_repo::normalize_name(&name).map_err(bad)?;

    let transport = obj
        .get("transport")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            bad(format!(
                "第 {idx} 项的 transport 必须是字符串。\
                 下一步：填 {} 之一。",
                ALLOWED_TRANSPORTS.join(" / ")
            ))
        })?
        .to_string();
    if !ALLOWED_TRANSPORTS.contains(&transport.as_str()) {
        return Err(bad(format!(
            "第 {idx} 项的 transport（{transport}）不是支持的传输方式。\
             可用：{}。下一步：下拉框里选一个。",
            ALLOWED_TRANSPORTS.join(" / ")
        )));
    }

    let command = obj.get("command").and_then(Value::as_str).map(str::to_string);
    let args = obj
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    let cwd = obj
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let url = obj
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let env = kv(obj.get("env"), &format!("第 {idx} 项的 env（stdio 专用）"))?;
    let headers = kv(obj.get("headers"), &format!("第 {idx} 项的 headers（http/sse 专用）"))?;

    // 传输方式与必填字段对不上时**在这里**报错，而不是丢给数据库 CHECK ——
    // 数据库只说 "CHECK constraint failed"，用户无从知道是哪台服务器的哪个字段。
    if transport == "stdio" {
        if command.as_deref().unwrap_or("").trim().is_empty() {
            return Err(bad(format!(
                "第 {idx} 项的 command 必填：stdio 传输要启动一个本地进程。\
                 下一步：填上可执行文件名，例如 `npx` 或 `uvx`。"
            )));
        }
        if url.is_some() {
            return Err(bad(format!(
                "第 {idx} 项的 url 不该出现在 stdio 配置上：stdio 连的是本地进程，没有地址。\
                 下一步：清空 url，或把传输方式改成 streamable_http / sse。"
            )));
        }
    } else {
        if url.as_deref().unwrap_or("").is_empty() {
            return Err(bad(format!(
                "第 {idx} 项的 url 必填：{transport} 要连一个 HTTP 地址。\
                 下一步：填上完整地址，例如 https://example.com/mcp"
            )));
        }
        if command.is_some() {
            return Err(bad(format!(
                "第 {idx} 项的 command 不该出现在 {transport} 配置上：它连的是远端，不在本机起进程。\
                 下一步：清空 command，并把命令要带的参数挪到 headers 之外的正确位置。"
            )));
        }
    }

    let max_concurrent_calls = match obj.get("max_concurrent_calls") {
        Some(Value::Null) | None => None,
        Some(v) => {
            let n = v.as_i64().ok_or_else(|| {
                bad(format!(
                    "第 {idx} 项的 max_concurrent_calls 必须是整数。\
                     下一步：填 1~64 之间的数字，或留空表示用默认值。"
                ))
            })?;
            if !(1..=64).contains(&n) {
                return Err(bad(format!(
                    "第 {idx} 项的 max_concurrent_calls={n} 超出范围（1..64）。\
                     下一步：留空表示用默认值。"
                )));
            }
            Some(n)
        }
    };

    let enabled_capabilities = match obj.get("enabled_capabilities") {
        Some(Value::Null) | None => None,
        Some(Value::Array(a)) => {
            let mut out: Vec<String> = Vec::new();
            for x in a {
                let sval = x.as_str().ok_or_else(|| {
                    bad(format!(
                        "第 {idx} 项的 enabled_capabilities 的元素必须是字符串。\
                         下一步：写成 [\"read\", \"write\"] 这样的一串字符串。"
                    ))
                })?;
                if !out.iter().any(|y| y == sval) {
                    out.push(sval.to_string());
                }
            }
            Some(out)
        }
        Some(_) => {
            return Err(bad(format!(
                "第 {idx} 项的 enabled_capabilities 必须是 null（全部禁用）或数组\
                 （空数组=全部启用）。下一步：想全开就传 []，想全禁就传 null。"
            )))
        }
    };

    let timeout_ms = match obj.get("timeout_ms") {
        Some(Value::Null) | None => 30_000,
        Some(v) => {
            let n = v.as_i64().ok_or_else(|| {
                bad(format!(
                    "第 {idx} 项的 timeout_ms 必须是整数。\
                     下一步：填 1000~600000 之间的毫秒数，或留空用默认的 30000。"
                ))
            })?;
            if !(1000..=600_000).contains(&n) {
                return Err(bad(format!(
                    "第 {idx} 项的 timeout_ms={n} 超出范围（1000..600000 毫秒）。\
                     下一步：连不上时把它调大。"
                )));
            }
            n
        }
    };

    Ok(McpServerRow {
        name,
        transport,
        command,
        args,
        env,
        url,
        headers,
        enabled: obj.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        timeout_ms,
        description: obj
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        cwd,
        max_concurrent_calls,
        enabled_capabilities,
        created_at: 0,
        updated_at: 0,
    })
}

fn kv(v: Option<&Value>, label: &str) -> Result<Vec<(String, String)>, ApiError> {
    let Some(v) = v else { return Ok(Vec::new()) };
    if v.is_null() {
        return Ok(Vec::new());
    }
    let m = v.as_object().ok_or_else(|| {
        ApiError::bad_request(format!(
            "{label} 必须是对象（键值对），例如 {{\"TOKEN\":\"xxx\"}}。\
             下一步：写成字符串到字符串的映射。"
        ))
    })?;
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, val) in m {
        // 值统一按字符串存：HTTP header 与环境变量的值在协议上都是字符串，
        // 前端发来 number 的话存成 "3" 比报错更合理（用户看到的是同一个框）。
        out.push((
            k.clone(),
            val.as_str().map(str::to_string).unwrap_or_else(|| val.to_string()),
        ));
    }
    out.sort();
    Ok(out)
}

// ------------------------------------------------------------------ SKILL

/// SKILL 正文目录。优先 `QUILL_SKILL_DIR`，否则用数据库同级的 `skills/`。
///
/// `pub(crate)` 是因为 `tools::ToolRegistry::with_skills` 也要按同一个目录读正文 ——
/// 挂进工具表时如果自己另算一份路径，界面上「已保存」与模型「能调用」就会
/// 悄悄指向两个地方。
pub(crate) fn skill_dir(cfg: &crate::config::Config) -> std::path::PathBuf {
    if let Ok(d) = std::env::var("QUILL_SKILL_DIR") {
        if !d.trim().is_empty() {
            return std::path::PathBuf::from(d);
        }
    }
    cfg.db_path
        .parent()
        .map(|p| p.join("skills"))
        .unwrap_or_else(|| std::path::PathBuf::from("data/skills"))
}

/// 把 slug 映射成安全的文件名。
///
/// **路径穿越防护**：slug 来自 URL/body，直接拼进路径就能用 `../` 跳出目录，
/// 写到任意位置。归一后仍要确认最终路径落在根目录内 —— 归一规则被绕过时
/// 这道检查是最后一道。
fn skill_file(root: &std::path::Path, slug: &str) -> Result<std::path::PathBuf, ApiError> {
    let name = mcp_repo::normalize_name(slug).map_err(ApiError::bad_request)?;
    let path = root.join(format!("{name}.md"));
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let parent = path.parent().unwrap_or(root).to_path_buf();
    if !parent.starts_with(&canon_root) {
        return Err(ApiError::bad_request(format!(
            "SKILL 名称 {slug:?} 解析后落在技能目录之外，已拒绝。\
             下一步：只用小写字母、数字与连字符。"
        )));
    }
    Ok(path)
}

/// 读 SKILL 正文。**读不到就当空**，由调用方决定这算「没配」还是「坏了」——
/// 静默变成空串会让「文件被删了」看起来像「用户没写内容」。
pub(crate) fn read_skill_body(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn write_skill_body(path: &std::path::Path, content: &str) -> Result<(), ApiError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| {
            ApiError::internal(format!("创建 SKILL 目录 {} 失败：{e}", dir.display()))
        })?;
    }
    std::fs::write(path, content)
        .map_err(|e| ApiError::internal(format!("写入 SKILL 正文 {} 失败：{e}", path.display())))
}

/// `GET /api/extensions/skills` —— 列出 SKILL。
pub async fn list_skills(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Response, ApiError> {
    let db = state.db()?;
    let rows = skills_repo::list(db, user.0.user_id)
        .await
        .map_err(|e| map_err("列出 SKILL", e))?;
    let dir = skill_dir(&state.config);
    let mut items = Vec::with_capacity(rows.len());
    for r in &rows {
        let mut j = skills_repo::to_json(r);
        let body = read_skill_body(&dir.join(format!("{}.md", r.name)));
        // 正文长度让前端能显示「这个技能多大」，也让我们能如实标出
        // 「文件在库里但磁盘上没找到」的情况。
        if let Some(obj) = j.as_object_mut() {
            obj.insert("content_chars".into(), json!(body.chars().count()));
            if body.is_empty() {
                obj.insert("content_missing".into(), json!(true));
            }
        }
        items.push(j);
    }
    Ok(Json(json!({ "skills": items })).into_response())
}

/// `POST /api/extensions/skills` —— 新建或更新一个 SKILL。
///
/// 正文会落盘成 `<slug>.md`，与 Octop 的 `skills/{slug}/SKILL.md` 同构。
pub async fn save_skill(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    crate::api_experts::only_keys(
        &body,
        &[
            "slug",
            "name",
            "description",
            "content",
            "version",
            "source",
            "enabled",
            "tool_allowlist",
        ],
        "POST /api/extensions/skills",
    )?;

    let raw_name = body
        .get("slug")
        .or_else(|| body.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError::bad_request("slug 必填。\u{0a}下一步：{\"slug\":\"code-review\",\"content\":\"…\"}")
        })?;
    let dir = skill_dir(&state.config);
    std::fs::create_dir_all(&dir)
        .map_err(|e| ApiError::internal(format!("创建技能目录 {} 失败：{e}", dir.display())))?;
    let path = skill_file(&dir, raw_name)?;

    let content = body
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if content.trim().is_empty() {
        return Err(ApiError::bad_request(
            "content 不能为空 —— 一个没有正文的 SKILL 等于没有。\n\
             下一步：写清楚这套方法在什么场景下用、怎么做。"
                .to_string(),
        ));
    }
    let chars = content.chars().count();
    if chars > skills_repo::MAX_SKILL_CHARS {
        return Err(ApiError::bad_request(format!(
            "content 太长（{chars} 字符，上限 {}）。\
             下一步：拆成多个 SKILL，或把说明压到上限内 —— \
             超长的正文会挤占上下文，模型也读不完。",
            skills_repo::MAX_SKILL_CHARS
        )));
    }

    let source = body
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("local")
        .to_string();
    if !["builtin", "local", "bundle", "market"].contains(&source.as_str()) {
        return Err(ApiError::bad_request(format!(
            "source={source:?} 不合法。可用：builtin / local / bundle / market。\n\
             下一步：用户自己写的就是 local。"
        )));
    }

    let slug = mcp_repo::normalize_name(raw_name).map_err(ApiError::bad_request)?;
    write_skill_body(&path, &content)?;

    let row = skills_repo::SkillRow {
        name: slug.clone(),
        version: body
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("0.1.0")
            .to_string(),
        source,
        source_ref: None,
        description: body
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
        enabled: body.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        install_path: path.display().to_string(),
        tool_allowlist: body
            .get("tool_allowlist")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default(),
        created_at: 0,
        updated_at: 0,
    };

    let db = state.db()?;
    let saved = skills_repo::upsert(db, user.0.user_id, row, skills_repo::content_hash(&content))
        .await
        .map_err(|e| map_err("保存 SKILL", e))?;

    let mut j = skills_repo::to_json(&saved);
    if let Some(o) = j.as_object_mut() {
        o.insert("content_chars".into(), json!(chars));
    }
    Ok(Json(json!({ "skill": j })).into_response())
}

/// `DELETE /api/extensions/skills/{name}` —— 软删，并移除磁盘正文。
pub async fn delete_skill(
    State(state): State<AppState>,
    user: AuthUser,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let dir = skill_dir(&state.config);
    let norm = mcp_repo::normalize_name(&name).map_err(ApiError::bad_request)?;
    // 走与写入同一个路径解析器，不要另拼一条。`norm` 已经归一过，
    // 这里再过一次是为了让「路径怎么算出来的」只有一处实现。
    let path = skill_file(&dir, &norm)?;

    let db = state.db()?;
    let deleted = skills_repo::soft_delete(db, user.0.user_id, &norm)
        .await
        .map_err(|e| map_err("删除 SKILL", e))?;

    // 正文文件删不掉的两种情况都无害：本来就不存在，或目录不可写。
    // 行已经软删了，留下一个孤儿 .md 不影响任何功能，下次同名重建会覆盖它。
    let removed_file = std::fs::remove_file(&path).is_ok();

    Ok(Json(json!({
        "deleted": deleted,
        "name": norm,
        "file_removed": removed_file,
    }))
    .into_response())
}
