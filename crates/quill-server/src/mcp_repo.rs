//! MCP 服务器配置的存储口径。
//!
//! 表在 0001 就建好了，0007 把 `transport` 对齐前端的
//! `stdio | streamable_http | sse`，并补了 `cwd` / `max_concurrent_calls` /
//! `enabled_capabilities` 三列。
//!
//! **这里只管存配置，不连服务器。** 协议层（`rmcp`）另算。把两者混在一起
//! 的后果是：保存配置时会因为「连不上」而失败，而连不上往往是配置本身就错、
//! 或者目标机器根本没起 —— 用户没法先把配置登记好。分工是：这里保证
//! 「写进去能读出来、隔离不串、软删能复用」，连不连由上层显式决定。
//!
//! 前端是全量提交（POST 整个 `servers` 数组），所以写入是「按 name 做差分」：
//! 数组里没有的软删，数组里有的 upsert。**不是**逐行 insert —— 那会留下
//! 用户已经删掉的行。

use std::collections::HashMap;

use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;

use quill_adapters::UserId;

use crate::db::{digest32, invariant_broken, now_ms, storage_error, DbBridge};

const OP_LIST: &str = "列出 MCP 服务器";
const OP_WRITE: &str = "写入 MCP 服务器";
const OP_PATCH: &str = "更新 MCP 服务器";

/// 显式列清单。`*` 会在列顺序变化时静默错位。
pub const COLUMNS: &str = "name, transport, command, args_json, env_json, url, headers_json, \
     enabled, timeout_ms, description, cwd, max_concurrent_calls, enabled_capabilities_json, \
     asset_hash, created_at, updated_at";

pub const LIST_SQL: &str = "SELECT name, transport, command, args_json, env_json, url, \
     headers_json, enabled, timeout_ms, description, cwd, max_concurrent_calls, \
     enabled_capabilities_json, asset_hash, created_at, updated_at FROM mcp_servers \
     WHERE user_id = ? AND deleted_at IS NULL ORDER BY name ASC";

/// 单条读取。**列清单与 [`LIST_SQL`] 必须逐列一致**（`row_from` 按列名取值，
/// 少一列会在运行时报「读列失败」，而不是编译期报错）—— 有一条测试逐列比这两条 SQL。
///
/// 谓词口径与 `LIST_SQL` 相同（同一个 `user_id` 隔离 + 同一个软删过滤），
/// 只多一个 `name`。少 `user_id` 就是跨用户可改。
pub const GET_SQL: &str = "SELECT name, transport, command, args_json, env_json, url, \
     headers_json, enabled, timeout_ms, description, cwd, max_concurrent_calls, \
     enabled_capabilities_json, asset_hash, created_at, updated_at FROM mcp_servers \
     WHERE user_id = ? AND name = ? AND deleted_at IS NULL";

/// 单条更新的写语句。**只改可见行、只改自己名下的行**：三个谓词少一个就是
/// 跨用户可改或改到软删行；`created_at` 不在 SET 列表里 —— 改名保留行身份，
/// 不能顺手把「什么时候加的」重置。
pub const PATCH_SQL: &str = "UPDATE mcp_servers SET name = ?, transport = ?, command = ?, \
     args_json = ?, env_json = ?, url = ?, headers_json = ?, enabled = ?, timeout_ms = ?, \
     description = ?, cwd = ?, max_concurrent_calls = ?, enabled_capabilities_json = ?, \
     asset_hash = ?, updated_at = ? WHERE user_id = ? AND name = ? AND deleted_at IS NULL";

/// 一行配置 —— 定义已随 MCP 协议客户端搬进内核层（queue Q014）：
/// 内核要拿它拉起进程，存储要按列拼它，**唯一一份**只能住在 `quill-core`，
/// 否则 `quill-core` 得反过来依赖本 crate。这里 re-export，路径不变。
pub use quill_core::mcp::McpServerRow;

