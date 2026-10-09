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

pub(crate) fn map_err(op: &str, e: quill_agent::AgentError) -> ApiError {
    ApiError::internal(format!("{op}失败（详情见服务端日志）：{e}"))
}

/// `GET /api/extensions/mcp` —— 列出当前用户的 MCP 服务器配置，**并真的连一次**。
///
/// 每台服务器都会走一遍 `mcp_client::probe`：启用且是 stdio 的会真的被拉起来、
/// 真的握手、真的 `tools/list`。连不上的那台在 `status` 里带一句白话原因，
/// 但**不会**让整个请求失败 —— 一台用户的服务器起不来，不该让整页 MCP 打不开。
pub async fn list_mcp(State(state): State<AppState>, user: AuthUser) -> Result<Response, ApiError> {
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
    for row in rows
        .iter()
        .filter(|r| !active_names.iter().any(|n| n == &r.name))
    {
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
        // 内置服务器有哪些名字，**只有服务端知道**（`quill_core::builtin`）。
        // 前端要在表单里给出可选项，就从这里拿 —— 自己硬编码一份必然会漂。
        "builtin_servers": quill_core::builtin::BUILTIN_SERVERS,
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
    // 没有库时不必建基线：与从前一样直接回空（端口内部的读取也会探到同一件事，
    // 这里早退只是省一次必然失败的往返，也不把「没库」报成「建基线失败」）。
    if state.db().is_err() {
        return Vec::new();
    }
    let dir = skill_dir(&state.config);
    // 基线 = 内置 + SKILL。与 `with_mcp_tools` 的前半段完全一致，所以
    // 「已被占住的名字」在两处是同一份 —— 否则界面上会说一个工具挂上了，
    // 而对话里它被内置工具顶掉了。素材走与对话同一条端口实现。
    let mut taken = match crate::tools::baseline_specs(
        crate::tool_sources::DbToolSources::shared(state, dir),
        uid,
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
                parsed
                    .iter()
                    .position(|x| x.name.to_lowercase() == n)
                    .map(|x| x + 1)
                    .unwrap_or(0)
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

// `builtin` 从第一天就在 `mcp_servers.transport` 的 CHECK 里（`0001_init.sql`），
// 但 API 一直不收 —— 声明了做不到。queue Q111 把它接上：`command` 写内置服务器的
// 名字（目前只有 memory），服务端在内存管道上跑它，不起子进程、不用装依赖。
const ALLOWED_TRANSPORTS: [&str; 4] = ["stdio", "builtin", "streamable_http", "sse"];

/// `PATCH /api/extensions/mcp/{name}` —— 单条局部更新（含改名）。
///
/// ## 为什么要有这一条
///
/// 前端目前是全量 `POST`（提交整个 `servers` 数组），要改一台服务器就得把**所有**
/// 服务器的完整配置回填再重传一遍；改名更是只能「删一条 + 加一条」，而删除是
/// 软删、名字又占着主键，删完再建同一个名字是否成立全看历史。这一条把
/// 「改一台」收成一个只关于那一台的请求。
///
/// ## 语义（缺省 = 不变）
///
/// **请求体里没有的键 = 这个字段保持原值**，不是清空。清空要显式给 `null`
/// （对 `enabled_capabilities` 来说 `null` 是**有意义的取值**：全禁 ——
/// 想「不变」就不发这个键，想全禁就发 `null`）。
///
/// 路径里的 `{name}` 是**现在的名字**；请求体里的 `name`（如果发）是
/// **想改成的新名字**，与前端 `McpServerConfig.name` 同义 —— 于是
/// 「GET 一台 → 改几个字段 → PATCH 回来」这条往返是通的。
///
/// ## 只改自己名下的行
///
/// `mcp_repo::get` / `apply_patch` 的 WHERE 都带 `user_id`（见 `mcp_repo::GET_SQL`、
/// `PATCH_SQL`），别人的行在这里是 404，不是能改的对象。
pub async fn patch_mcp(
    State(state): State<AppState>,
    user: AuthUser,
    Path(name): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    let norm = mcp_repo::normalize_name(&name).map_err(ApiError::bad_request)?;
    let patch = parse_patch(&body)?;

    let db = state.db()?;
    // 先读一次：合并字段要有基准。冲突判定不靠这次读，见 `mcp_repo::apply_patch`
    // 的说明（它在事务里贴着写再读一次）。
    let current = mcp_repo::get(db, user.0.user_id, &norm)
        .await
        .map_err(|e| map_err("读取 MCP 服务器", e))?
        .ok_or_else(|| mcp_not_found(&norm))?;
    let next = patch.apply_to(&current)?;
    let fields = changed_fields(&current, &next);
    // `apply_patch` 会吃掉 `next`；冲突文案里要用新名字，先留一份。
    let next_name = next.name.clone();

    let outcome = mcp_repo::apply_patch(db, user.0.user_id, &norm, next)
        .await
        .map_err(|e| map_err("更新 MCP 服务器", e))?;
    let (row, changed) = match outcome {
        mcp_repo::PatchOutcome::NotFound => return Err(mcp_not_found(&norm)),
        mcp_repo::PatchOutcome::NameTaken => {
            let detail = format!(
                "改成的名字「{next_name}」已经被你名下的另一台 MCP 服务器占着，这次没有任何改动。\
                 名字是主键的一部分（`(user_id, name)`），库里的两行不能同名。"
            );
            return Err(ApiError::conflict(detail, RENAME_TAKEN_ADVICE));
        }
        mcp_repo::PatchOutcome::Unchanged(row) => (row, false),
        mcp_repo::PatchOutcome::Updated(row) => (row, true),
    };

    // 不变量：**写没写库**与**字段到底变没变**必须是同一件事。
    // 两边说法不一致的话，要么改了却报「没改」（用户以为没生效，实际生效了），
    // 要么报「改了」而库里一字未动 —— 两种都是本项目的红线。
    if changed != !fields.is_empty() {
        return Err(ApiError::internal(format!(
            "PATCH 的变更判定自相矛盾：字段差异 {fields:?}，落库结果说{}。",
            if changed { "写了" } else { "没写" }
        )));
    }

    let renamed_from = fields.contains(&"name").then(|| current.name.clone());
    let note = if !changed {
        format!(
            "请求里的值与库里「{norm}」的当前值逐字相同：这次没有写库，`updated_at` 也没动 —— \
             与全量提交那条「内容没变就别动时间戳」的语义一致。"
        )
    } else {
        let mut n = format!("已写入并回读：{} 变了。", fields.join("、"));
        if let Some(from) = &renamed_from {
            n.push_str(&format!(
                "改名是同一行的键换了（保留 created_at），不是删一条加一条；旧名字「{from}」现在查不到了。"
            ));
        }
        n.push_str(
            "连通性不在这一步测 —— 要测请 GET /api/extensions/mcp，它会对启用的服务器真的握手。",
        );
        n
    };

    let rows = mcp_repo::list(db, user.0.user_id)
        .await
        .map_err(|e| map_err("更新后回读列表", e))?;
    Ok(Json(json!({
        "server": mcp_repo::to_json(&row),
        "changed": changed,
        "changed_fields": fields,
        "renamed_from": renamed_from,
        "servers": rows.iter().map(mcp_repo::to_json).collect::<Vec<Value>>(),
        "note": note,
    }))
    .into_response())
}

/// 改名撞名时的「下一步」。
///
/// 不能写「先删掉占名字的那台再改名」—— 删除是**软删**，行还在、名字还占着
/// 主键（`mcp_servers` 的 PRIMARY KEY 是 `(user_id, name)`，见 migrations/0007），
/// 删完照样改名失败。那句话是照着做仍然错的建议。
const RENAME_TAKEN_ADVICE: &str = "下一步：换一个没被用过的名字。\
     注意已删除（软删）的服务器仍然占着它原来的名字 —— 名字是主键的一部分，历史保留；\
     要复用某个旧名字，请走 POST /api/extensions/mcp 全量提交（它会把同名行整行复活）。";

fn mcp_not_found(norm: &str) -> ApiError {
    // 用 `entity_not_found` 而不是 `not_found`：后者的文案是「本实例没有路由」，
    // 拿它说「这台服务器不存在」会拼出一句病句。
    ApiError::entity_not_found(format!(
        "名为 {norm} 的 MCP 服务器不存在或已删除，所以这次没有任何改动。\
         下一步：GET /api/extensions/mcp 确认名字；要新建请 POST /api/extensions/mcp（全量提交）。"
    ))
}

/// `PATCH` 的局部改动。每个字段的 `None` 都是「请求里没提这个键」→ 不变；
/// 提了就按请求里的值算（`null` 也是取值：清空，或能力三态里的「全禁」）。
#[derive(Debug, Default)]
struct McpPatch {
    name: Option<String>,
    transport: Option<String>,
    command: Option<Option<String>>,
    args: Option<Vec<String>>,
    cwd: Option<Option<String>>,
    env: Option<Vec<(String, String)>>,
    url: Option<Option<String>>,
    headers: Option<Vec<(String, String)>>,
    enabled: Option<bool>,
    timeout_ms: Option<i64>,
    description: Option<String>,
    max_concurrent_calls: Option<Option<i64>>,
    enabled_capabilities: Option<Option<Vec<String>>>,
}

fn parse_patch(v: &Value) -> Result<McpPatch, ApiError> {
    // 白名单与 POST 同一份 `SERVER_FIELDS`：两条路能改的字段集合必须一致，
    // 否则会出现「POST 存不进去的键，PATCH 悄悄写进去」。
    crate::api_experts::only_keys_at(v, SERVER_FIELDS, "MCP 服务器的 PATCH")?;
    let obj = v.as_object().expect("only_keys_at 已经确认是对象");

    let mut p = McpPatch::default();
    if let Some(val) = obj.get("name") {
        p.name = Some(patch_str(val, "name")?);
    }
    if let Some(val) = obj.get("transport") {
        let t = patch_str(val, "transport")?;
        if !ALLOWED_TRANSPORTS.contains(&t.as_str()) {
            return Err(ApiError::bad_request(format!(
                "PATCH 里的 transport（{t}）不是支持的传输方式。可用：{}。\
                 下一步：下拉框里选一个，或整个键不发（不发 = 不改传输方式）。",
                ALLOWED_TRANSPORTS.join(" / ")
            )));
        }
        p.transport = Some(t);
    }
    if let Some(val) = obj.get("command") {
        p.command = Some(patch_opt_str(val, "command")?);
    }
    if let Some(val) = obj.get("args") {
        p.args = Some(patch_args(val)?);
    }
    if let Some(val) = obj.get("cwd") {
        p.cwd = Some(patch_opt_str(val, "cwd")?);
    }
    if let Some(val) = obj.get("env") {
        p.env = Some(kv(Some(val), "PATCH 里的 env（stdio 专用）")?);
    }
    if let Some(val) = obj.get("url") {
        p.url = Some(patch_opt_str(val, "url")?);
    }
    if let Some(val) = obj.get("headers") {
        p.headers = Some(kv(Some(val), "PATCH 里的 headers（http/sse 专用）")?);
    }
    if let Some(val) = obj.get("enabled") {
        p.enabled = Some(val.as_bool().ok_or_else(|| {
            ApiError::bad_request(
                "PATCH 里的 enabled 必须是 true 或 false。\
                 下一步：{\"enabled\": false} 会停用它（不握手、不挂工具），\
                 或整个键不发（不发 = 不改）。"
                    .to_string(),
            )
        })?);
    }
    if let Some(val) = obj.get("timeout_ms") {
        p.timeout_ms = Some(patch_int_in_range(val, "timeout_ms", 1000, 600_000)?);
    }
    if let Some(val) = obj.get("description") {
        p.description = Some(patch_str(val, "description")?);
    }
    if let Some(val) = obj.get("max_concurrent_calls") {
        p.max_concurrent_calls = Some(patch_opt_int_in_range(val, "max_concurrent_calls", 1, 64)?);
    }
    if let Some(val) = obj.get("enabled_capabilities") {
        p.enabled_capabilities = Some(patch_caps(val)?);
    }
    Ok(p)
}

impl McpPatch {
    /// 把局部改动合并到当前行上，并做与 POST 同一套的交叉校验。
    fn apply_to(&self, current: &McpServerRow) -> Result<McpServerRow, ApiError> {
        let name = match &self.name {
            Some(n) => mcp_repo::normalize_name(n).map_err(ApiError::bad_request)?,
            None => current.name.clone(),
        };
        let transport = self
            .transport
            .clone()
            .unwrap_or_else(|| current.transport.clone());
        let command = match &self.command {
            Some(c) => c.clone(),
            None => current.command.clone(),
        };
        let args = self.args.clone().unwrap_or_else(|| current.args.clone());
        let cwd = match &self.cwd {
            Some(c) => c.clone(),
            None => current.cwd.clone(),
        };
        let env = self.env.clone().unwrap_or_else(|| current.env.clone());
        let url = match &self.url {
            Some(u) => u.clone(),
            None => current.url.clone(),
        };
        let headers = self
            .headers
            .clone()
            .unwrap_or_else(|| current.headers.clone());
        let enabled = self.enabled.unwrap_or(current.enabled);
        let timeout_ms = self.timeout_ms.unwrap_or(current.timeout_ms);
        let description = self
            .description
            .clone()
            .unwrap_or_else(|| current.description.clone());
        let max_concurrent_calls = match self.max_concurrent_calls {
            Some(v) => v,
            None => current.max_concurrent_calls,
        };
        let enabled_capabilities = match &self.enabled_capabilities {
            Some(v) => v.clone(),
            None => current.enabled_capabilities.clone(),
        };

        let next = McpServerRow {
            name,
            transport,
            command,
            args,
            env,
            url,
            headers,
            enabled,
            timeout_ms,
            description,
            cwd,
            max_concurrent_calls,
            enabled_capabilities,
            // 时间戳不是配置：合并时保持原值，写库时由 `mcp_repo::apply_patch`
            // 决定要不要动 `updated_at`。
            created_at: current.created_at,
            updated_at: current.updated_at,
        };
        // 与 POST 共用同一份交叉校验：PATCH 能把 transport 换掉，
        // 换完必须仍是「stdio 有 command 无 url / http 有 url 无 command」。
        check_transport_shape(
            "PATCH 后的",
            &next.transport,
            next.command.as_deref(),
            next.url.as_deref(),
        )?;
        Ok(next)
    }
}

/// 两个整行之间「能被 PATCH 改动的字段」的差异清单（固定顺序，便于测试与阅读）。
///
/// 与 `mcp_repo::asset_hash` 覆盖的字段**必须一一对应**：指纹漏了哪个字段，
/// 那个字段就会被 `apply_patch` 判成「没改」而不落库。有一条测试逐字段钉这件事。
fn changed_fields(a: &McpServerRow, b: &McpServerRow) -> Vec<&'static str> {
    let mut out = Vec::new();
    if a.name != b.name {
        out.push("name");
    }
    if a.transport != b.transport {
        out.push("transport");
    }
    if a.command != b.command {
        out.push("command");
    }
    if a.args != b.args {
        out.push("args");
    }
    if a.cwd != b.cwd {
        out.push("cwd");
    }
    if a.env != b.env {
        out.push("env");
    }
    if a.url != b.url {
        out.push("url");
    }
    if a.headers != b.headers {
        out.push("headers");
    }
    if a.enabled != b.enabled {
        out.push("enabled");
    }
    if a.timeout_ms != b.timeout_ms {
        out.push("timeout_ms");
    }
    if a.description != b.description {
        out.push("description");
    }
    if a.max_concurrent_calls != b.max_concurrent_calls {
        out.push("max_concurrent_calls");
    }
    if a.enabled_capabilities != b.enabled_capabilities {
        out.push("enabled_capabilities");
    }
    out
}

fn patch_str(val: &Value, key: &str) -> Result<String, ApiError> {
    val.as_str().map(str::to_string).ok_or_else(|| {
        ApiError::bad_request(format!(
            "PATCH 里的 {key} 必须是字符串。\
             下一步：给它一个字符串值，或整个键不发（不发 = 不改这个字段）。"
        ))
    })
}

fn patch_opt_str(val: &Value, key: &str) -> Result<Option<String>, ApiError> {
    if val.is_null() {
        return Ok(None);
    }
    let s = patch_str(val, key)?;
    let t = s.trim();
    Ok(if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    })
}

fn patch_int_in_range(val: &Value, key: &str, lo: i64, hi: i64) -> Result<i64, ApiError> {
    let n = val.as_i64().ok_or_else(|| {
        ApiError::bad_request(format!(
            "PATCH 里的 {key} 必须是整数。\
             下一步：填 {lo}~{hi} 之间的数字，或整个键不发（不发 = 不改）。"
        ))
    })?;
    if !(lo..=hi).contains(&n) {
        return Err(ApiError::bad_request(format!(
            "PATCH 里的 {key}={n} 超出范围（{lo}..{hi}）。\
             下一步：改成范围内的数字，或整个键不发（不发 = 不改）。"
        )));
    }
    Ok(n)
}

fn patch_opt_int_in_range(
    val: &Value,
    key: &str,
    lo: i64,
    hi: i64,
) -> Result<Option<i64>, ApiError> {
    if val.is_null() {
        return Ok(None);
    }
    patch_int_in_range(val, key, lo, hi).map(Some)
}

fn patch_args(val: &Value) -> Result<Vec<String>, ApiError> {
    if val.is_null() {
        return Ok(Vec::new());
    }
    let a = val.as_array().ok_or_else(|| {
        ApiError::bad_request(
            "PATCH 里的 args 必须是字符串数组（`null` 表示清空）。\
             下一步：写成 [\"-y\", \"some-server\"]，或整个键不发（不发 = 不改）。"
                .to_string(),
        )
    })?;
    let mut out = Vec::with_capacity(a.len());
    for x in a {
        let s = x.as_str().ok_or_else(|| {
            ApiError::bad_request(
                "PATCH 里的 args 的元素必须是字符串。\
                 下一步：检查有没有写成数字或对象的元素，或整个键不发（不发 = 不改）。"
                    .to_string(),
            )
        })?;
        out.push(s.to_string());
    }
    Ok(out)
}

fn patch_caps(val: &Value) -> Result<Option<Vec<String>>, ApiError> {
    match val {
        // `null` = 全禁，是**取值**不是「不变」：想不变就别发这个键。
        Value::Null => Ok(None),
        Value::Array(a) => {
            let mut out: Vec<String> = Vec::new();
            for x in a {
                let s = x.as_str().ok_or_else(|| {
                    ApiError::bad_request(
                        "PATCH 里的 enabled_capabilities 的元素必须是字符串。\
                         下一步：写成 [\"read\", \"write\"] 这样的一串字符串。"
                            .to_string(),
                    )
                })?;
                if !out.iter().any(|y| y == s) {
                    out.push(s.to_string());
                }
            }
            Ok(Some(out))
        }
        _ => Err(ApiError::bad_request(
            "PATCH 里的 enabled_capabilities 必须是数组（[]=全部启用）或 null（全部禁用）。\
             下一步：想全开传 []，想全禁传 null，想不改就别发这个键。"
                .to_string(),
        )),
    }
}

/// 传输方式与必填字段的交叉校验。**POST 与 PATCH 共用这一份** ——
/// 两边各写一遍迟早会漂，而漂的后果是「POST 拦得住的错配置，PATCH 放得进去」。
///
/// `label` 自带尾部的「的」：POST 传「第 0 项的」，PATCH 传「PATCH 后的」。
fn check_transport_shape(
    label: &str,
    transport: &str,
    command: Option<&str>,
    url: Option<&str>,
) -> Result<(), ApiError> {
    let bad = ApiError::bad_request;
    if transport == "builtin" {
        let name = command.unwrap_or("").trim();
        if name.is_empty() {
            return Err(bad(format!(
                "{label} command 必填：builtin 传输要在 command 里写内置服务器的**名字**\
                 （目前内置的有：{}）。\
                 下一步：把 command 填成 memory。",
                quill_core::builtin::names()
            )));
        }
        if !quill_core::builtin::is_builtin(name) {
            return Err(bad(format!(
                "{label} 内置服务器 {name:?} 不认识（目前内置的有：{}）。\
                 下一步：改成其中之一，或把传输方式改成 stdio 自己起一个进程。",
                quill_core::builtin::names()
            )));
        }
        if url.is_some() {
            return Err(bad(format!(
                "{label} url 不该出现在 builtin 配置上：内置服务器就在本进程里跑，没有地址。\
                 下一步：清空 url（PATCH 传 null），或把传输方式改成 streamable_http / sse。"
            )));
        }
        return Ok(());
    }
    if transport == "stdio" {
        if command.unwrap_or("").trim().is_empty() {
            return Err(bad(format!(
                "{label} command 必填：stdio 传输要启动一个本地进程。\
                 下一步：填上可执行文件名，例如 `npx` 或 `uvx`。"
            )));
        }
        if url.is_some() {
            return Err(bad(format!(
                "{label} url 不该出现在 stdio 配置上：stdio 连的是本地进程，没有地址。\
                 下一步：清空 url（PATCH 传 null），或把传输方式改成 streamable_http / sse。"
            )));
        }
    } else {
        if url.unwrap_or("").is_empty() {
            return Err(bad(format!(
                "{label} url 必填：{transport} 要连一个 HTTP 地址。\
                 下一步：填上完整地址，例如 https://example.com/mcp"
            )));
        }
        if command.is_some() {
            return Err(bad(format!(
                "{label} command 不该出现在 {transport} 配置上：它连的是远端，不在本机起进程。\
                 下一步：清空 command（PATCH 传 null），并把命令要带的参数挪到 headers 之外的正确位置。"
            )));
        }
    }
    Ok(())
}

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

    let command = obj
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string);
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
    let headers = kv(
        obj.get("headers"),
        &format!("第 {idx} 项的 headers（http/sse 专用）"),
    )?;

    // 传输方式与必填字段对不上时**在这里**报错，而不是丢给数据库 CHECK ——
    // 数据库只说 "CHECK constraint failed"，用户无从知道是哪台服务器的哪个字段。
    check_transport_shape(
        &format!("第 {idx} 项的"),
        &transport,
        command.as_deref(),
        url.as_deref(),
    )?;

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
            val.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| val.to_string()),
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
        let e = hub_error(
            "读技能市场列表失败",
            HubError::Parse("期望 skillSets".into()),
        );
        let d = e.detail();
        assert!(d.contains("读技能市场列表失败"), "{d}");
        assert!(d.contains("skillSets"), "真正的原因要留着：{d}");
    }

    /// 超限不许拿「连不上」那句「下一步」。
    ///
    /// 这一层原先把 `HubError::TooLarge` 一起塞进 `other`，
    /// 于是 detail 说对了、`next_step` 却在教用户去查网络 ——
    /// 而网络上没有任何毛病。下面把两档都钉住，防止**只**改一边。
    ///
    /// 状态码两档都是 503，理由写在这里免得下次有人顺手改成 413：
    /// 413 的定义是**请求体**太大（RFC 9110 §15.5.14），我们的请求只有几百字节，
    /// 发 413 等于告诉调用方「你的请求被拒了」，那是一句新的假话。
    /// 502 更贴切，但 `ApiError` 里上游类错误统一是 503，
    /// 且 `error.rs` 明写「状态码另议」；把新状态码留给一次专门的对外改动。
    #[test]
    fn an_oversized_response_gets_its_own_advice_and_never_the_network_one() {
        let oversize = hub_error(
            "拉技能市场榜单失败",
            HubError::TooLarge {
                label: "/api/v1/showcase/recommended".to_string(),
                limit: crate::skillhub::MAX_HTTP_BYTES,
            },
        );
        assert_eq!(
            oversize.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "413 是「请求体太大」，这里超的是上游的响应，不是我们的请求"
        );
        assert_eq!(oversize.next_step(), super::SKILLHUB_OVERSIZE_ADVICE);
        for wrong in ["先确认网络能到上游", "检查网络", "可达的"] {
            assert!(
                !oversize.next_step().contains(wrong),
                "超限不是连不上，「下一步」里不该出现「{wrong}」：{}",
                oversize.next_step()
            );
        }
        // detail 要留住上限与是哪个接口，用户才知道该拿什么去比。
        let d = oversize.detail();
        assert!(d.contains("32"), "上限要给出数字：{d}");
        assert!(
            d.contains("/api/v1/showcase/recommended"),
            "接口要说清：{d}"
        );

        // 反过来也得钉住：真正的传输失败仍然要给网络建议，
        // 别把这个 bug 修成「谁都不再说网络」。
        let transport = hub_error("拉技能市场榜单失败", HubError::Fetch("refused".into()));
        assert!(transport.next_step().contains("先确认网络能到上游"));
        assert_ne!(
            transport.next_step(),
            oversize.next_step(),
            "两档的「下一步」必须是两句不同的话"
        );
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
        let manifest = crate::skillhub::parse_manifest(REAL_MANIFEST).expect("manifest 应当能解析");
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

/// 钉住 [`patch_mcp`] 的合并语义与 [`changed_fields`] 的覆盖度。
///
/// 这一组是**纯函数**测试：真库往返在 `tests/extensions_http.rs`（`patch_mcp` 本体
/// 直接调用；`routes.rs` 那条路由的接线不归本文件管）。
#[cfg(test)]
mod mcp_patch_tests {
    use super::*;

    fn base() -> McpServerRow {
        McpServerRow {
            name: "a".into(),
            transport: "stdio".into(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "some-server".into()],
            env: vec![("TOKEN".into(), "secret".into())],
            url: None,
            headers: vec![],
            enabled: true,
            timeout_ms: 30_000,
            description: "说明".into(),
            cwd: Some("/tmp/work".into()),
            max_concurrent_calls: Some(4),
            enabled_capabilities: Some(vec!["read".into()]),
            created_at: 111,
            updated_at: 222,
        }
    }

    fn mutate(base: &McpServerRow, f: impl FnOnce(&mut McpServerRow)) -> McpServerRow {
        let mut r = base.clone();
        f(&mut r);
        r
    }

    /// **每一个 PATCH 能改的字段，落库层的变更判定都必须看得见。**
    ///
    /// `mcp_repo::apply_patch` 用「名字变了或指纹变了」决定写不写。指纹
    /// （`asset_hash`）漏掉哪个字段，那个字段就会被判成「没改」而**静默不落库** ——
    /// 请求 200，库里没变。这条测试逐个字段钉住这件事，是这一组的支点。
    #[test]
    fn every_patchable_field_is_visible_to_the_repos_change_detection() {
        let b = base();
        let changes: Vec<(&str, McpServerRow)> = vec![
            ("name", mutate(&b, |r| r.name = "b".into())),
            (
                "transport",
                mutate(&b, |r| {
                    r.transport = "streamable_http".into();
                    r.command = None;
                    r.url = Some("https://example/mcp".into());
                }),
            ),
            ("command", mutate(&b, |r| r.command = Some("uvx".into()))),
            ("args", mutate(&b, |r| r.args = vec![])),
            ("cwd", mutate(&b, |r| r.cwd = None)),
            (
                "env",
                mutate(&b, |r| r.env = vec![("TOKEN".into(), "other".into())]),
            ),
            (
                "url",
                mutate(&b, |r| {
                    r.transport = "streamable_http".into();
                    r.command = None;
                    r.url = Some("https://example/mcp".into());
                }),
            ),
            (
                "headers",
                mutate(&b, |r| r.headers = vec![("TOKEN".into(), "1".into())]),
            ),
            ("enabled", mutate(&b, |r| r.enabled = false)),
            ("timeout_ms", mutate(&b, |r| r.timeout_ms = 45_000)),
            ("description", mutate(&b, |r| r.description = "别的".into())),
            (
                "max_concurrent_calls",
                mutate(&b, |r| r.max_concurrent_calls = None),
            ),
            (
                "enabled_capabilities",
                mutate(&b, |r| r.enabled_capabilities = None),
            ),
        ];

        for (label, changed) in &changes {
            let fields = changed_fields(&b, changed);
            assert!(
                fields.contains(label),
                "{label} 改了却不在差异清单里：{fields:?}"
            );
            assert!(
                b.name != changed.name || mcp_repo::asset_hash(&b) != mcp_repo::asset_hash(changed),
                "{label} 改了，但指纹与名字都看不出来 —— apply_patch 会当成没改而不落库"
            );
        }
        // 反方向：一字不动时，差异清单是空的、指纹也一样。
        assert!(changed_fields(&b, &base()).is_empty(), "没改就什么都不该报");
        assert_eq!(mcp_repo::asset_hash(&b), mcp_repo::asset_hash(&base()));
    }

    #[test]
    fn a_patch_only_touches_the_keys_it_carries() {
        let current = base();
        let next = parse_patch(&serde_json::json!({"timeout_ms": 45_000}))
            .expect("合法请求体")
            .apply_to(&current)
            .expect("合并");

        assert_eq!(next.timeout_ms, 45_000);
        // 缺省 = 不变：其余字段逐字保持。
        assert_eq!(next.name, current.name);
        assert_eq!(next.transport, current.transport);
        assert_eq!(next.command, current.command);
        assert_eq!(next.args, current.args);
        assert_eq!(next.cwd, current.cwd);
        assert_eq!(next.env, current.env, "env 不许被顺手清掉");
        assert_eq!(next.headers, current.headers);
        assert_eq!(next.enabled, current.enabled);
        assert_eq!(next.description, current.description);
        assert_eq!(next.max_concurrent_calls, current.max_concurrent_calls);
        assert_eq!(next.enabled_capabilities, current.enabled_capabilities);
        assert_eq!(next.created_at, current.created_at);
        assert_eq!(next.updated_at, current.updated_at);
    }

    #[test]
    fn an_explicit_null_clears_while_an_absent_key_keeps() {
        let current = base();
        let next = parse_patch(&serde_json::json!({
            "cwd": null,
            "env": null,
            "max_concurrent_calls": null
        }))
        .expect("合法请求体")
        .apply_to(&current)
        .expect("合并");

        assert_eq!(next.cwd, None, "显式 null = 清空");
        assert!(next.env.is_empty(), "显式 null = 清空");
        assert_eq!(next.max_concurrent_calls, None);
        assert_eq!(next.description, current.description, "没提的键不动");
        assert_eq!(next.args, current.args, "没提的键不动");
    }

    /// `enabled_capabilities` 的三态里，`null` 是**取值**（全禁），不是「不变」。
    /// 混了就是把用户明确的全禁改成了「保持原样」。
    #[test]
    fn a_null_capability_list_means_all_off_not_unchanged() {
        let all_on = mutate(&base(), |r| r.enabled_capabilities = Some(vec![]));
        let next = parse_patch(&serde_json::json!({"enabled_capabilities": null}))
            .expect("合法请求体")
            .apply_to(&all_on)
            .expect("合并");
        assert_eq!(next.enabled_capabilities, None, "null 必须落成「全禁」");

        let keep = parse_patch(&serde_json::json!({}))
            .expect("合法请求体")
            .apply_to(&all_on)
            .expect("合并");
        assert_eq!(
            keep.enabled_capabilities,
            Some(vec![]),
            "不发这个键才是「不变」"
        );
    }

    #[test]
    fn the_merge_runs_the_same_transport_checks_as_post() {
        let current = base();
        // stdio → streamable_http：只发 transport 不够 —— 合并后仍是「有 command、没 url」。
        let err = parse_patch(&serde_json::json!({"transport": "streamable_http"}))
            .expect("合法请求体")
            .apply_to(&current)
            .expect_err("必须拦下");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(err.detail().contains("url"), "{}", err.detail());
        assert!(err.detail().contains("下一步"), "{}", err.detail());

        // 一次把 url 补上、command 清掉就成立。
        let next = parse_patch(&serde_json::json!({
            "transport": "streamable_http",
            "url": "http://127.0.0.1:1/mcp",
            "command": null
        }))
        .expect("合法请求体")
        .apply_to(&current)
        .expect("合并");
        assert_eq!(next.transport, "streamable_http");
        assert_eq!(next.url.as_deref(), Some("http://127.0.0.1:1/mcp"));
        assert_eq!(next.command, None);
        assert!(
            next.env == current.env,
            "换传输方式不清 env（与 POST 的口径一致：怎么存就怎么读）"
        );
    }

    #[test]
    fn a_patch_that_does_not_parse_refuses_the_whole_request() {
        for bad in [
            serde_json::json!({"autorize": "Bearer x"}),
            serde_json::json!({"timeout_ms": 999}),
            serde_json::json!({"timeout_ms": "30000"}),
            serde_json::json!({"enabled": "yes"}),
            serde_json::json!({"args": "-y"}),
            serde_json::json!({"enabled_capabilities": "all"}),
            serde_json::json!([]),
        ] {
            let err = parse_patch(&bad).expect_err("必须被拒");
            assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST, "{bad}");
            assert!(err.detail().contains("下一步"), "{bad}：{}", err.detail());
        }

        // 名字非法在合并那一步才判（它要过归一）。
        let err = parse_patch(&serde_json::json!({"name": "!!!"}))
            .expect("解析得通")
            .apply_to(&base())
            .expect_err("非法名字必须被拒");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(err.detail().contains("下一步"), "{}", err.detail());
    }
}

