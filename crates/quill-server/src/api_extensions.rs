//! `/api/extensions/*` —— MCP 服务器与 SKILL 的管理入口。
//!
//! **这一层管两件事**：把 `mcp_servers` / `skills` 两张表接上真实现，
//! 以及在读 MCP 配置时**真的去连一次**（`mcp_client::probe`）。
//!
//! 仍然刻意**不**做的事：不让「保存配置」依赖「连得上」。保存配置与连通性是两件事 ——
//! 服务器没起、地址写错、token 过期，都会让「保存」失败，而用户恰恰需要先把配置
//! 登记进去才能去排查。所以写入只碰库，连接只发生在读的那一侧，并且**读也是尽力而为**：
//! 一台连不上的服务器会让 `status` 里那一条报错，但不会让整个请求 500。
//!
//! 响应里 `connected` 与 `note` 是**实测**出来的，不是配置的回声。2026-10-06 之前
//! 这两个字段恒为 `false` 并附一句「rmcp 协议层还没落地」；协议层落地之后它们随
//! 每次真实握手变化，那句说明也随之改掉。见 `mcp_client`。
//!
//! 契约抄前端（`ui/web/src/devices/api.ts`，它抄的是 Octop）：全量提交
//! `{ servers: [...] }`，没有的即软删。**只读的状态另放一个 `status` 数组**，
//! 不混进 `servers` —— 前端会把 `servers` 原样回填进编辑表单再 POST 回来，
//! 混进只读字段会被 `parse_server` 的字段白名单拒掉。

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::mcp_client;
use crate::mcp_repo::{self, McpServerRow};
use crate::skills_repo;
use crate::state::AppState;

fn map_err(op: &str, e: quill_agent::AgentError) -> ApiError {
    ApiError::internal(format!("{op}失败（详情见服务端日志）：{e}"))
}