/// 配置内容的指纹。用于判断「内容有没有真的变」——没变就整行跳过，
/// 否则每次点保存都会让所有行看起来都变过一遍，`updated_at` 也就失去意义了。
///
/// 长度必须是 32 字节：`mcp_servers.asset_hash` 的 CHECK 就是
/// `length(asset_hash) = 32`。所以这里复用 `db::digest32` —— 它本来就返回
/// 32 字节，`experts` 也在用。早先这里截 SHA-256 到 16 字节，编译能过、
/// 单元测试也过，但**每一行写入都会被 CHECK 拒掉**，而错误只会说
/// "CHECK constraint failed"，不告诉你是哪一列。
///
/// 摘要刻意不含 `name` 与时间戳：它比的是「配置内容」。
pub fn asset_hash(row: &McpServerRow) -> Vec<u8> {
    let mut acc: Vec<u8> = Vec::new();
    field(&mut acc, row.transport.as_bytes());
    field(&mut acc, row.command.as_deref().unwrap_or("").as_bytes());
    // 数组必须**按元素写**，不能 join 成一个字符串再写：`[""]` 与 `[]`
    // join 后都是空串，前缀长度也一样，于是这两份不同的配置算出同一个指纹。
    // 同样的道理适用于能力列表：`["a\u{1}b"]` 与 `["a","b"]`。
    list_of(&mut acc, &row.args);
    field(&mut acc, row.url.as_deref().unwrap_or("").as_bytes());
    field(&mut acc, row.cwd.as_deref().unwrap_or("").as_bytes());
    field(&mut acc, row.description.as_bytes());
    field(&mut acc, if row.enabled { b"1" } else { b"0" });
    let timeout = row.timeout_ms.to_string();
    field(&mut acc, timeout.as_bytes());
    let conc = match row.max_concurrent_calls {
        Some(n) => n.to_string(),
        None => "-".to_string(),
    };
    field(&mut acc, conc.as_bytes());
    // 三态：`None`=全禁、`Some(vec![])`=全开、`Some(v)`=精确列举。必须能被
    // 指纹区分开，否则「把全开改成全禁」会被当成没改。
    match &row.enabled_capabilities {
        None => field(&mut acc, b"caps-off"),
        Some(v) => {
            field(&mut acc, b"caps-on");
            list_of(&mut acc, v);
        }
    }
    // env 与 headers 是两个区段，不能合成一张 map —— 同名字段会互相覆盖，
    // 于是「只改了 header 里的 TOKEN」可能算不出指纹变化。
    kv_section(&mut acc, b"env", &row.env);
    kv_section(&mut acc, b"headers", &row.headers);
    digest32("mcp.asset_hash", &[&acc]).to_vec()
}

/// 长度前缀式分段。不加长度的话，`["a"]` 与 `["a", ""]` 会拼出同样的字节，
/// 指纹一样而内容不同。
fn field(acc: &mut Vec<u8>, b: &[u8]) {
    acc.extend_from_slice(&(b.len() as u32).to_be_bytes());
    acc.extend_from_slice(b);
}

fn list_of(acc: &mut Vec<u8>, items: &[String]) {
    acc.extend_from_slice(&(items.len() as u32).to_be_bytes());
    for it in items {
        field(acc, it.as_bytes());
    }
}

fn kv_section(acc: &mut Vec<u8>, section: &[u8], pairs: &[(String, String)]) {
    field(acc, section);
    // 键排序后再算：map 的迭代顺序不保证稳定，直接顺序哈希会让同样的配置
    // 每次算出不同的指纹。
    let mut sorted: Vec<&(String, String)> = pairs.iter().collect();
    sorted.sort();
    for (k, v) in sorted {
        field(acc, k.as_bytes());
        field(acc, v.as_bytes());
    }
}

/// 与 `list` 同一条 SQL、同一套映射，只是**同步**入口。
///
/// 为什么要有它：内核端口 `quill_core::tools::ToolSources::mcp_servers` 是同步
/// 方法（`ToolRegistry::builtin*` 本身同步），而 `DbBridge::call` 本来就是阻塞
/// 调用 —— 真正的实现放在这里，`list` 只是它的 async 外壳（搬进内核前，
/// `list` 自己就是那个外壳，行为一字未改）。
pub fn list_blocking(
    db: &DbBridge,
    uid: UserId,
) -> Result<Vec<McpServerRow>, quill_agent::AgentError> {
    let b = crate::db::blob_of(&uid);
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let rows = sqlx::query(LIST_SQL)
                .bind(&b)
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error(OP_LIST, e))?;
            rows.iter().map(row_from).collect()
        })
    })
}

pub async fn list(
    db: &DbBridge,
    uid: UserId,
) -> Result<Vec<McpServerRow>, quill_agent::AgentError> {
    list_blocking(db, uid)
}

/// 单条读取（同一用户的可见行）。`PATCH` 的前置读用这一条。
pub async fn get(
    db: &DbBridge,
    uid: UserId,
    name: &str,
) -> Result<Option<McpServerRow>, quill_agent::AgentError> {
    let b = crate::db::blob_of(&uid);
    let name = name.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let row = sqlx::query(GET_SQL)
                .bind(&b)
                .bind(&name)
                .fetch_optional(&pool)
                .await
                .map_err(|e| storage_error(OP_LIST, e))?;
            row.as_ref().map(row_from).transpose()
        })
    })
}

/// `PATCH` 单条的落库结局。
///
/// 用枚举而不是 `Result<McpServerRow>`：这一层要能分开说「没这一行」「想改成的
/// 名字被占了」「一个字都没改」「真改了」—— 它们在 HTTP 上是 404 / 409 / 200
/// 三种不同的回答，压成一个 `Option` 就只能靠调用方猜。
#[derive(Debug)]
pub enum PatchOutcome {
    /// 这个用户名下没有这一行（不存在，或已被软删）。
    NotFound,
    /// 想改成的名字已被**本用户**的另一行占着（活跃或软删都算：主键是
    /// `(user_id, name)`，软删的行仍然占着那个名字）。别的用户有同名行不算冲突。
    NameTaken,
    /// 名字与配置内容都没变：**没有写库**，`updated_at` 也没动。
    Unchanged(McpServerRow),
    /// 真写进去了；这是回读得到的、库里现在的那一行。
    Updated(McpServerRow),
}