/// 钉住 [`plan_install`]：一个技能名**只能有一个正文**，
/// 而 `installed_count` 必须等于装完真的存在的技能数。
///
/// 这条曾经是坏的，而且坏得很难看：有 manifest 时包里每个 `.md` 都算成同一个
/// slug，第二个条目的正文盖掉第一个，库里只剩一行，循环却照样数了两下 ——
/// 于是「装到 2 个技能」，实际只有 1 个，而且正文是谁全看 zip 里的顺序。
#[cfg(test)]
mod install_plan_tests {
    use super::{plan_install, MANIFEST_FILE};
    use crate::skillhub::parse_manifest;

    const MANIFEST: &str = r#"{"slug":"tech-test-automation","displayName":"自动化测试"}"#;

    /// 造一份解包结果。`plan_install` 只看条目名，不看正文，
    /// 所以正文在这里统一填个能认出来的东西就够了。
    fn entries(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|n| (n.to_string(), format!("正文 of {n}")))
            .collect()
    }

    fn slugs(plan: &super::InstallPlan) -> Vec<String> {
        plan.picks.iter().map(|(_, s)| s.clone()).collect()
    }

    #[test]
    fn two_bodies_in_one_package_produce_one_skill_and_the_loser_is_named() {
        // 上游的正常形状：manifest + `SKILL.md` + `README.md`。
        // 三个条目（manifest 不算）里只有**一个**技能名能存在。
        let files = entries(&[MANIFEST_FILE, "SKILL.md", "README.md"]);
        let manifest = parse_manifest(MANIFEST).expect("manifest 应当能解析");
        let plan = plan_install(Some(&manifest), &files).expect("应当能算出计划");

        assert_eq!(
            slugs(&plan),
            vec!["tech-test-automation".to_string()],
            "一个包 = 一个技能名，正文只留第一篇"
        );
        assert_eq!(plan.skipped.len(), 1, "被盖掉的那一篇要点名");
        let (loser, keeper, skill) = &plan.skipped[0];
        assert_eq!(loser, "README.md");
        assert_eq!(keeper, "SKILL.md", "要说清留下的是哪一篇");
        assert_eq!(skill, "tech-test-automation");
    }

    #[test]
    fn the_single_body_package_is_untouched_by_the_new_check() {
        // `tech-test-automation` 的真实形状：manifest + 一篇正文。
        // 这里不能多挡任何东西 —— 挡了就是让一个本来能装的包装不上。
        let files = entries(&[MANIFEST_FILE, "identify.md"]);
        let manifest = parse_manifest(MANIFEST).expect("manifest 应当能解析");
        let plan = plan_install(Some(&manifest), &files).expect("应当能算出计划");

        assert_eq!(slugs(&plan), vec!["tech-test-automation".to_string()]);
        assert!(plan.skipped.is_empty(), "只有一个正文，不该报撞名");
    }

    #[test]
    fn two_directories_with_the_same_basename_collide_without_a_manifest() {
        // 没有 manifest 时用包内文件名命名，而 `skillhub_unpack` 把条目名
        // 拍成 basename 且不去重：`a/x.md` 与 `b/x.md` 都会变成 `x`。
        let files = entries(&["x.md", "y.md", "x.md"]);
        let plan = plan_install(None, &files).expect("应当能算出计划");

        assert_eq!(slugs(&plan), vec!["x".to_string(), "y".to_string()]);
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(plan.skipped[0].2, "x");
    }

    #[test]
    fn a_package_of_distinct_bodies_without_a_manifest_installs_each_one() {
        let files = entries(&["alpha.md", "beta.md"]);
        let plan = plan_install(None, &files).expect("应当能算出计划");

        assert_eq!(slugs(&plan), vec!["alpha".to_string(), "beta".to_string()]);
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn a_package_of_only_metadata_installs_nothing_and_skips_nothing() {
        // 这时由 handler 那条「一个都没装上就报 400」接手。
        // `plan_install` 不该在这里编一个技能出来。
        let files = entries(&[MANIFEST_FILE]);
        let manifest = parse_manifest(MANIFEST).expect("manifest 应当能解析");
        let plan = plan_install(Some(&manifest), &files).expect("应当能算出计划");

        assert!(plan.picks.is_empty());
        assert!(plan.skipped.is_empty());
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
///
/// ## 为什么要 `AuthUser`（哪怕一个字段都不用）
///
/// 这个端点只是读，但 `kind=all` 一次要并发拉 6 个上游榜单
/// （`skillhub::showcase_all`），而本项目的认证是**逐 handler** 的 ——
/// `GuardLayer` 只兜 panic，不做鉴权。不挂 `AuthUser`，它就是一个
/// **未登录就能打的对外放大器**：拿别人的服务器替自己去敲上游，还不限速。
/// 这里连一个字段都不需要，只要求「调用者有会话」。
pub async fn skill_hub_list(
    _user: AuthUser,
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
}
fn one() -> u32 {
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
pub(crate) fn hub_error(what: &str, e: crate::skillhub::HubError) -> ApiError {
    use crate::skillhub::HubError;
    match e {
        // 调用方给错了值 → 400。**说「上游坏了」会把用户引去查一个没坏的东西。**
        HubError::Input(detail) => ApiError::bad_request(format!("{what}：{detail}")),
        // 上游说「没有这个东西」→ 404，不是 503。
        // 它与「连不上」在界面上是两件事，见 ISSUE-055。
        HubError::Status(404) => ApiError::entity_not_found(format!(
            "{what}：技能市场里没有这个东西（上游返回 404），它可能已被下架。"
        )),
        // 剩下的都归「上游没给出可用结果」，但**超限必须自己一句下一步**。
        //
        // 之前超限掉进下面这个 `other`，于是响应里的 `next_step` 说的是
        // 「先确认网络能到上游 / 换可达的镜像」—— 那一档的前提是**市场连不上**，
        // 而这一次市场好好地回了话，只是回得超过 `MAX_HTTP_BYTES`。
        // 把没坏的东西报成坏了，用户会去查一件从头到尾都正常的东西。
        //
        // 判定用 `is_oversize()`（枚举变体本身），**不拿中文认字**：
        // 认字的判据会跟着文案改而悄悄失效，那正是这条防线最初塌掉的方式。
        other => {
            let advice = if other.is_oversize() {
                SKILLHUB_OVERSIZE_ADVICE
            } else {
                SKILLHUB_ADVICE
            };
            ApiError::upstream_unavailable(format!("{what}：{}", other.message()), advice)
        }
    }
}

/// 技能市场这一类错误的「下一步」。
///
/// **不能复用 `ProviderUnavailable` 那句** —— 它谈的是模型服务与
/// llama-server，而这里坏的是另一个外部服务。
pub(crate) const SKILLHUB_ADVICE: &str = "技能市场（SkillHub）是外部服务，本机没有它的副本。\
     先确认网络能到上游；若是自建或镜像的市场，用 `QUILL_SKILLHUB_HOST` \
     指向可达的地址后重启 quill-server。已安装的技能不受影响，\
     它们本来就在本机磁盘上。";

/// 技能市场「回得太大」这一档的「下一步」。
///
/// 与 [`SKILLHUB_ADVICE`] 是**对偶**的两句，不能混用：
/// 那一句的前提是「市场连不上」，这一句的前提是
/// 「市场好好地回了话，只是回得超过 `skillhub::MAX_HTTP_BYTES`，被我们读的时候拒收了」。
///
/// ## 为什么必须分开
///
/// 掉进上一句时，用户被叫去**检查网络**、去换一个**可达的**镜像 ——
/// 而网络没坏、镜像也是通的：它确实回了东西，只不过回得太大。
/// 换一个可达的镜像只会原样再撞一次同样的上限，
/// 而真正能动的那两件事（换一个更小的技能包、把上限调高）一个字都没被提到。
///
/// 措辞照 `SKILLHUB_ADVICE` 的路子：先说清这次的**前提**是什么，
/// 再给能照着做的下一步，最后仍要提一句「已安装的技能不受影响」——
/// 那是这个文件里所有市场类错误共有的收尾，用户最关心的就是这句。
pub(crate) const SKILLHUB_OVERSIZE_ADVICE: &str =
    "技能市场（SkillHub）是通的，回来的响应比我们单次接收的上限还大，\
     于是在读的过程中被我们拒收了 —— 网络与镜像都没有坏，\
     重试同一个地址不会有结果。\
     下一步：换一个更小的技能包，或把搜索条件收窄后重试；\
     若某个市场的响应本来就大于上限，用 `QUILL_SKILLHUB_HOST` \
     指向一个响应更小的 SkillHub，或改高单次接收的上限\
     （它是编译期常量 `MAX_HTTP_BYTES`，不读环境变量）后重新构建 quill-server。\
     已安装的技能不受影响，它们本来就在本机磁盘上。";

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
///
/// 认证：同 [`skill_hub_list`] —— 读接口也是要打到外部服务上的，
/// 逐 handler 的 `AuthUser` 是唯一的门。
pub async fn skill_hub_search(
    _user: AuthUser,
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
///
/// 认证：同 [`skill_hub_list`]。这个端点最需要它 —— `kind=all` 一次六发上游，
/// 未登录就能按需放大。
pub async fn skill_hub_rankings(
    _user: AuthUser,
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
    let installed = install_one_skill(&state, user.0.user_id, &slug).await?;
    Ok(Json(json!({
        "installed": skills_repo::to_json(&installed.row),
        "installed_count": 1,
        "source_slug": installed.slug,
        // 包里那个文件被当成了正文。**说出来**，别让人以为装的是 SKILL.md。
        "body_file": installed.body_file,
        "already_present": installed.already_present,
    }))
    .into_response())
}

/// 装**一个**单技能：下载、解包、落盘、登记。**HTTP 处理器与专家市场共用它。**
///
/// 抽出来的理由：市场里「装一个专家」会连带装它点名的那些技能，
/// 那时已经没有请求上下文可用了 —— 所以核芯不能住在 handler 里。
/// 复制一份的话，两边迟早在「装完要不要启用」这种地方分叉。
pub(crate) struct InstalledSkill {
    pub row: skills_repo::SkillRow,
    pub slug: String,
    pub body_file: String,
    pub already_present: bool,
}

pub(crate) async fn install_one_skill(
    state: &AppState,
    user_id: quill_adapters::UserId,
    slug: &str,
) -> Result<InstalledSkill, ApiError> {
    let safe = crate::skillhub::validate_slug(slug).map_err(|e| hub_error("请求不合法", e))?;

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

    // 已经在册上就是「本来就有」，**不重复写盘**：
    // 市场连带安装时会把同一个技能点到多次，那属于一次安装里的正常情况。
    let db = state.db()?;
    if let Some(existing) = skills_repo::get(db, user_id, &name)
        .await
        .map_err(|e| map_err("查技能", e))?
    {
        return Ok(InstalledSkill {
            row: existing,
            slug: safe,
            body_file,
            already_present: true,
        });
    }

    let saved = skills_repo::upsert(
        db,
        user_id,
        skills_repo::SkillRow {
            name: name.clone(),
            version: "0.1.0".to_string(),
            source: HUB_SOURCE.to_string(),
            source_ref: Some(safe.clone()),
            description: hub_description(&body),
            // 装完不启用。理由与技能包那条一样，见 skill_hub_install 的文档。
            enabled: false,
            install_path: path.display().to_string(),
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

    Ok(InstalledSkill {
        row: saved,
        slug: safe,
        body_file,
        already_present: false,
    })
}

/// 技能包里的 manifest：它是元数据，不是技能正文。
const MANIFEST_FILE: &str = "manifest.json";

/// 一次安装的**打算**：每个技能名用包里哪个条目，以及哪些条目被挡掉了。
///
/// 单独一个结构体、单独一个函数，是为了让「哪些条目会落到同一个技能名上」
/// 这件事在**动磁盘之前**就算完。撞名一旦发生在写盘之后，就是一个
/// 「库里有行、正文被另一篇盖掉」的技能 —— 那正是本项目最恨的
/// 「看起来装成功了、实际内容是随机的」。
struct InstallPlan {
    /// 要装的：`(unpacked.files 里的下标, 技能名)`。
    /// **一个技能名最多出现一次。**
    picks: Vec<(usize, String)>,
    /// 被挡掉的：`(被挡掉的条目名, 占住那个技能名的条目名, 技能名)`。
    /// 说出来，不静默丢。
    skipped: Vec<(String, String, String)>,
}

/// 把包内条目映射成技能名，并**挡掉撞名的那些**。
///
/// ## 为什么名字取包 slug，却不能因此把整包正文都装成同一个名字
///
/// 技能名必须是包 slug（见 `skill_hub_install` 的文档，以及
/// `hub_source_tests` 里钉住的那条测试）——`tool_name` 就是 slug，
/// 而这个名字会直接出现在对话工具表里让模型调。
///
/// 但有 manifest 时，**包里每个 `.md` 都会算成同一个 slug**。而
/// `skills.name` 上有 `UNIQUE(user_id, name)`、`skill_file` 又给出同一个
/// `<slug>.md`：第二个条目的正文会盖掉第一个，库里也只剩一行 ——
/// 而循环照样 `installed.push`，`installed_count` 于是报 2、实际只存在 1 个。
/// 没有 manifest 时 `skillhub_unpack` 把条目名拍成 basename 且不去重，
/// `a/x.md` 与 `b/x.md` 撞成同一个 `x`，是同一个问题。
///
/// **这里选「第一个占住、后面逐条报成撞名」，而不是整包失败**：
/// 一个包同时带 `SKILL.md` 与 `README.md` 是上游的正常形状，为它把整包
/// 装不下去，是拿一个真问题换一个更大的问题。哪个条目占住，与单技能那条
/// （`skill_hub_install_skill` 取 `.first()`）保持同一个口径。
///
/// **也不能给 slug 随便加个后缀来「消歧」** —— 上游没有那个技能，
/// 名字是编的，而这个名字正是模型在工具表里看到的东西。
///
/// 被挡掉的那几项在响应里逐条点名（`skipped_duplicate`），
/// 与 `installed_count` 一起构成「装到几个、落下几个」。
fn plan_install(
    manifest: Option<&crate::skillhub::HubManifest>,
    files: &[(String, String)],
) -> Result<InstallPlan, ApiError> {
    let mut picks: Vec<(usize, String)> = Vec::new();
    let mut taken: Vec<(String, String)> = Vec::new(); // (技能名, 占住它的条目名)
    let mut skipped: Vec<(String, String, String)> = Vec::new();

    for (i, (name, _)) in files.iter().enumerate() {
        // manifest.json 不是技能，是元数据。**不算进「装了几个」**。
        if name == MANIFEST_FILE {
            continue;
        }
        // 技能名优先取**包的 slug**，不是包内文件名。
        //
        // 实测：`tech-test-automation` 的 zip 里那篇正文叫 `identify.md`，
        // 按文件名装出来就叫 `identify` —— 这个名字既看不懂，也会直接
        // 出现在对话工具表里让模型调（`tool_name` 就是 slug）。
        // 一个包 = 一个技能，名字就该是那个包。
        let stem = manifest
            .map(|m| m.slug.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| name.trim_end_matches(".md").to_string());
        // 归一失败（上游给了个带斜杠的 slug）时退回文件名，而不是整包失败 ——
        // `normalize_name` 的错误信息里没有「换个名字继续」这句话。
        let slug_row = match mcp_repo::normalize_name(&stem) {
            Ok(n) => n,
            Err(_) => mcp_repo::normalize_name(name.trim_end_matches(".md"))
                .map_err(ApiError::bad_request)?,
        };

        match taken.iter().find(|(s, _)| *s == slug_row) {
            // 已经有条目占住这个名字了：这一篇**不装**。
            // 装上去只会覆盖掉已装的那一篇，然后多报一个不存在的技能。
            Some((_, keeper)) => skipped.push((name.clone(), keeper.clone(), slug_row)),
            None => {
                taken.push((slug_row.clone(), name.clone()));
                picks.push((i, slug_row));
            }
        }
    }

    Ok(InstallPlan { picks, skipped })
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
/// 落盘之前还要**先把要装哪几个算完**（[`plan_install`]）：多个条目算到
/// 同一个技能名上时只装第一个，其余逐条报成撞名。先算后写，写之前就确定了
/// 「这一包到底会产出几个技能」，也就不会出现「报装 2 个、正文只有 1 篇」。
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
    let safe = crate::skillhub::validate_slug(&slug).map_err(|e| hub_error("请求不合法", e))?;

    let bytes = match crate::skillhub::download_skillset(&safe).await {
        Ok(b) => b,
        Err(e) => return Err(hub_error(&format!("下载技能包 {safe} 失败"), e)),
    };

    let unpacked =
        crate::skillhub_unpack::unpack(&bytes).map_err(|e| ApiError::bad_request(e.message()))?;

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
        .find(|(name, _)| name == MANIFEST_FILE)
        .and_then(|(_, body)| crate::skillhub::parse_manifest(body));

    let referenced: Vec<String> = manifest
        .as_ref()
        .map(|m| m.referenced_slugs())
        .unwrap_or_default();

    // **先算清楚要装哪几个，再动磁盘。** 撞名的条目在这里就被挡掉，
    // 不会先写进去再被第二篇盖掉 —— 那会装出一个正文随机、却报着「装好了」的技能。
    let plan = plan_install(manifest.as_ref(), &unpacked.files)?;

    let dir = skill_dir(&state.config);
    std::fs::create_dir_all(&dir)
        .map_err(|e| ApiError::internal(format!("创建技能目录 {} 失败：{e}", dir.display())))?;

    let db = state.db()?;
    let mut installed: Vec<Value> = Vec::new();

    for (idx, slug_row) in &plan.picks {
        let (_, body) = &unpacked.files[*idx];
        // 落盘路径只允许 `skill_file` 这一处计算（`sanitize_name` 已在解包时挡过穿越）。
        let path = skill_file(&dir, slug_row)?;

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
        // 这个数现在等于**装完真的存在**的技能数：撞名的条目在 `plan_install`
        // 里就被挡掉并记进 `skipped_duplicate`，不会在这里被多算一次。
        "installed_count": installed.len(),
        // **挡掉的要逐条点名。** 只给一个数字的话，用户会以为那些条目的正文
        // 也装上了 —— 它们没有：同一个技能名在磁盘上只有一份正文文件。
        "skipped_duplicate": plan.skipped.iter()
            .map(|(entry, keeper, skill)| json!({
                "entry": entry,
                "would_have_overwritten": skill,
                "kept_entry": keeper,
            }))
            .collect::<Vec<_>>(),
        "skipped_duplicate_count": plan.skipped.len(),
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
pub(crate) fn hub_description(body: &str) -> String {
    let head = body.split("---").nth(1).unwrap_or("");
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
/// `pub(crate)` 是因为 `tool_sources::DbToolSources`（挂工具表时读技能正文）
/// 也要按同一个目录读正文 ——
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

/// 技能正文路径 —— 全项目**唯一**的算法。
///
/// 三个调用点（写入 `skill_file`、列举 `list_skills`、挂载工具表时读正文的
/// `tool_sources::DbToolSources::skills`）必须都走这里：各写一份必然漂，而漂了的后果
/// 是「界面说一个位置、模型读另一个位置」，而且没有任何迹象指向原因。
///
/// **为什么一个函数要同时服务两类名字**：
/// 写入侧的名字是 URL slug，刚过完 `mcp_repo::normalize_name`；
/// 读取侧的名字直接来自 `skills.name`（migration 0001 上有 CHECK）。
/// 两者的可信度不一样，所以这里**不再**依赖调用方的归一，
/// 自己做最后一道防线：空名、带路径分隔符、带 `.` 的名字一律拒绝
/// （合法的 kebab-case 技能名里根本不会出现它们）。
/// 这样「DB 里有一行脏名字」也只会退化成「磁盘上没正文」，
/// 而不是读到目录外面去。
///
/// 归一化走 `pathsafe::is_within`，两侧同一口径 —— 这一条是硬约束：
/// Windows 上 `canonicalize` 带 `\\?\` 前缀，只归一化一侧会让**所有**
/// 技能写入都被判成「落在目录之外」。
pub(crate) fn skill_body_path(
    root: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, ApiError> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains('.') {
        return Err(ApiError::bad_request(format!(
            "SKILL 名称 {name:?} 不是合法的技能名。\
             下一步：只用小写字母、数字与连字符。"
        )));
    }
    let path = root.join(format!("{name}.md"));
    // 问的是**父目录**而不是文件本身：文件还没落盘，canonicalize
    // 必然失败走兜底，那时的判定就退化成字面前缀比较了。
    let parent = path.parent().unwrap_or(root);
    if !crate::pathsafe::is_within(root, parent) {
        return Err(ApiError::bad_request(format!(
            "SKILL 名称 {name:?} 解析后落在技能目录之外，已拒绝。\
             下一步：只用小写字母、数字与连字符。"
        )));
    }
    Ok(path)
}

/// 把 slug 映射成安全的文件名（写入侧的入口）。
///
/// **路径穿越防护**：slug 来自 URL/body，直接拼进路径就能用 `../` 跳出目录，
/// 写到任意位置。归一后仍要确认最终路径落在根目录内 —— 归一规则被绕过时
/// 这道检查是最后一道。
pub(crate) fn skill_file(
    root: &std::path::Path,
    slug: &str,
) -> Result<std::path::PathBuf, ApiError> {
    let name = mcp_repo::normalize_name(slug).map_err(ApiError::bad_request)?;
    skill_body_path(root, &name)
}

/// 读 SKILL 正文。**读不到就当空**，由调用方决定这算「没配」还是「坏了」——
/// 静默变成空串会让「文件被删了」看起来像「用户没写内容」。
pub(crate) fn read_skill_body(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

pub(crate) fn write_skill_body(path: &std::path::Path, content: &str) -> Result<(), ApiError> {
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
    // 素材走与对话同一条端口实现（`DbToolSources` → 既有查询函数）。
    let sources = crate::tool_sources::DbToolSources::shared(&state, dir.clone());
    let builtin_names: Vec<quill_provider::ToolSpec> =
        crate::tools::ToolRegistry::builtin(sources, user.0.user_id).specs();

    let mut items = Vec::with_capacity(rows.len());
    let mut taken = builtin_names;
    for r in &rows {
        let mut j = skills_repo::to_json(r);
        // 算不出路径（DB 里名字不合法）就当「磁盘上没正文」—— 与正文
        // 文件被删是同一种形状，下面的 `content_missing` / `NotMounted`
        // 照常报得出来，不另造一套说法。
        let body = match skill_body_path(&dir, &r.name) {
            Ok(p) => read_skill_body(&p),
            Err(_) => String::new(),
        };
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
            let vis = crate::tools::skill_visibility(
                &taken,
                &crate::tool_sources::skill_tool_row(r, &body),
            );
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
            ApiError::bad_request(
                "slug 必填。\u{0a}下一步：{\"slug\":\"code-review\",\"content\":\"…\"}",
            )
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
        // `tool_allowlist` **接受并忽略**（2026-10-09 删列，Q102 / ISSUE-008）：
        // 这一列从来只被写、没被读过，所以忽略它之后行为与删之前**完全一样**；
        // 继续接受这个键是为了让老客户端不至于因为多传一个字段就 400。
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
            "请求体必须是 JSON 对象。下一步：{\"enabled\": true} 或 {\"enabled\": false}。"
                .to_string(),
        )
    })?;

    // `only_keys_at` 会把多余的键逐个点名，而不是只说「参数不对」。
    crate::api_experts::only_keys_at(&patch, &["enabled"], "SKILL 的 PATCH")?;

    let enabled = obj.get("enabled").and_then(Value::as_bool).ok_or_else(|| {
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
        .ok_or_else(|| ApiError::internal(format!("刚改完的 SKILL {norm} 立刻就读不到了")))?;

    Ok(Json(json!({ "skill": skills_repo::to_json(&saved) })).into_response())
}