/// `GET /api/extensions/mcp` —— 列出当前用户的 MCP 服务器配置，**并真的连一次**。
///
/// 每台服务器都会走一遍 `mcp_client::probe`：启用且是 stdio 的会真的被拉起来、
/// 真的握手、真的 `tools/list`。连不上的那台在 `status` 里带一句白话原因，
/// 但**不会**让整个请求失败 —— 一台用户的服务器起不来，不该让整页 MCP 打不开。
pub async fn list_mcp(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Response, ApiError> {
    let db = state.db()?;
    let rows = mcp_repo::list(db, user.0.user_id)
        .await
        .map_err(|e| map_err("列出 MCP 服务器", e))?;
    Ok(Json(mcp_body(&state, user.0.user_id, &rows).await).into_response())
}

/// 拼一份响应。`GET` 与 `POST` 共用。
///
/// **POST 也要重测一次**，不能直接回 `connected: false` —— 前端保存成功后会用
/// POST 的响应替换掉列表缓存（`client.setQueryData`）。这里回一个恒假的
/// `connected`，用户刚存好的服务器在界面上会立刻显示成「没连上」，
/// 而那不是任何一次真实探测的结果。
///
/// **「连上了」与「模型能调」分开报。** `status` 里每台带 `mounted`：
/// 这一轮真的挂进对话工具表的工具条数，以及每个工具的**挂载名 ↔ 原名**映射。
/// 只报 `tool_count`（服务器自己报了几个）的话，界面上就会出现
/// 「3 个工具可用」而模型那轮一个都调不到 —— 配对没上、能力被关掉、
/// 与内置工具撞名，这三种原因长得一模一样。
async fn mcp_body(state: &AppState, uid: quill_adapters::UserId, rows: &[McpServerRow]) -> Value {
    // **停用的服务器不握手，但要在 `status` 里如实出现。**
    //
    // 与 `tools::with_mcp_tools` 用**同一个** `tools::enabled_servers` 过滤 ——
    // 两边各判一次，迟早漂；漂了就变成「界面说 mounted=3、模型一个都调不到」，
    // 那正是本项目最不能出的那种不一致（ISSUE-014 那一类）。
    //
    // `servers` 仍然返回**全部**行（含停用的），因为前端要靠它把编辑表单回填、
    // 再 POST 回来；这里就把停用的藏起来的话，用户在界面上再也开不回来。
    let active = crate::tools::enabled_servers(rows.to_vec());
    // **持有 String 而不是 &str**：`active` 紧接着要 move 进 `discover_all`，
    // 借它的切片会活不过那次 move。
    let active_names: Vec<String> = active.iter().map(|r| r.name.clone()).collect();

    // **只握手一次。** 状态与挂载名都从这同一批 `Discovery` 里出 ——
    // 分两次握手的话，用户配的进程会被拉起来两遍，而两遍的工具列表可能不一样，
    // 于是界面上「已挂载」那一栏会和自己都算不上稳定的第二次握手对不上。
    let found = mcp_client::discover_all(active).await;
    let probes: Vec<mcp_client::Probe> = found.iter().map(|d| d.probe.clone()).collect();
    let summary = mcp_client::Summary::of(&probes);
    // 挂载口径与 `with_mcp_tools` 共用同一套判断（`tools::mcp_tool_visibility`），
    // 基线工具表也用 `tools::baseline_specs` 建 —— 界面上报的必须是**真的**挂了什么，
    // 而不是「如果挂的话大概会挂什么」。
    let mounted = mount_from(state, uid, &found).await;
    let mut status: Vec<Value> = found
        .iter()
        .map(|d| {
            let mut v = mcp_client::status_json(&d.probe);
            let m = mounted.iter().find(|m| m.server == d.probe.name);
            if let Some(obj) = v.as_object_mut() {
                let (tools, skipped) = match m {
                    Some(m) => (m.tools.clone(), m.skipped.clone()),
                    None => (Vec::new(), Vec::new()),
                };
                obj.insert("mounted".into(), json!(tools.len()));
                obj.insert("mounted_tools".into(), json!(tools));
                obj.insert("not_mounted".into(), json!(skipped));
            }
            v
        })
        .collect();
    // 停用的那些：**如实报「已停用」**，而不是把它们从 status 里抹掉 ——
    // 抹掉的话界面上这台服务器就没了，用户会以为它被删了。
    for row in rows.iter().filter(|r| !active_names.iter().any(|n| n == &r.name)) {
        status.push(json!({
            "name": row.name,
            "probed": false,
            "connected": false,
            "tool_count": 0,
            "mounted": 0,
            "mounted_tools": [],
            "not_mounted": [],
            "disabled": true,
            "error": "这台服务器已被停用：这一轮不握手、也不会挂任何工具进对话。\
                      下一步：要重新启用，在编辑里把「启用」打开并保存。",
            "protocol_version": Value::Null,
            "server_info": Value::Null,
            "server_declares_tools": Value::Null,
        }));
    }
    let total_mounted: usize = mounted.iter().map(|m| m.tools.len()).sum();
    let mut note = summary.note(&probes);
    if total_mounted > 0 {
        note.push_str(&format!(
            " 这一轮真的挂进对话工具表的有 {total_mounted} 个，模型调得到（挂载名与原名的对应见每行的「已挂载」）。"
        ));
    } else if summary.connected > 0 {
        note.push_str(
            " 但一个都没挂上：逐条原因见每行的「未挂载」。\
             下一步：按那一条的白话处理，处理完刷新这一页再看。",
        );
    }
    json!({
        "servers": rows.iter().map(mcp_repo::to_json).collect::<Vec<Value>>(),
        "status": status,
        "connected": summary.all_connected(),
        "probed": summary.probed,
        "connected_count": summary.connected,
        "failed_count": summary.failed,
        "mounted_count": total_mounted,
        "note": note,
    })
}

/// 一台服务器这一轮**真的**挂上了哪些工具，以及没挂上的那些卡在哪。
struct MountedFor {
    server: String,
    /// 挂上去的工具：`(挂载名, 远端原名)`。
    tools: Vec<(String, String)>,
    /// 没挂上的：`(远端原名, 白话原因)`。
    skipped: Vec<(String, String)>,
}

/// 从**已经握过一次手**的结果里判挂载。**不再发起任何连接。**
async fn mount_from(
    state: &AppState,
    uid: quill_adapters::UserId,
    found: &[mcp_client::Discovery],
) -> Vec<MountedFor> {
    let Ok(db) = state.db() else {
        return Vec::new();
    };
    let dir = skill_dir(&state.config);
    // 基线 = 内置 + SKILL。与 `with_mcp_tools` 的前半段完全一致，所以
    // 「已被占住的名字」在两处是同一份 —— 否则界面上会说一个工具挂上了，
    // 而对话里它被内置工具顶掉了。
    let mut taken = match crate::tools::baseline_specs(
        std::sync::Arc::new(state.clone()),
        db.as_ref(),
        uid,
        &dir,
    )
    .await
    {
        Ok(specs) => specs,
        Err(why) => {
            eprintln!("[mcp] 建基线工具表失败，界面不报挂载数：{why}");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for d in found {
        let mut tools = Vec::new();
        let mut skipped = Vec::new();
        for tool in &d.tools {
            match crate::tools::mcp_tool_visibility(&taken, &d.probe.name, tool) {
                crate::tools::McpToolVisibility::Visible => {
                    let spec = crate::tools::mcp_tool_spec(&d.probe.name, tool);
                    tools.push((spec.name.clone(), tool.remote_name.clone()));
                    // 挂上的占住这个名字，后面的要比对它 —— 与 `with_mcp_tools`
                    // 里 `register` 之后 `self.specs` 变长的效果一致。
                    taken.push(spec);
                }
                crate::tools::McpToolVisibility::NotMounted(why) => {
                    skipped.push((tool.remote_name.clone(), why.to_string()));
                }
            }
        }
        out.push(MountedFor {
            server: d.probe.name.clone(),
            tools,
            skipped,
        });
    }
    out
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
    Ok(Json(mcp_body(&state, user.0.user_id, &rows).await).into_response())
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

// ------------------------------------------------------------------ 技能市场

/// 装进来的技能在 `skills.source` 里存的值。
///
/// **不能是 `"skillhub"`。** 0001 迁移的 CHECK 是
/// `source IN ('builtin','local','bundle','market')` —— 第一版写死
/// `"skillhub"`，于是每次安装都撞 CHECK 变成 500。错误信息倒是很诚实
/// （把整条约束打了出来），但那条错误里没有一个字在说「你该填 market」。
pub const HUB_SOURCE: &str = "market";

#[cfg(test)]
mod hub_error_tests {
    use super::{hub_error, SKILLHUB_ADVICE};
    use crate::skillhub::HubError;

    #[test]
    fn a_missing_skill_is_a_404_not_a_service_outage() {
        // 真机实测：上游对不存在的 slug 回 404。第一版把它归成 503，
        // 界面上就显示成「技能市场没连上」—— 而市场明明好好地回了话。
        let e = hub_error("下载技能 nope 失败", HubError::Status(404));
        assert_eq!(e.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_bad_slug_from_the_caller_is_a_400_not_a_5xx() {
        let e = hub_error("请求不合法", HubError::Input("技能标识含斜杠".into()));
        assert_eq!(e.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn a_genuine_outage_says_what_to_do_about_the_market_and_nothing_about_the_model() {
        // 这条是 ISSUE-054 的钉子：之前挂的是 `ProviderUnavailable`，
        // 它的「下一步」让用户去 curl 模型端点、启动 llama-server。
        let e = hub_error("读技能市场列表失败", HubError::Fetch("timeout".into()));
        assert_eq!(e.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(e.code(), "upstream_unavailable");
        assert_eq!(e.next_step(), SKILLHUB_ADVICE);
        assert!(!SKILLHUB_ADVICE.contains("llama-server"));
        assert!(!SKILLHUB_ADVICE.contains("QUILL_LLM_BASE_URL"));
        // 下一步要真的提到那个能救它的开关。
        assert!(SKILLHUB_ADVICE.contains("QUILL_SKILLHUB_HOST"));
    }

    #[test]
    fn the_detail_keeps_the_actual_reason_next_to_the_generic_advice() {
        let e = hub_error("读技能市场列表失败", HubError::Parse("期望 skillSets".into()));
        let d = e.detail();
        assert!(d.contains("读技能市场列表失败"), "{d}");
        assert!(d.contains("skillSets"), "真正的原因要留着：{d}");
    }
}

#[cfg(test)]
mod hub_source_tests {
    use super::HUB_SOURCE;

    /// 测试用的上游 manifest 样例（结构取自 2026-10-06 的真实响应）。
    const REAL_MANIFEST: &str = r#"{
      "slug": "tech-test-automation",
      "displayName": "自动化测试",
      "skillSlugs": ["superpowers-tdd", "test-case-generator"]
    }"#;

    #[test]
    fn the_source_value_is_one_the_schema_actually_accepts() {
        // 与 0001 迁移里那条 CHECK 一一对应。改了这条约束就要同步改这里，
        // 而**先撞它的人应该是测试**，不是用户在真机上点安装。
        assert!(
            ["builtin", "local", "bundle", "market"].contains(&HUB_SOURCE),
            "{HUB_SOURCE:?} 不在 skills.source 的 CHECK 允许值里"
        );
    }

    #[test]
    fn an_installed_skill_is_named_after_the_package_not_the_file_inside_it() {
        // 实测（2026-10-06）：`tech-test-automation` 的 zip 里那篇正文叫
        // `identify.md`。按文件名装出来技能就叫 `identify` —— 这个名字
        // 既看不懂，又会直接出现在对话工具表里让模型调（tool_name 就是 slug）。
        // 一个包 = 一个技能，名字必须是那个包。
        let manifest = crate::skillhub::parse_manifest(REAL_MANIFEST)
            .expect("manifest 应当能解析");
        let file_name = "identify.md";
        let chosen = manifest.slug.trim().to_string();
        assert_eq!(chosen, "tech-test-automation");
        assert_ne!(
            chosen,
            file_name.trim_end_matches(".md"),
            "绝不能退回包内文件名"
        );
    }
}

/// `GET /api/extensions/skill-hub` —— 列出技能市场（SkillHub）里的技能集。
///
/// **上游是外部服务**（默认 `https://api.skillhub.cn`，可用 `QUILL_SKILLHUB_HOST` 改）。
/// 抄 Octop 的 `skill-packages/hub/search` 与 `/hub/rankings` 背后那套。
///
/// 上游挂了就说挂了。**绝不能返回空列表** ——
/// 「市场连不上」与「市场里没有技能」在界面上是两件完全不同的事，
/// 而空列表这句话会让用户以为是后者，然后跑去怀疑自己的技能包。
pub async fn skill_hub_list(
    Query(q): Query<HubListQuery>,
) -> Result<Response, ApiError> {
    let page = match crate::skillhub::list_skillsets(q.page, q.page_size).await {
        Ok(p) => p,
        Err(e) => {
            // 503 而不是 502：本项目把「上游连不上」与「上游回错」都归到 503
            // （见 error.rs 里的显式取舍），另开一个 502 只会与整套错误语义不一致。
            // 但**不能**用 `service_unavailable` —— 它那句「下一步」谈的是
            // 模型服务，见 `hub_error` 的说明。
            return Err(hub_error("读技能市场列表失败", e));
        }
    };
    Ok(Json(json!({
        "host": crate::skillhub::host(),
        "items": page.items,
        "total": page.total,
        "page": page.page,
        "page_size": page.page_size,
    }))
    .into_response())
}

#[derive(serde::Deserialize)]
pub struct HubListQuery {
    #[serde(default = "one")]
    pub page: u32,
    #[serde(default = "fifty")]
    pub page_size: u32,
}fn one() -> u32 {
    1
}
fn fifty() -> u32 {
    50
}

// ---------------------------------------------------------------------------
// 单技能：搜索、榜单、安装
// ---------------------------------------------------------------------------

/// 技能市场出错的统一说法。
///
/// 为什么要一个 helper：这些错误散在 6 个 handler 里，各写一遍的话，
/// 迟早有一处挂上**说错话**的下一步 —— 而那正是用户唯一会照着做的东西。
/// 真机已经踩过一次：技能 404 的「下一步」写着「执行 curl $QUILL_LLM_BASE_URL/models
/// 确认端点活着」，把用户引向一台与技能市场毫无关系的机器。见 ISSUE-054。
///
/// `what` 是**这次失败在做什么**（「读技能市场列表」「下载技能 xxx」），
/// 拼在错误前面好让用户知道是哪一步坏的。
fn hub_error(what: &str, e: crate::skillhub::HubError) -> ApiError {
    use crate::skillhub::HubError;
    match e {
        // 调用方给错了值 → 400。**说「上游坏了」会把用户引去查一个没坏的东西。**
        HubError::Input(detail) => ApiError::bad_request(format!("{what}：{detail}")),
        // 上游说「没有这个东西」→ 404，不是 503。
        // 它与「连不上」在界面上是两件事，见 ISSUE-055。
        HubError::Status(404) => ApiError::entity_not_found(format!(
            "{what}：技能市场里没有这个东西（上游返回 404），它可能已被下架。"
        )),
        other => ApiError::upstream_unavailable(
            format!("{what}：{}", other.message()),
            SKILLHUB_ADVICE,
        ),
    }
}

/// 技能市场这一类错误的「下一步」。
///
/// **不能复用 `ProviderUnavailable` 那句** —— 它谈的是模型服务与
/// llama-server，而这里坏的是另一个外部服务。
const SKILLHUB_ADVICE: &str =
    "技能市场（SkillHub）是外部服务，本机没有它的副本。\
     先确认网络能到上游；若是自建或镜像的市场，用 `QUILL_SKILLHUB_HOST` \
     指向可达的地址后重启 quill-server。已安装的技能不受影响，\
     它们本来就在本机磁盘上。";

#[derive(serde::Deserialize)]
pub struct HubSkillQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default = "fifty")]
    pub limit: u32,
}

/// `GET /api/extensions/skill-hub/skills` —— 搜单技能。
///
/// 与 [`skill_hub_list`]（技能包）是**两件事**，不是同一个列表的两种叫法。
/// 上游实测：技能包 56 个，单技能搜索 `pdf` 出来 5 个，两边 slug 各不相同。
pub async fn skill_hub_search(
    Query(q): Query<HubSkillQuery>,
) -> Result<Response, ApiError> {
    let items = match crate::skillhub::search_skills(&q.q, q.limit).await {
        Ok(v) => v,
        Err(e) => return Err(hub_error("搜技能市场失败", e)),
    };
    Ok(Json(json!({
        "host": crate::skillhub::host(),
        "query": q.q,
        "items": items,
        // **没有 total**：上游的 search 就是一个数组。
        // 拿 items.len() 当「共 N 个」是在编一个总数。
        "total": null,
    }))
    .into_response())
}

#[derive(serde::Deserialize)]
pub struct HubRankQuery {
    #[serde(default = "recommended")]
    pub kind: String,
}

fn recommended() -> String {
    "recommended".to_string()
}

/// `GET /api/extensions/skill-hub/rankings?kind=` —— 上游的推荐/热门榜单。
///
/// `kind=all` 是**我们**的聚合词（上游没有这个端点），走并发拉全部再合并。
/// 部分榜单没拉到时，成功的那部分照常返回、失败的记进 `errors` ——
/// 全部失败才算连不上。
pub async fn skill_hub_rankings(
    Query(q): Query<HubRankQuery>,
) -> Result<Response, ApiError> {
    if q.kind == "all" {
        let all = match crate::skillhub::showcase_all().await {
            Ok(v) => v,
            Err(e) => return Err(hub_error("拉技能市场榜单失败", e)),
        };
        let errors: serde_json::Map<String, serde_json::Value> = all
            .errors
            .iter()
            .map(|(k, m)| (k.clone(), serde_json::Value::String(m.clone())))
            .collect();
        return Ok(Json(json!({
            "host": crate::skillhub::host(),
            "kind": "all",
            "section": null,
            "items": all.merged(),
            "sections": all.sections.iter().map(|s| json!({
                "kind": s.kind,
                "section": s.section,
                "count": s.items.len(),
            })).collect::<Vec<_>>(),
            "errors": errors,
            "kinds": crate::skillhub::SHOWCASE_KINDS,
        }))
        .into_response());
    }

    let board = match crate::skillhub::showcase_skills(&q.kind).await {
        Ok(v) => v,
        Err(e) => return Err(hub_error("拉技能市场榜单失败", e)),
    };
    Ok(Json(json!({
        "host": crate::skillhub::host(),
        "kind": board.kind,
        // 上游自己给这一份榜单起的名字，如 `hot_downloads`。原样透传。
        "section": board.section,
        "items": board.items,
        "errors": {},
        "kinds": crate::skillhub::SHOWCASE_KINDS,
    }))
    .into_response())
}

/// `POST /api/extensions/skill-hub/skills/{slug}/install` —— 装一个单技能。
///
/// ## 技能名必须是 slug，不能是包里的文件名
///
/// 实测 `pdf-image-text-extractor` 的包里有 `SKILL.md`、`README.md`、
/// `README.en.md` 三个 `.md`。按文件名命名会装出一个叫 `skill` 的技能 ——
/// 这个名字会直接出现在对话工具表里让模型调（`tool_name` 就是 slug）。
/// 包的标识只有一个权威答案：**URL 里那个 slug**。
pub async fn skill_hub_install_skill(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    let safe = crate::skillhub::validate_slug(&slug)
        .map_err(|e| hub_error("请求不合法", e))?;

    let bytes = match crate::skillhub::download_skill(&safe).await {
        Ok(b) => b,
        Err(e) => return Err(hub_error(&format!("下载技能 {safe} 失败"), e)),
    };

    // 单技能用**另一套**解包规则，见 `skillhub_unpack::PackageKind`。
    let unpacked = crate::skillhub_unpack::unpack_skill(&bytes)
        .map_err(|e| ApiError::bad_request(e.message()))?;

    let body = unpacked
        .files
        .first()
        .map(|(_, b)| b.clone())
        .ok_or_else(|| ApiError::bad_request("这个包里没有技能正文。".to_string()))?;
    // 正文来自包里哪个文件，要报给用户 —— 换了名字也得能追到。
    let body_file = unpacked
        .files
        .first()
        .map(|(n, _)| n.clone())
        .unwrap_or_default();

    let dir = skill_dir(&state.config);
    std::fs::create_dir_all(&dir)
        .map_err(|e| ApiError::internal(format!("创建技能目录 {} 失败：{e}", dir.display())))?;

    let name = mcp_repo::normalize_name(&safe).map_err(ApiError::bad_request)?;
    let path = skill_file(&dir, &name)?;

    let db = state.db()?;
    let saved = skills_repo::upsert(
        db,
        user.0.user_id,
        skills_repo::SkillRow {
            name: name.clone(),
            version: "0.1.0".to_string(),
            source: HUB_SOURCE.to_string(),
            source_ref: Some(safe.clone()),
            description: hub_description(&body),
            // 装完不启用。理由与技能包那条一样，见 skill_hub_install 的文档。
            enabled: false,
            install_path: path.display().to_string(),
            tool_allowlist: Vec::new(),
            created_at: 0,
            updated_at: 0,
        },
        skills_repo::content_hash(&body),
    )
    .await
    .map_err(|e| map_err("登记技能", e))?;

    if let Err(e) = std::fs::write(&path, &body) {
        return Err(ApiError::internal(format!(
            "技能 {name} 的正文写不进 {}：{e}。下一步：检查技能目录的写权限，\
             或用 QUILL_SKILL_DIR 指向一个可写目录。",
            path.display()
        )));
    }

    Ok(Json(json!({
        "installed": skills_repo::to_json(&saved),
        "installed_count": 1,
        "source_slug": safe,
        // 包里那个文件被当成了正文。**说出来**，别让人以为装的是 SKILL.md。
        "body_file": body_file,
        "skipped_other": unpacked.skipped_other,
        "compressed_bytes": unpacked.compressed_bytes,
        "uncompressed_bytes": unpacked.uncompressed_bytes,
        "enabled": false,
    }))
    .into_response())
}

/// `POST /api/extensions/skill-hub/{slug}/install` —— 把一个技能集装进本地技能目录。
///
/// ## 这条路的风险在哪
///
/// 上游给的是**任意 zip 字节**。所以顺序不能反：
/// 先过完 [`crate::skillhub_unpack`] 的全部检查（条目数、解压总量、压缩比、
/// 路径穿越），**再**落盘。任何一条检查不过就整包拒绝 ——
/// 不能「装到一半发现不对」然后留半个技能在目录里。
///
/// ## 装完默认是停用的
///
/// 安装**不启用**。启用会立刻给每一轮请求加上它的摘要（ISSUE-036：
/// 实测 13 个技能就把 8192 窗口顶爆），而「装一个包」这件事
/// 本身不蕴含「现在就让它进上下文窗口」。要不要开由用户在技能包里看一眼再点。
pub async fn skill_hub_install(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    let safe = crate::skillhub::validate_slug(&slug)
        .map_err(|e| hub_error("请求不合法", e))?;

    let bytes = match crate::skillhub::download_skillset(&safe).await {
        Ok(b) => b,
        Err(e) => return Err(hub_error(&format!("下载技能包 {safe} 失败"), e)),
    };

    let unpacked = crate::skillhub_unpack::unpack(&bytes)
        .map_err(|e| ApiError::bad_request(e.message()))?;

    // 上游的包里有一份 manifest（实测 2026-10-06）：
    // `tech-test-automation` 的 zip 只有 `manifest.json` 与 `identify.md` 两个条目，
    // 而 manifest 里列的 6 个子技能（superpowers-tdd、test-case-generator…）
    // **只有 slug 与一句简介，没有各自的正文**。
    //
    // 所以这一包能装进 quill 的就是**一个**技能（编排说明本身），
    // 其余 6 个是「包里点名要用的下游技能」。**如实说成一个**，
    // 装完在界面上写「1 个技能、另有 6 个待取」—— 装成 6 个就是编数据。
    let manifest: Option<crate::skillhub::HubManifest> = unpacked
        .files
        .iter()
        .find(|(name, _)| name == "manifest.json")
        .and_then(|(_, body)| crate::skillhub::parse_manifest(body));

    let referenced: Vec<String> = manifest
        .as_ref()
        .map(|m| m.referenced_slugs())
        .unwrap_or_default();

    let dir = skill_dir(&state.config);
    std::fs::create_dir_all(&dir)
        .map_err(|e| ApiError::internal(format!("创建技能目录 {} 失败：{e}", dir.display())))?;

    let db = state.db()?;
    let mut installed: Vec<Value> = Vec::new();

    for (name, body) in &unpacked.files {
        // manifest.json 不是技能，是元数据。**不算进「装了几个」**。
        if name == "manifest.json" {
            continue;
        }
        // 技能名优先取**包的 slug**，不是包内文件名。
        //
        // 实测：`tech-test-automation` 的 zip 里那篇正文叫 `identify.md`，
        // 按文件名装出来就叫 `identify` —— 这个名字既看不懂，也会直接
        // 出现在对话工具表里让模型调（`tool_name` 就是 slug）。
        // 一个包 = 一个技能，名字就该是那个包。
        let stem = manifest
            .as_ref()
            .map(|m| m.slug.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| name.trim_end_matches(".md").to_string());
        // 归一失败（上游给了个带斜杠的 slug）时退回文件名，而不是整包失败 ——
        // `normalize_name` 的错误信息里没有「换个名字继续」这句话。
        let slug_row = match mcp_repo::normalize_name(&stem) {
            Ok(n) => n,
            Err(_) => {
                mcp_repo::normalize_name(name.trim_end_matches(".md"))
                    .map_err(ApiError::bad_request)?
            }
        };
        // 落盘路径只允许 `skill_file` 这一处计算（`sanitize_name` 已在解包时挡过穿越）。
        let path = skill_file(&dir, &slug_row)?;

        // 正文哈希算的是**真要落盘的这几行**，不是下载下来的 zip ——
        // 这两者的哈希本来就不该相等。
        let saved = skills_repo::upsert(
            db,
            user.0.user_id,
            skills_repo::SkillRow {
                name: slug_row.clone(),
                version: "0.1.0".to_string(),
                // **`market` 不是 `skillhub`**：见 `HUB_SOURCE` 的说明。
                source: HUB_SOURCE.to_string(),
                source_ref: Some(safe.clone()),
                description: hub_description(body),
                // **装完不启用。** 见函数文档。
                enabled: false,
                install_path: path.display().to_string(),
                tool_allowlist: Vec::new(),
                created_at: 0,
                updated_at: 0,
            },
            skills_repo::content_hash(body),
        )
        .await
        .map_err(|e| map_err("登记技能", e))?;

        if let Err(e) = std::fs::write(&path, body) {
            // 行已经写了而文件没写成 = 一个「库里有行、正文不在」的技能。
            // 与其悄悄留着（`skill_visibility` 会把它标成 not_mounted，
            // 用户看到的是一句莫名其妙的「磁盘上没有正文」），
            // 不如在这里说清楚到底哪一步失败了。
            return Err(ApiError::internal(format!(
                "技能 {slug_row} 的正文写不进 {}：{e}。下一步：检查技能目录的写权限，\
                 或用 QUILL_SKILL_DIR 指向一个可写目录。",
                path.display()
            )));
        }
        installed.push(skills_repo::to_json(&saved));
    }

    // 一个都没装上时不能报成功。空数组 + 200 会被界面读成「装好了」。
    if installed.is_empty() {
        return Err(ApiError::bad_request(format!(
            "这个技能包里没有能装的技能（{} 个条目全是元数据或非 .md）。\
             下一步：换一个技能包，或联系市场方补上正文。",
            unpacked.files.len()
        )));
    }

    Ok(Json(json!({
        "installed": installed,
        "source": "skillhub",
        "source_slug": safe,
        "display_name": manifest.as_ref().and_then(|m| m.display_name()),
        // **装到几个就说几个。** manifest 里点名的子技能只有 slug 与简介，
        // 没有正文，所以它们**不在** installed 里 —— 把它们算进去就是编数据。
        "installed_count": installed.len(),
        "skipped_other": unpacked.skipped_other,
        "referenced_not_installed": referenced,
        "compressed_bytes": unpacked.compressed_bytes,
        "uncompressed_bytes": unpacked.uncompressed_bytes,
        // 明说「装完是停用的」。装完就自动生效，是另一种越权。
        "enabled": false,
    }))
    .into_response())
}

/// 从技能正文里取 frontmatter 的 `description`，没有就留空。///
/// **留空是有意的**：`skills_repo::skill_summary` 在 description 为空时会
/// 退回正文开头，模型照样看得见。而这里编一句摘要，只会让用户看到一句
/// 我们自己造的说明。
fn hub_description(body: &str) -> String {
    let head = body.splitn(3, "---").nth(1).unwrap_or("");
    for line in head.lines() {
        if let Some(rest) = line.trim().strip_prefix("description:") {
            return rest
                .trim()
                .trim_matches(['"', '\''])
                .trim_start_matches(">-")
                .trim()
                .lines()
                .collect::<Vec<_>>()
                .join(" ");
        }
    }
    String::new()
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
///
/// 每条都带 `model_can_see`：**这次对话里模型到底看不看得见它**。
/// 「库里有一行」与「模型看得见」是两件事 —— 停用的、正文文件被删的，
/// 界面上都还列得出来，但工具表里没有它。不报这一项，界面就会显示
/// 「已启用」而模型压根不知道有这个技能，没有任何迹象指向原因。
///
/// 判断走 `tools::skill_visibility` —— 与 `ToolRegistry::with_skills` 同一个函数。
/// 两处各判一次迟早会漂，漂了就成了「界面说一套、对话做另一套」。
pub async fn list_skills(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Response, ApiError> {
    let db = state.db()?;
    let rows = skills_repo::list(db, user.0.user_id)
        .await
        .map_err(|e| map_err("列出 SKILL", e))?;
    let dir = skill_dir(&state.config);
    // 内置工具名是「已占住」的名字。SKILL 之间的重名在当前 schema 下不存在
    // （`UNIQUE(user_id, name)`），所以拿内置这一份就够判定。
    let builtin_names: Vec<quill_provider::ToolSpec> = crate::tools::ToolRegistry::builtin(
        std::sync::Arc::new(state.clone()),
        user.0.user_id,
    )
    .specs();

    let mut items = Vec::with_capacity(rows.len());
    let mut taken = builtin_names;
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
            // **模型这一轮真正看到的那段字**。界面上算常驻开销只能用它，
            // 不能用上面那个 `description` 列 —— 两者在 description 为空时
            // 分叉：库里那一列是空的，`skill_summary` 却会退回正文开头一段。
            // 拿空列去算，结果是「116 个技能 · 常驻开销 0 字符」：
            // 一个既好看又危险的说法，它让人以为可以随便开，而模型每轮
            // 实际都在吃这些正文开头（ISSUE-035/036 那个 8192 窗口就是这么
            // 被顶爆的）。**宁可难看也不许说反。**
            obj.insert(
                "model_sees_summary".into(),
                json!(skills_repo::skill_summary(r, &body)),
            );
            let vis = crate::tools::skill_visibility(&taken, r, &body);
            obj.insert("model_can_see".into(), json!(vis.model_can_see()));
            if let crate::tools::SkillVisibility::NotMounted(why) = vis {
                obj.insert("not_mounted_reason".into(), json!(why));
            }
            // 挂上的话它就占住这个名字，后面的 SKILL 要跟它比对。
            if vis.model_can_see() {
                taken.push(crate::skills_repo::as_tool_spec(r, &body));
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

/// `PATCH /api/extensions/skills/{name}` —— 只改开关，不碰正文。
///
/// ## 为什么必须是这一条
///
/// `tools::skill_visibility` 第一句就是 `if !row.enabled { Disabled }`：
/// **停用的技能不挂进对话工具表，模型这一轮根本调不到它。**
/// 而 `save_skill`（POST）要连正文一起重传 —— 拿它当「启停」用，
/// 就得先读回全文、改一个布尔、再整体写回去。正文几千上万字，
/// 一次误操作就是把技能写坏了。
///
/// 真机上就是这个状态：库里所有技能 `enabled = false`，一个都没挂上，
/// 而**界面上没有任何地方能改这个值**（前端此前一次都没调用过
/// `/api/extensions/skills`）。后端做完了、开关没出口，功能等于没做。
///
/// 只接受 `enabled` 一个字段：其余字段改动一律 400，
/// 免得有人以为 PATCH 能改名字或描述，悄悄把一个技能改成了另一个。
pub async fn update_skill(
    State(state): State<AppState>,
    user: AuthUser,
    Path(name): Path<String>,
    JsonBody(patch): JsonBody,
) -> Result<Response, ApiError> {
    let obj = patch.as_object().ok_or_else(|| {
        ApiError::bad_request(
            "请求体必须是 JSON 对象。下一步：{\"enabled\": true} 或 {\"enabled\": false}。".to_string(),
        )
    })?;

    // `only_keys_at` 会把多余的键逐个点名，而不是只说「参数不对」。
    crate::api_experts::only_keys_at(&patch, &["enabled"], "SKILL 的 PATCH")?;

    let enabled = obj
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            ApiError::bad_request(
                "enabled 必须是 true 或 false。下一步：{\"enabled\": true} 可以让模型这一轮调到它，\
                 {\"enabled\": false} 会把它从对话工具表里摘掉。"
                    .to_string(),
            )
        })?;

    let norm = mcp_repo::normalize_name(&name).map_err(ApiError::bad_request)?;
    let db = state.db()?;

    // 走只写一列的专用更新：复用 `upsert` 会把 `content_hash` 换成
    // 调用方手里的值（这里没有真正的正文哈希），还会顺手把 `deleted_at`
    // 重置成 NULL —— 一次「只想启用」的点击就会改坏别的列。
    // 详见 `skills_repo::set_enabled` 的说明。
    let changed = skills_repo::set_enabled(db, user.0.user_id, &norm, enabled)
        .await
        .map_err(|e| map_err("更新 SKILL", e))?;

    if !changed {
        // 区分「没有这个技能」和「有但已经软删」：后者 refresh 后自然就没了，
        // 前者多半是名字打错。两种都给得出下一步，所以不用同一个笼统的 404。
        // 用 `entity_not_found` 而不是 `not_found`：后者说的是「**这条路径**
        // 没有登记」，会把「本实例没有路由」拼在 detail 前面，句子直接不通。
        // 这里是「路径存在、但没有这个技能」，两件事必须分开说。
        // 顺带一提，这正是本项目那条规矩的又一次应验：
        // 404 只能说明路径没登记 —— 不能拿它当「资源不存在」的下场。
        return Err(ApiError::entity_not_found(format!(
            "名为 {norm} 的 SKILL 不存在或已删除，所以这次开关没有改动任何东西。\
             下一步：刷新列表确认名字；要新建先 POST /api/extensions/skills。"
        )));
    }

    // 回读而不是把拼出来的对象返回：界面显示的必须与库里一致。
    let saved = skills_repo::get(db, user.0.user_id, &norm)
        .await
        .map_err(|e| map_err("回读 SKILL", e))?
        .ok_or_else(|| {
            ApiError::internal(format!("刚改完的 SKILL {norm} 立刻就读不到了"))
        })?;

    Ok(Json(json!({ "skill": skills_repo::to_json(&saved) })).into_response())
}