/// 单条局部更新的落库：`next` 是**合并完成后**的整行（合并语义在 HTTP 层，见
/// `api_extensions::patch_mcp`），这一层只管「按主键定位、检查冲突、写、回读」。
///
/// ## 改名走 `UPDATE`，不是 delete + insert
///
/// 主键是 `(user_id, name)`（migrations/0007 第 48 行），改名在本用户内是
/// 「同一行的键换了」：`UPDATE ... SET name=?` 保留 `created_at` 与行身份。
/// 走 delete + insert 的话，每次改名都会把 `created_at` 重置成现在 ——
/// 界面上「什么时候加的」会凭空变成「刚刚」。
///
/// ## `updated_at` 的语义按指纹 + 名字两条判定
///
/// `asset_hash` **刻意不含 `name` 与时间戳**（见 [`asset_hash`] 的说明），
/// 所以「只改名字」的内容指纹不变。若照抄 `replace_all` 里「指纹没变就整行跳过」，
/// 改名就会被静默丢掉 —— 于是这里改成：**名字变了或指纹变了**才算改过，
/// 两者都没变就一个字都不写（`updated_at` 自然不动，与全量提交那条语义一致）。
pub async fn apply_patch(
    db: &DbBridge,
    uid: UserId,
    current_name: &str,
    next: McpServerRow,
) -> Result<PatchOutcome, quill_agent::AgentError> {
    let b = crate::db::blob_of(&uid);
    let current_name = current_name.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let mut tx = pool.begin().await.map_err(|e| storage_error(OP_PATCH, e))?;

            // 事务内再读一次：HTTP 层那次读只用来合并字段，冲突判定必须贴着写。
            let current = sqlx::query(GET_SQL)
                .bind(&b)
                .bind(&current_name)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| storage_error(OP_PATCH, e))?;
            let Some(current) = current.as_ref().map(row_from).transpose()? else {
                // 提前返回时事务由 Drop 回滚；这里本来就没写过，回滚是空操作。
                return Ok(PatchOutcome::NotFound);
            };

            let renaming = current.name != next.name;
            if renaming {
                // 不加 `deleted_at IS NULL`：软删的行仍占着主键的那个名字，
                // 直接 UPDATE 会撞 UNIQUE 并报成一条难懂的 500。
                //
                // 并发窗口：两条同名改名的请求可能同时通过这条预检，后到的那条
                // 会在 UPDATE 上撞 UNIQUE → 一条 500，而不是静默把行改错。
                // 单用户本地应用接受这个窗口（无声的错误才是不能接受的）。
                let taken =
                    sqlx::query("SELECT 1 FROM mcp_servers WHERE user_id = ? AND name = ? LIMIT 1")
                        .bind(&b)
                        .bind(&next.name)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(|e| storage_error(OP_PATCH, e))?;
                if taken.is_some() {
                    return Ok(PatchOutcome::NameTaken);
                }
            }

            let content_changed = asset_hash(&current) != asset_hash(&next);
            if !renaming && !content_changed {
                return Ok(PatchOutcome::Unchanged(current));
            }

            let args_json =
                serde_json::to_string(&next.args).map_err(|e| storage_error(OP_PATCH, e))?;
            let env_json = serde_json::to_string(
                &next
                    .env
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeMap<_, _>>(),
            )
            .map_err(|e| storage_error(OP_PATCH, e))?;
            let headers_json = serde_json::to_string(
                &next
                    .headers
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeMap<_, _>>(),
            )
            .map_err(|e| storage_error(OP_PATCH, e))?;
            let caps_json = match &next.enabled_capabilities {
                None => None,
                Some(v) => Some(serde_json::to_string(v).map_err(|e| storage_error(OP_PATCH, e))?),
            };
            let hash = asset_hash(&next);
            let now = now_ms();

            sqlx::query(PATCH_SQL)
                .bind(&next.name)
                .bind(&next.transport)
                .bind(&next.command)
                .bind(&args_json)
                .bind(&env_json)
                .bind(&next.url)
                .bind(&headers_json)
                .bind(if next.enabled { 1i64 } else { 0i64 })
                .bind(next.timeout_ms)
                .bind(&next.description)
                .bind(&next.cwd)
                .bind(next.max_concurrent_calls)
                .bind(&caps_json)
                .bind(&hash)
                .bind(now)
                .bind(&b)
                .bind(&current_name)
                .execute(&mut *tx)
                .await
                .map_err(|e| storage_error(OP_PATCH, e))?;

            // 回读而不是把 `next` 返回：调用方要看到库里现在的样子。
            let row = sqlx::query(GET_SQL)
                .bind(&b)
                .bind(&next.name)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| storage_error(OP_PATCH, e))?;
            let row = row
                .as_ref()
                .map(row_from)
                .transpose()?
                .ok_or_else(|| invariant_broken("刚更新完的 MCP 行在同一个事务里读不回来"))?;

            tx.commit().await.map_err(|e| storage_error(OP_PATCH, e))?;
            Ok(PatchOutcome::Updated(row))
        })
    })
}

