//! 外部消息通道的落库层。
//!
//! 结构对齐 Octop 的 `channels` 表
//! （`.octop-ref/octop/src/octop/infra/db/repos/channels.py:11-20`，**只读对齐**）：
//! 一行 = 一条命名通道，按用户分组，配置整体存 `config_json`。
//!
//! 有三条要守住的规矩：
//! - **凭据永远不出现在任何响应里。** `to_public` 是唯一出口，它按白名单
//!   逐字段挑出来，绝不把 `config_json` 整个倒出去。加字段时别图省事改成
//!   `json!(row.config)` —— 那等于把长期凭据发到浏览器。
//! - **运行状态不落库。** `stopped/connecting/ready` 那套是
//!   `Zpoteiti/OpenOctopus`（另一个项目，见 `.octop-baseline-check.mjs`）
//!   的设计，Octop 自己没有，本项目也不要有：进程重启后本来就要重连，
//!   把瞬时状态写进库只会让人误以为它跨重启有效。
//! - **`sync_state_json` 与 `config_json` 分开。** 前者是机器写的
//!   （轮询游标、context_token），后者是人改的。混在一起会把用户的编辑
//!   和每 35 秒一次的轮询写入搅在一起。

use crate::db::DbBridge;
use crate::error::ApiError;
use serde_json::{json, Value};
use sqlx::Row;

/// 本实例支持的通道。**第一版只接微信** —— 用户明确说了「连接一个微信就可以」。
///
/// 加第二个通道不改这张表：表是数据驱动的（按 `kind` 分行），只要在这里
/// 加一条实现即可。Octop 支持九种，我们先走通一条。
pub const SUPPORTED: [&str; 1] = ["weixin"];

/// 私聊准入策略。四值，口径对齐 Octop
/// （`.octop-ref/octop/dashboard/src/api/types/channel.ts:60`）。
pub const DM_POLICIES: [&str; 4] = ["open", "allowlist", "pairing", "disabled"];

/// `allowlist` 的默认态。空名单 = 谁都不许发。
///
/// 刻意如此：Octop 的 wecom 用的也是这个默认值。配好通道却让陌生人直接
/// 能跟智能体说话，是更难被发现的问题（表现为「怎么有人找上门」），
/// 而「机器人不理我」用户立刻就知道。
pub const DEFAULT_DM_POLICY: &str = "allowlist";

pub fn is_supported(kind: &str) -> bool {
    SUPPORTED.contains(&kind)
}

pub fn is_valid_dm_policy(p: &str) -> bool {
    DM_POLICIES.contains(&p)
}

#[derive(Debug, Clone)]
pub struct ChannelRow {
    pub channel_id: String,
    pub owner: [u8; 16],
    pub kind: String,
    pub name: String,
    /// 人配的。里面可能有凭据，见本文件开头的规矩。
    pub config: Value,
    pub enabled: bool,
    /// 机器写的同步状态：游标与各对端的 context_token。
    pub sync_state: Value,
    pub updated_at: i64,
}