/// 全量覆盖：按 name 差分软删 + upsert。
///
/// 一次事务里做完，中途失败不留「删了一半」的中间态。
pub async fn replace_all(
    db: &DbBridge,
    uid: UserId,
    incoming: Vec<McpServerRow>,
) -> Result<Vec<McpServerRow>, quill_agent::AgentError> {
    let b = crate::db::blob_of(&uid);
    db.call(move |pool, _rt| {
        Box::pin(async move {
            // 名字先归一再开事务：`keep` 要和库里的键（已归一）逐字可比。
            // 归一放在事务外，是为了让非法名字在**没开事务**时就失败 ——
            // 否则前面几步白写，回滚还要多付一次代价。
            let mut incoming = incoming;
            for s in incoming.iter_mut() {
                s.name = normalize_name(&s.name).map_err(invariant_broken)?;
            }
            let mut tx = pool.begin().await.map_err(|e| storage_error(OP_WRITE, e))?;

            // 库里现有的（name → 指纹）。指纹要留着：下面靠它判断这行是不是
            // 白写。只取 name 会让「内容没变就别动 updated_at」这条优化落空。
            let existing: HashMap<String, Vec<u8>> = sqlx::query(
                "SELECT name, asset_hash FROM mcp_servers \
                 WHERE user_id = ? AND deleted_at IS NULL",
            )
            .bind(&b)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| storage_error(OP_WRITE, e))?
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("name"),
                    r.get::<Vec<u8>, _>("asset_hash"),
                )
            })
            .collect();

            let keep: Vec<&str> = incoming.iter().map(|s| s.name.as_str()).collect();
            let now = now_ms();

            // 1) 本次没提到的 → 软删。软删而不是物理删：name 是主键的一部分，
            //    物理删会让「同名重建」丢历史；软删后同名可复用。
            for name in existing.keys() {
                if !keep.contains(&name.as_str()) {
                    sqlx::query(
                        "UPDATE mcp_servers SET deleted_at = ?, updated_at = ? \
                         WHERE user_id = ? AND name = ? AND deleted_at IS NULL",
                    )
                    .bind(now)
                    .bind(now)
                    .bind(&b)
                    .bind(name)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| storage_error(OP_WRITE, e))?;
                }
            }

            // 2) 提交进来的 → upsert。内容一字未改就整行跳过。
            for s in incoming {
                let hash = asset_hash(&s);
                if existing.get(&s.name) == Some(&hash) {
                    continue;
                }
                let args_json =
                    serde_json::to_string(&s.args).map_err(|e| storage_error(OP_WRITE, e))?;
                let env_json = serde_json::to_string(
                    &s.env
                        .into_iter()
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
                .map_err(|e| storage_error(OP_WRITE, e))?;
                let headers_json = serde_json::to_string(
                    &s.headers
                        .into_iter()
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
                .map_err(|e| storage_error(OP_WRITE, e))?;
                let caps_json = match &s.enabled_capabilities {
                    None => None,
                    Some(v) => {
                        Some(serde_json::to_string(v).map_err(|e| storage_error(OP_WRITE, e))?)
                    }
                };

                // 软删过的同名行要先复活：主键是 (user_id, name)，直接 INSERT
                // 会撞唯一键。
                sqlx::query(
                    "INSERT INTO mcp_servers (user_id, name, transport, command, args_json, \
                     env_json, url, headers_json, enabled, timeout_ms, description, cwd, \
                     max_concurrent_calls, enabled_capabilities_json, asset_hash, created_at, \
                     updated_at, deleted_at) \
                     VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,NULL) \
                     ON CONFLICT(user_id, name) DO UPDATE SET \
                       transport=excluded.transport, command=excluded.command, \
                       args_json=excluded.args_json, env_json=excluded.env_json, \
                       url=excluded.url, headers_json=excluded.headers_json, \
                       enabled=excluded.enabled, timeout_ms=excluded.timeout_ms, \
                       description=excluded.description, cwd=excluded.cwd, \
                       max_concurrent_calls=excluded.max_concurrent_calls, \
                       enabled_capabilities_json=excluded.enabled_capabilities_json, \
                       asset_hash=excluded.asset_hash, updated_at=excluded.updated_at, \
                       deleted_at=NULL",
                )
                .bind(&b)
                .bind(&s.name)
                .bind(&s.transport)
                .bind(&s.command)
                .bind(&args_json)
                .bind(&env_json)
                .bind(&s.url)
                .bind(&headers_json)
                .bind(if s.enabled { 1i64 } else { 0i64 })
                .bind(s.timeout_ms)
                .bind(&s.description)
                .bind(&s.cwd)
                .bind(s.max_concurrent_calls)
                .bind(&caps_json)
                .bind(&hash)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| storage_error(OP_WRITE, e))?;
            }

            tx.commit().await.map_err(|e| storage_error(OP_WRITE, e))?;

            // 回读，让调用方拿到「库里现在的样子」而不是「我们以为写进去的样子」。
            let rows = sqlx::query(LIST_SQL)
                .bind(&b)
                .fetch_all(&pool)
                .await
                .map_err(|e| storage_error(OP_LIST, e))?;
            rows.iter().map(row_from).collect()
        })
    })
}

/// name 归一：小写 kebab-case。与 `mcp_servers` 的 CHECK 兼容。
///
/// 失败信息带上具体值 —— 「名字非法」这种错不说清楚的话，用户根本不知道
/// 哪个字符有问题。
pub fn normalize_name(raw: &str) -> Result<String, String> {
    let t = raw.trim().to_lowercase();
    let out: String = t
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() || out.len() > 64 {
        return Err(format!(
            "MCP 服务器名 {raw:?} 非法：归一后为空或超过 64 字符（得到 {out:?}）。\
             下一步：用小写字母、数字与连字符，例如 `filesystem`。"
        ));
    }
    if out.contains("--") {
        return Err(format!(
            "MCP 服务器名 {raw:?} 归一后出现连续连字符：{out:?}。\
             下一步：名字只用单个连字符分隔单词，例如 `my-tools`。"
        ));
    }
    Ok(out)
}

fn s(row: &SqliteRow, k: &str) -> Result<String, quill_agent::AgentError> {
    row.try_get::<String, _>(k)
        .map_err(|e| storage_error(OP_LIST, e))
}

fn os(row: &SqliteRow, k: &str) -> Result<Option<String>, quill_agent::AgentError> {
    row.try_get::<Option<String>, _>(k)
        .map_err(|e| storage_error(OP_LIST, e))
}

fn oi(row: &SqliteRow, k: &str) -> Result<Option<i64>, quill_agent::AgentError> {
    row.try_get::<Option<i64>, _>(k)
        .map_err(|e| storage_error(OP_LIST, e))
}

fn json_str(raw: &str, column: &str) -> Result<Vec<String>, quill_agent::AgentError> {
    // 报错要带列名：库里存的是「别人写进去的字节」，解析失败是迟早的事，
    // 只说「读取失败」的话，没人知道该去看哪一列。
    serde_json::from_str::<Vec<String>>(raw).map_err(|e| {
        storage_error(
            &format!("{OP_LIST}（{column} 列的内容不是合法 JSON 数组）"),
            e,
        )
    })
}

fn json_map(raw: &str, column: &str) -> Result<Vec<(String, String)>, quill_agent::AgentError> {
    let m: std::collections::BTreeMap<String, String> = serde_json::from_str(raw).map_err(|e| {
        storage_error(
            &format!("{OP_LIST}（{column} 列的内容不是合法的键值对对象）"),
            e,
        )
    })?;
    Ok(m.into_iter().collect())
}

fn row_from(r: &SqliteRow) -> Result<McpServerRow, quill_agent::AgentError> {
    let caps_json = os(r, "enabled_capabilities_json")?;
    let enabled_capabilities = match caps_json {
        None => None,
        Some(raw) => Some(json_str(&raw, "enabled_capabilities_json")?),
    };
    Ok(McpServerRow {
        name: s(r, "name")?,
        transport: s(r, "transport")?,
        command: os(r, "command")?,
        args: json_str(&s(r, "args_json")?, "args_json")?,
        env: json_map(&s(r, "env_json")?, "env_json")?,
        url: os(r, "url")?,
        headers: json_map(&s(r, "headers_json")?, "headers_json")?,
        enabled: r
            .try_get::<i64, _>("enabled")
            .map_err(|e| storage_error(OP_LIST, e))?
            != 0,
        timeout_ms: r
            .try_get::<i64, _>("timeout_ms")
            .map_err(|e| storage_error(OP_LIST, e))?,
        description: s(r, "description")?,
        cwd: os(r, "cwd")?,
        max_concurrent_calls: oi(r, "max_concurrent_calls")?,
        enabled_capabilities,
        created_at: r
            .try_get::<i64, _>("created_at")
            .map_err(|e| storage_error(OP_LIST, e))?,
        updated_at: r
            .try_get::<i64, _>("updated_at")
            .map_err(|e| storage_error(OP_LIST, e))?,
    })
}