fn s(row: &sqlx::sqlite::SqliteRow, name: &str) -> String {
    row.try_get::<Option<String>, _>(name)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn n(row: &sqlx::sqlite::SqliteRow, name: &str) -> i64 {
    row.try_get::<Option<i64>, _>(name).ok().flatten().unwrap_or(0)
}

/// 读一列 JSON。列上有 `json_valid` 约束，所以解析失败只可能是程序自己
/// 写坏了 —— 退化成 `{}` 而不是报错，免得一行坏数据把整个列表页打挂。
fn j(row: &sqlx::sqlite::SqliteRow, name: &str) -> Value {
    serde_json::from_str(&s(row, name)).unwrap_or_else(|_| json!({}))
}

fn from_row(row: &sqlx::sqlite::SqliteRow) -> ChannelRow {
    let raw = row
        .try_get::<Option<Vec<u8>>, _>("owner_user_id")
        .ok()
        .flatten();
    let owner = <[u8; 16]>::try_from(raw.unwrap_or_default()).unwrap_or([0u8; 16]);
    ChannelRow {
        channel_id: s(row, "channel_id"),
        owner,
        kind: s(row, "kind"),
        name: s(row, "name"),
        config: j(row, "config_json"),
        enabled: n(row, "enabled") != 0,
        sync_state: j(row, "sync_state_json"),
        updated_at: n(row, "updated_at"),
    }
}

const COLS: &str = "channel_id, owner_user_id, kind, name, config_json, \
     enabled, sync_state_json, updated_at";

fn storage(detail: impl std::fmt::Display) -> ApiError {
    ApiError::storage_unavailable(format!("通道存储操作失败：{detail}"))
}

/// 对外形状。**唯一出口**。
///
/// 逐字段从 `config` 里挑，绝不把 `config_json` 整体倒出去。凭据
/// （`token` / `bot_token` / `client_secret` …）在这里被折成布尔
/// `configured`，值不外泄。
pub fn to_public(row: &ChannelRow) -> Value {
    let mut accounts = Vec::new();
    let mut configured = false;
    let mut dm_policy = DEFAULT_DM_POLICY;

    if row.kind == "weixin" {
        // 微信是多账号的（Octop: types/channel.ts:80-84 的 accounts 数组）。
        // 这里只回非密标识：account_id / account_name。
        if let Some(list) = row.config.get("accounts").and_then(Value::as_array) {
            for a in list {
                let has_token = a
                    .get("token")
                    .and_then(Value::as_str)
                    .map(|t| !t.is_empty())
                    .unwrap_or(false);
                if has_token {
                    configured = true;
                }
                accounts.push(json!({
                    "account_id": a.get("account_id").and_then(Value::as_str).unwrap_or(""),
                    "account_name": a.get("account_name").and_then(Value::as_str).unwrap_or(""),
                    "configured": has_token,
                }));
            }
        }
        if let Some(p) = row.config.get("dm_policy").and_then(Value::as_str) {
            dm_policy = p;
        }
    }

    json!({
        "channel_id": row.channel_id,
        "kind": row.kind,
        "name": row.name,
        "enabled": row.enabled,
        "configured": configured,
        "accounts": accounts,
        "dm_policy": dm_policy,
    })
}

pub async fn list(
    db: &DbBridge,
    owner: quill_domain::UserId,
) -> Result<Vec<ChannelRow>, ApiError> {
    let uid = owner.as_bytes().to_vec();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            let sql = format!(
                "SELECT {COLS} FROM channels WHERE owner_user_id = ? ORDER BY kind"
            );
            let rows = sqlx::query(&sql)
                .bind(&uid)
                .fetch_all(&pool)
                .await
                .map_err(|e| quill_agent::AgentError::Storage { detail: format!("list channels: {e}") })?;
            Ok::<_, quill_agent::AgentError>(rows.iter().map(from_row).collect())
        })
    })
    .map_err(storage)
}

pub async fn get(
    db: &DbBridge,
    owner: quill_domain::UserId,
    channel_id: &str,
) -> Result<ChannelRow, ApiError> {
    let uid = owner.as_bytes().to_vec();
    let cid = channel_id.to_string();
    let found = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let sql = format!(
                    "SELECT {COLS} FROM channels WHERE owner_user_id = ? AND channel_id = ?"
                );
                sqlx::query(&sql)
                    .bind(&uid)
                    .bind(&cid)
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e| {
                        quill_agent::AgentError::Storage { detail: format!("get channel {cid}: {e}") }
                    })
            })
        })
        .map_err(storage)?;

    found.map(|r| from_row(&r)).ok_or_else(|| {
        ApiError::not_found(format!("通道 {channel_id} 不存在，或不属于当前用户"))
    })
}

/// 找出这个用户该 kind 的那一条（表上有 UNIQUE(owner_user_id, kind)）。
///
/// 供写入路径在 INSERT 之前查冲突：撞约束出来的 sqlx 错误会被兜成
/// 503「存储不可用」，而同名通道是用户操作冲突，不是存储故障。
pub async fn get_by_kind(
    db: &DbBridge,
    owner: quill_domain::UserId,
    kind: &str,
) -> Result<Option<ChannelRow>, ApiError> {
    let uid = owner.as_bytes().to_vec();
    let k = kind.to_string();
    let found = db
        .call(move |pool, _rt| {
            Box::pin(async move {
                let sql = format!("SELECT {COLS} FROM channels WHERE owner_user_id = ? AND kind = ?");
                sqlx::query(&sql)
                    .bind(&uid)
                    .bind(&k)
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e| {
                        quill_agent::AgentError::Storage { detail: format!("get channel by kind {k}: {e}") }
                    })
            })
        })
        .map_err(storage)?;
    Ok(found.as_ref().map(|r| from_row(r)))
}

/// 建或更新一条通道。
///
/// 冲突键用 `channel_id`，但**不更新 owner/kind**：换主人或换类型应该
/// 删了重建，而不是原地改 —— 那样会让既有凭据挂到一个语义不同的通道上。
pub async fn save(db: &DbBridge, row: &ChannelRow, now: i64) -> Result<(), ApiError> {
    let r = row.clone();
    let cfg = serde_json::to_string(&r.config).unwrap_or_else(|_| "{}".to_string());
    let sync = serde_json::to_string(&r.sync_state).unwrap_or_else(|_| "{}".to_string());
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO channels (channel_id, owner_user_id, kind, name, config_json, \
                   enabled, sync_state_json, created_at, updated_at) \
                 VALUES (?,?,?,?,?,?,?,?,?) \
                 ON CONFLICT(channel_id) DO UPDATE SET \
                   name=excluded.name, config_json=excluded.config_json, \
                   enabled=excluded.enabled, updated_at=excluded.updated_at",
            )
            .bind(&r.channel_id)
            .bind(r.owner.to_vec())
            .bind(&r.kind)
            .bind(&r.name)
            .bind(&cfg)
            .bind(if r.enabled { 1i64 } else { 0i64 })
            .bind(&sync)
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|e| {
                quill_agent::AgentError::Storage {
                    detail: format!("save channel {}: {e}", r.channel_id),
                }
            })?;
            Ok::<(), quill_agent::AgentError>(())
        })
    })
    .map_err(storage)
}