/// 转成前端 `McpServerConfig` 的形状。`null` 与 `[]` 的区别必须保住 ——
/// 前端用它们区分「全禁」与「全开」。
pub fn to_json(r: &McpServerRow) -> Value {
    let mut v = serde_json::Map::new();
    v.insert("name".into(), r.name.clone().into());
    v.insert("transport".into(), r.transport.clone().into());
    v.insert(
        "enabled_capabilities".into(),
        match &r.enabled_capabilities {
            None => Value::Null,
            Some(c) => serde_json::to_value(c).unwrap_or(Value::Null),
        },
    );
    v.insert("enabled".into(), r.enabled.into());
    v.insert("timeout_ms".into(), r.timeout_ms.into());
    v.insert("description".into(), r.description.clone().into());
    if r.transport == "stdio" {
        v.insert(
            "command".into(),
            r.command.clone().unwrap_or_default().into(),
        );
        v.insert("args".into(), r.args.clone().into());
        v.insert(
            "cwd".into(),
            match &r.cwd {
                Some(c) if !c.is_empty() => c.clone().into(),
                _ => Value::Null,
            },
        );
        v.insert("env".into(), map_to_json(&r.env));
    } else if r.transport == "builtin" {
        // 内置服务器的**名字**就写在 `command` 里（见 `quill_core::builtin`）。
        // 不回这一列，界面上就显示不出这台内置服务器是哪一台；而且表单是全量提交 ——
        // 回填时名字丢了，用户下一次点保存必被 400 挡下（「command 必填」）。
        v.insert(
            "command".into(),
            r.command.clone().unwrap_or_default().into(),
        );
    } else {
        v.insert("url".into(), r.url.clone().unwrap_or_default().into());
        v.insert("headers".into(), map_to_json(&r.headers));
    }
    v.insert(
        "max_concurrent_calls".into(),
        match r.max_concurrent_calls {
            Some(n) => n.into(),
            None => Value::Null,
        },
    );
    Value::Object(v)
}