/// 只更新同步状态。**不碰 config_json** —— 见本文件开头的第三条规矩。
pub async fn save_sync_state(
    db: &DbBridge,
    channel_id: &str,
    sync_state: &Value,
    now: i64,
) -> Result<(), ApiError> {
    let cid = channel_id.to_string();
    let st = serde_json::to_string(sync_state).unwrap_or_else(|_| "{}".to_string());
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query(
                "UPDATE channels SET sync_state_json = ?, updated_at = ? WHERE channel_id = ?",
            )
            .bind(&st)
            .bind(now)
            .bind(&cid)
            .execute(&pool)
            .await
            .map_err(|e| {
                quill_agent::AgentError::Storage { detail: format!("save sync state {cid}: {e}") }
            })?;
            Ok::<(), quill_agent::AgentError>(())
        })
    })
    .map_err(storage)
}

pub async fn remove(
    db: &DbBridge,
    owner: quill_domain::UserId,
    channel_id: &str,
) -> Result<(), ApiError> {
    let uid = owner.as_bytes().to_vec();
    let cid = channel_id.to_string();
    db.call(move |pool, _rt| {
        Box::pin(async move {
            sqlx::query("DELETE FROM channels WHERE owner_user_id = ? AND channel_id = ?")
                .bind(&uid)
                .bind(&cid)
                .execute(&pool)
                .await
                .map_err(|e| {
                    quill_agent::AgentError::Storage { detail: format!("delete channel {cid}: {e}") }
                })?;
            Ok::<(), quill_agent::AgentError>(())
        })
    })
    .map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_weixin() -> ChannelRow {
        ChannelRow {
            channel_id: "wx-main".into(),
            owner: [1u8; 16],
            kind: "weixin".into(),
            name: "我的微信".into(),
            config: json!({
                "dm_policy": "allowlist",
                "accounts": [{
                    "account_id": "acc-1@im.bot",
                    "account_name": "小王",
                    "token": "SECRET-TOKEN",
                    "base_url": "https://ilinkai.weixin.qq.com",
                }],
            }),
            enabled: true,
            sync_state: json!({}),
            updated_at: 0,
        }
    }

    #[test]
    fn public_view_never_leaks_the_token() {
        let out = to_public(&row_weixin()).to_string();
        assert!(!out.contains("SECRET-TOKEN"), "token leaked: {out}");
        assert!(out.contains("acc-1@im.bot"), "非密标识要能回显: {out}");
    }

    #[test]
    fn public_view_does_not_dump_config_json() {
        // 整列倒出去是最容易犯的错：config 将来可能加别的敏感字段
        let out = to_public(&row_weixin());
        let obj = out.as_object().unwrap();
        assert!(!obj.contains_key("config"));
        assert!(!obj.contains_key("config_json"));
        assert_eq!(obj["dm_policy"], "allowlist");
        let accs = obj["accounts"].as_array().unwrap();
        assert_eq!(accs[0]["configured"], true);
        assert!(!accs[0].as_object().unwrap().contains_key("token"));
    }

    #[test]
    fn unconfigured_account_is_reported_as_not_configured() {
        let mut row = row_weixin();
        row.config["accounts"][0]["token"] = json!("");
        let out = to_public(&row);
        assert_eq!(out["configured"], false);
        assert_eq!(out["accounts"][0]["configured"], false);
    }

    #[test]
    fn default_policy_is_allowlist_not_open() {
        let row = ChannelRow {
            config: json!({ "accounts": [] }),
            ..row_weixin()
        };
        // 没写 dm_policy 时不能默认放行任何人
        assert_eq!(to_public(&row)["dm_policy"], "allowlist");
    }

    #[test]
    fn policy_vocabulary_matches_octop() {
        // 对齐 Octop types/channel.ts:60 的 dm_policy 四值
        assert_eq!(
            DM_POLICIES.to_vec(),
            vec!["open", "allowlist", "pairing", "disabled"]
        );
        assert!(is_valid_dm_policy("open"));
        assert!(!is_valid_dm_policy("everything"));
    }
}