fn map_to_json(pairs: &[(String, String)]) -> Value {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert(k.clone(), v.clone().into());
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(name: &str) -> McpServerRow {
        McpServerRow {
            name: name.to_string(),
            transport: "stdio".into(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "server".into()],
            env: vec![],
            url: None,
            headers: vec![],
            enabled: true,
            timeout_ms: 30_000,
            description: String::new(),
            cwd: None,
            max_concurrent_calls: None,
            enabled_capabilities: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn the_list_sql_always_filters_by_user_and_excludes_soft_deleted() {
        // 少一个谓词就是跨用户泄露。逐字断言，不靠肉眼看。
        assert!(LIST_SQL.contains("user_id = ?"), "必须按用户过滤");
        assert!(LIST_SQL.contains("deleted_at IS NULL"), "必须过滤软删行");
    }

    /// 单条读与单条写的隔离口径。
    ///
    /// `PATCH` 这条路最危险的两件事：改到别人的行、改到软删行。两条语句的
    /// WHERE 都必须同时带 `user_id` / `name` / `deleted_at IS NULL` 三个谓词。
    #[test]
    fn the_single_row_statements_only_touch_this_users_visible_row() {
        for (label, sql) in [("GET_SQL", GET_SQL), ("PATCH_SQL", PATCH_SQL)] {
            let where_ = sql
                .split(" WHERE ")
                .nth(1)
                .unwrap_or_else(|| panic!("{label} 必须有 WHERE"));
            assert!(where_.contains("user_id = ?"), "{label} 必须按用户过滤");
            assert!(where_.contains("name = ?"), "{label} 必须按 name 定位");
            assert!(
                where_.contains("deleted_at IS NULL"),
                "{label} 不许动软删行"
            );
        }

        // 改名保留行身份：`created_at` 与 `deleted_at` 都不该出现在 SET 列表里，
        // 而 `updated_at` 必须在 —— 改了却不更新时间戳，界面上的「更新时间」就是假话。
        let set = PATCH_SQL
            .split(" SET ")
            .nth(1)
            .and_then(|s| s.split(" WHERE ").next())
            .expect("PATCH_SQL 必须是 SET ... WHERE 的形状");
        assert!(
            !set.contains("created_at"),
            "改名不许重置 created_at：{set}"
        );
        assert!(!set.contains("deleted_at"), "更新不许碰 deleted_at：{set}");
        assert!(
            set.contains("updated_at = ?"),
            "真改了就要更新时间戳：{set}"
        );
    }

    /// 两条读语句的列清单必须逐列一致。
    ///
    /// `row_from` 按列名取值，少一列只会在运行时报「读列失败」——
    /// 编译期、clippy 都看不见，只有这条测试看得见。
    #[test]
    fn get_and_list_select_the_same_columns() {
        fn cols(sql: &str) -> String {
            sql.strip_prefix("SELECT ")
                .expect("必须是 SELECT")
                .split(" FROM ")
                .next()
                .expect("必须有 FROM")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        }
        assert_eq!(cols(GET_SQL), cols(LIST_SQL), "两条读语句的列清单漂了");
    }

    #[test]
    fn names_are_normalized_to_the_shape_the_check_constraint_accepts() {
        assert_eq!(normalize_name("Filesystem").unwrap(), "filesystem");
        assert_eq!(normalize_name("  my-tools  ").unwrap(), "my-tools");
        assert_eq!(normalize_name("my tools").unwrap(), "my-tools");
        assert_eq!(normalize_name("a.b").unwrap(), "a-b");
    }

    /// 与前端 `MCP_NAME_PATTERN`（`ui/web/src/devices/mcpConfig.ts`）的
    /// `[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?` 等价实现。
    ///
    /// 手写而不是引入 `regex`：那条约束明写着不新增依赖，而这条正则的全部
    /// 语义就是「小写字母数字与连字符、首尾非连字符、长度 1~64」，二十来行
    /// 就能写死。**改前端正则时必须同步改这里**，两个文件里都有对照注释。
    fn frontend_pattern_accepts(s: &str) -> bool {
        if s.is_empty() || s.len() > 64 {
            return false;
        }
        if !s
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return false;
        }
        let b = s.as_bytes();
        let edge_ok = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
        edge_ok(b[0]) && edge_ok(b[b.len() - 1])
    }

    /// 归一后的**输出**必须落在前端 `MCP_NAME_PATTERN` 之内。
    ///
    /// 这不是「两边差不多就行」：前端把 `pattern` 挂在 `<input name="name">`
    /// 上，服务端把归一后的名字回填进**编辑框**。只要服务端能产出某个前端
    /// 收不下的名字，「保存 → 重新编辑」这一圈就死 —— 用户看到的是一个
    /// 没有来由的红框，而真正的原因是两个文件里的两条正则对不上。
    ///
    /// `ui/web/src/devices/mcpConfig.test.ts` 里有一份同源的表子，两边
    /// 任何一边单改都会红。
    #[test]
    fn every_normalized_name_passes_the_frontend_pattern() {
        let long = "a".repeat(64);
        for (raw, expected) in [
            ("a", "a"),
            ("Filesystem", "filesystem"),
            ("  my-tools  ", "my-tools"),
            ("my tools", "my-tools"),
            ("a.b", "a-b"),
            ("1tool", "1tool"),
            ("company_search", "company-search"),
            (long.as_str(), long.as_str()),
        ] {
            let out = normalize_name(raw).unwrap_or_else(|e| panic!("{raw:?} 应可归一：{e}"));
            assert_eq!(out, expected, "归一结果变了，前端那张表子也要跟着改");
            assert!(
                frontend_pattern_accepts(&out),
                "服务端产出 {out:?}（来自 {raw:?}），前端 pattern 收不下"
            );
        }
    }

    /// 反过来记一笔：**前端比服务端严一点是可以的，严过头不行。**
    ///
    /// 前端 `pattern` 拒掉下划线、首尾连字符，服务端其实会收（`_` 归一成
    /// `-`，首尾连字符被 trim）。这属于「提前拦下」，不构成契约破裂 ——
    /// 用户看到的是浏览器给的 pattern 提示，而不是一个说不清的失败。
    ///
    /// 真正致命的是反向：服务端产出前端收不下的名字，那才会让
    /// 「保存 → 重新编辑」死锁。上面那条测试钉的就是它。
    #[test]
    fn being_stricter_than_the_frontend_is_the_safe_direction() {
        // 服务端收下并归一；前端会提前拒。两者不冲突，但值得钉住意图，
        // 以免有人为了「让前端别拦」而放宽前端的 pattern。
        assert_eq!(normalize_name("a_b_c").unwrap(), "a-b-c");
        assert_eq!(normalize_name("-ab-").unwrap(), "ab");
    }

    #[test]
    fn an_unusable_name_is_rejected_with_a_reason_not_just_a_bool() {
        // 名字非法时要说清「哪个值、变成了什么、下一步怎么改」。
        let e = normalize_name("!!!").expect_err("全是符号应当被拒");
        assert!(e.contains("!!!"), "错误里要带上原值：{e}");
        assert!(e.contains("下一步"), "错误要给修复动作：{e}");
        assert!(normalize_name(&"x".repeat(100)).is_err(), "超长应被拒");
    }

    #[test]
    fn the_asset_hash_is_stable_across_key_ordering() {
        // env 是 map，迭代顺序不保证稳定；不排序的话同样的配置每次指纹都不同，
        // 于是「内容没变就别动 updated_at」这条优化形同虚设。
        let mut a = base("a");
        a.env = vec![("B".into(), "2".into()), ("A".into(), "1".into())];
        let mut b = base("a");
        b.env = vec![("A".into(), "1".into()), ("B".into(), "2".into())];
        assert_eq!(asset_hash(&a), asset_hash(&b));
    }

    #[test]
    fn the_asset_hash_is_the_length_the_check_constraint_demands() {
        // mcp_servers 的 CHECK 是 length(asset_hash) = 32。短一字节，
        // 每一行写入都会被数据库拒掉，而错误只说 CHECK constraint failed。
        assert_eq!(asset_hash(&base("a")).len(), 32);
    }

    #[test]
    fn a_real_content_change_moves_the_hash() {
        let a = base("a");
        let mut b = base("a");
        b.command = Some("uvx".into());
        assert_ne!(asset_hash(&a), asset_hash(&b));
    }

    #[test]
    fn env_and_headers_are_hashed_as_two_separate_sections() {
        // 同名键分处两个区段，必须算出不同指纹。合成一张 map 的话后者会
        // 覆盖前者，于是「只改了 header」会看起来没改过任何东西。
        let mut a = base("a");
        a.env = vec![("TOKEN".into(), "1".into())];
        let mut b = base("a");
        b.headers = vec![("TOKEN".into(), "1".into())];
        assert_ne!(asset_hash(&a), asset_hash(&b));
    }

    #[test]
    fn the_capability_three_states_are_three_different_fingerprints() {
        // null=全禁、[]=全开、["read"]=精确列举。少区分一个，用户把「全开」
        // 调成「全禁」就会被当成没改，配置看着保存成功其实没生效。
        let mut off = base("a");
        off.enabled_capabilities = None;
        let mut all = base("a");
        all.enabled_capabilities = Some(vec![]);
        let mut one = base("a");
        one.enabled_capabilities = Some(vec!["read".into()]);
        assert_ne!(asset_hash(&off), asset_hash(&all));
        assert_ne!(asset_hash(&all), asset_hash(&one));
    }

    #[test]
    fn field_boundaries_cannot_be_forged_by_shifting_a_delimiter() {
        // 分段是「个数 + 每项长度前缀」的：参数 [""] 与 [] 不是同一份配置。
        // 早先把参数 join 成一个字符串再哈希，两者拼出同样的字节，
        // 于是「给命令加了个空参数」看起来等于没改过 —— 这条断言就是钉它的。
        let mut a = base("a");
        a.command = Some("a".into());
        a.args = vec![];
        let mut b = base("a");
        b.command = Some("a".into());
        b.args = vec!["".into()];
        assert_ne!(asset_hash(&a), asset_hash(&b));

        // 同样地，["a\u{1}b"] 与 ["a","b"] 也不能撞。
        let mut c = base("a");
        c.args = vec!["a\u{1}b".into()];
        let mut d = base("a");
        d.args = vec!["a".into(), "b".into()];
        assert_ne!(asset_hash(&c), asset_hash(&d));
    }

    #[test]
    fn stdio_and_http_serialize_into_different_shapes() {
        // 前端按 transport 决定读 env 还是 headers、显示 command 还是 url。
        // 两边混了会让编辑框在切换传输方式后残留上一份的值。
        let stdio = to_json(&base("a"));
        assert!(stdio.get("command").is_some());
        assert!(stdio.get("url").is_none());
        assert!(stdio.get("env").is_some());

        let mut http = base("b");
        http.transport = "streamable_http".into();
        http.command = None;
        http.url = Some("https://example/mcp".into());
        let j = to_json(&http);
        assert!(j.get("url").is_some());
        assert!(j.get("headers").is_some());
        assert!(j.get("command").is_none());
    }

    #[test]
    fn a_builtin_row_serializes_with_its_name_and_without_a_url() {
        // 内置服务器的名字住在 `command` 里（`quill_core::builtin`）。不回它，
        // 界面上显示不出是哪一台，而且编辑框回填时名字就丢了 —— 下一次点保存
        // 必被 400 挡下。`url` 冒出来则是另一个方向的问题：切到 builtin 之后
        // 残留上一份地址，而内置服务器根本没有地址。
        let mut b = base("mem");
        b.transport = "builtin".into();
        b.command = Some("memory".into());
        b.url = None;
        let j = to_json(&b);
        assert_eq!(j["command"], serde_json::json!("memory"), "{j}");
        assert!(j.get("url").is_none(), "{j}");
        assert!(j.get("headers").is_none(), "{j}");
    }

    #[test]
    fn all_disabled_and_all_enabled_stay_distinguishable() {
        // null = 全禁，[] = 全开。前端据此渲染不同的选择状态。
        let mut off = base("a");
        off.enabled_capabilities = None;
        assert!(to_json(&off)["enabled_capabilities"].is_null());

        let mut on = base("a");
        on.enabled_capabilities = Some(vec![]);
        assert_eq!(to_json(&on)["enabled_capabilities"], serde_json::json!([]));

        let mut some = base("a");
        some.enabled_capabilities = Some(vec!["read".into()]);
        assert_eq!(
            to_json(&some)["enabled_capabilities"],
            serde_json::json!(["read"])
        );
    }
}
