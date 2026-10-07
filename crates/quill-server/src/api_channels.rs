//! 通道的 HTTP 接线与长轮询后台任务。
//!
//! 路由形态对齐 Octop
//! （`.octop-ref/octop/src/octop/api/routers/channels.py:118-241`，**只读对齐**）：
//! 列表 / 建 / 读 / 改 / 删 + 各平台自己的 `qrcode/generate` 与 `qrcode/poll`。
//!
//! ## 长轮询怎么把消息喂给 agent
//!
//! 走的是 `api_chat::prepare_turn` + `run_turn` —— 与网页聊天**同一份工具循环**。
//! 不另写一套，因为两套循环迟早只改一边，而「网页还能用、通道坏了」是最难
//! 发现的错法。
//!
//! iLink 不能改已发消息（协议限制），所以回复**不等这一轮跑完就发**：
//! 整轮跑完再一次性发出去。这是协议决定的，不是偷懒。
//!
//! ## 每个微信用户一条会话
//!
//! `channels` 表里没有会话 id —— 那属于运行状态，会随重连失效。
//! 映射规则写在 [`session_for_peer`]：一个 `from_user_id` 对应一条会话，
//! 建出来后把 id 存进该通道的 `sync_state_json`。
//! 换账号（重新扫码）后 `sync_state` 里的旧会话 id 必须作废 —— 那是另一个
//! 身份的消息历史，混在一起等于把别人的对话喂给新身份。

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::channels::store::{self, ChannelRow};
use crate::channels::weixin::{self, QrStatus, Session};
use crate::error::ApiError;
use crate::state::AppState;

// ------------------------------------------------------------------ REST 线

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let rows = store::list(state.db()?, user.0.user_id).await?;
    Ok(Json(json!({
        "channels": rows.iter().map(store::to_public).collect::<Vec<Value>>(),
        // 已支持的种类。前端据此决定画几张卡，而不是自己写死。
        "supported": store::SUPPORTED,
    })))
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let row = store::get(state.db()?, user.0.user_id, &id).await?;
    Ok(Json(store::to_public(&row)))
}

/// 建或改一条通道。
///
/// 配置里的凭据**只进不出**：请求可以带 token，响应里没有。
pub async fn upsert(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    crate::api_experts::only_keys(
        &body,
        &["kind", "name", "config", "enabled", "channel_id"],
        "POST /api/channels",
    )?;

    let kind = req_str(&body, "kind")?;
    if !store::is_supported(&kind) {
        return Err(ApiError::bad_request(format!(
            "不支持的通道类型 {kind}。本实例支持：{}。",
            store::SUPPORTED.join("、")
        )));
    }
    let name = opt_str(&body, "name")?.unwrap_or_else(|| default_name(&kind));
    let enabled = opt_bool(&body, "enabled")?.unwrap_or(false);

    // 通道 id：客户端可以指定（改既有那条），不指定就按类型生成。
    let channel_id = match opt_str(&body, "channel_id")? {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => format!("{kind}-main"),
    };
    validate_channel_id(&channel_id)?;

    let db = state.db()?;
    // 已存在就保留没传的字段 —— 否则「只改白名单」会把凭据抹掉。
    let existing = store::get(db, user.0.user_id, &channel_id).await.ok();

    // 同 kind 只能有一条（表上有 UNIQUE(owner_user_id, kind)）。
    // **在这里查而不是等 INSERT 撞约束**：撞出来的 sqlx 错误会被 store 的
    // map_err 兜成 503「存储不可用」，而这是用户操作冲突，不是存储故障 ——
    // 报 503 会让人去查数据库，而真因是他刚建了第二条同名通道。
    if let Some(other) = store::get_by_kind(db, user.0.user_id, &kind).await? {
        if other.channel_id != channel_id {
            return Err(ApiError::conflict(
                format!(
                    "已经有一条 {kind} 通道「{}」（id {}）了。",
                    other.name, other.channel_id
                ),
                "改那条既有的，或先 DELETE /api/channels/{id} 删掉它。",
            ));
        }
    }

    let mut config = match (opt_value(&body, "config"), existing.as_ref()) {
        (Some(c), _) => c,
        (None, Some(e)) => e.config.clone(),
        (None, None) => json!({}),
    };
    validate_config(&kind, &mut config)?;

    let now = now_ms();
    let row = ChannelRow {
        channel_id,
        owner: *user.0.user_id.as_bytes(),
        kind,
        name,
        config,
        enabled,
        // 同步状态**不因配置改动而清空**：白名单改了不该丢游标，
        // 否则服务端会重放整个历史。
        sync_state: existing.map(|e| e.sync_state).unwrap_or_else(|| json!({})),
        updated_at: now,
    };
    store::save(db, &row, now).await?;

    if row.enabled && row.kind == "weixin" {
        spawn_poller(state.clone(), row.clone());
    }
    Ok(Json(store::to_public(&row)))
}

pub async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<axum::response::Response, ApiError> {
    store::remove(state.db()?, user.0.user_id, &id).await?;
    Ok(Json(json!({ "deleted": id })).into_response())
}

// ------------------------------------------------------------ 微信扫码三步

/// 第一步：要一张二维码。
pub async fn weixin_qr_generate(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Value>, ApiError> {
    let now = now_ms();
    // base_url 从哪来：优先已配的那条通道（可能服务端返回过专属地址），
    // 否则用默认。Octop 同理（mcp-server 的 baseUrl 解析链）。
    let base = store::get(state.db()?, user.0.user_id, "weixin-main")
        .await
        .ok()
        .and_then(|r| weixin_base_of(&r))
        .unwrap_or_else(|| weixin::DEFAULT_BASE_URL.to_string());

    let t = weixin::fetch_qrcode(&base, now).await.map_err(ilink_err)?;
    Ok(Json(json!({
        "qrcode_token": t.qrcode,
        "qrcode_url": t.image,
        "expires_at": t.expires_at,
    })))
}

/// 第二步：轮询扫码状态。
///
/// 未扫 → `wait`；已扫待确认 → `scaned`；确认 → 把凭据写进通道并返回。
/// 约定与 Octop 一致（qr_bind.py:117-136 的返回形状）。
pub async fn weixin_qr_poll(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    crate::api_experts::only_keys(
        &body,
        &["qrcode_token", "channel_id"],
        "POST /api/channels/weixin/qrcode/poll",
    )?;
    let token = req_str(&body, "qrcode_token")?;
    let channel_id = opt_str(&body, "channel_id")?.unwrap_or_else(|| "weixin-main".into());
    validate_channel_id(&channel_id)?;

    let db = state.db()?;
    let existing = store::get(db, user.0.user_id, &channel_id).await.ok();
    let base = existing
        .as_ref()
        .and_then(weixin_base_of)
        .unwrap_or_else(|| weixin::DEFAULT_BASE_URL.to_string());

    let st = weixin::poll_qr_status(&base, &token).await.map_err(ilink_err)?;
    match st {
        QrStatus::Waiting => Ok(Json(json!({ "status": "wait" }))),
        QrStatus::Scanned => Ok(Json(json!({ "status": "scaned" }))),
        QrStatus::Expired => Ok(Json(json!({
            "status": "expired",
            "detail": "二维码已过期（约 5 分钟有效），请重新获取一张。",
        }))),
        QrStatus::Unknown(s) => Ok(Json(json!({
            "status": "wait",
            "detail": format!("扫码服务返回了未预期的状态 {s}，继续等待。"),
        }))),
        QrStatus::Confirmed(sess) => {
            let row = persist_session(db, user.0.user_id, &channel_id, &sess, existing).await?;
            spawn_poller(state.clone(), row.clone());
            Ok(Json(json!({
                "status": "success",
                "channel": store::to_public(&row),
            })))
        }
    }
}

/// 把扫码确认下来的凭据写进通道，并**作废旧的同步状态**。
///
/// 作废是必须的：旧 `sync_state` 里的会话 id 与 context_token 属于上一个身份，
/// 留着等于把旧身份的消息历史接到新身份上。
async fn persist_session(
    db: &crate::db::DbBridge,
    uid: quill_domain::UserId,
    channel_id: &str,
    sess: &Session,
    existing: Option<ChannelRow>,
) -> Result<ChannelRow, ApiError> {
    let now = now_ms();
    let mut config = existing
        .as_ref()
        .map(|r| r.config.clone())
        .unwrap_or_else(|| json!({}));

    let account = json!({
        "account_id": sess.bot_id,
        "account_name": sess.bot_id,
        "base_url": sess.base_url,
        "token": sess.token,
    });
    // 同 account_id 就替换，否则追加 —— 扫码两次同一个号不该出两条。
    let mut accounts = config
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match accounts
        .iter()
        .position(|a| a.get("account_id").and_then(Value::as_str) == Some(sess.bot_id.as_str()))
    {
        Some(i) => accounts[i] = account,
        None => accounts.push(account),
    }
    config["accounts"] = Value::Array(accounts);
    if !config.get("dm_policy").is_some() {
        config["dm_policy"] = json!(store::DEFAULT_DM_POLICY);
    }

    let row = ChannelRow {
        channel_id: channel_id.to_string(),
        owner: *uid.as_bytes(),
        kind: "weixin".into(),
        name: existing
            .as_ref()
            .map(|r| r.name.clone())
            .unwrap_or_else(|| default_name("weixin")),
        config,
        enabled: existing.as_ref().map(|r| r.enabled).unwrap_or(true),
        // 换身份 -> 旧的会话映射与游标全部作废。
        sync_state: json!({}),
        updated_at: now,
    };
    store::save(db, &row, now).await?;
    Ok(row)
}

/// 从通道配置里取 base_url（第一个已配账号的）。
fn weixin_base_of(row: &ChannelRow) -> Option<String> {
    row.config
        .get("accounts")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|a| {
            a.get("base_url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        })
}

// ------------------------------------------------------------ 长轮询后台任务

/// 把一条已启用的微信通道挂上长轮询。
///
/// **不重复起任务**：同一个 `channel_id` 已在轮询就直接返回。否则每次保存
/// 配置都会多一个轮询者，而 iLink 明确要求一个凭据只能有一个长轮询 ——
/// 两个轮询者会互相偷消息。
fn spawn_poller(state: AppState, row: ChannelRow) {
    let key = row.channel_id.clone();
    if !ACTIVE.swap_insert(key.clone()) {
        return; // 已在轮询
    }
    tokio::spawn(async move {
        poll_loop(state, row).await;
        ACTIVE.swap_remove(&key);
    });
}

/// 正在轮询的通道集合。
///
/// 只在**进任务前 / 出任务后**各锁一次，中间不跨 `.await` 持锁，
/// 所以 std 锁够用。`LazyLock` 是因为 `HashSet::new()` 不是 const，
/// 而 static 的初始化只允许 const 调用。
struct ActiveChannels(std::sync::Mutex<std::collections::HashSet<String>>);

impl ActiveChannels {
    fn swap_insert(&self, key: String) -> bool {
        match self.0.lock() {
            Ok(mut g) => g.insert(key),
            Err(_) => false,
        }
    }
    fn swap_remove(&self, key: &str) -> bool {
        match self.0.lock() {
            Ok(mut g) => g.remove(key),
            Err(_) => false,
        }
    }
}

static ACTIVE: std::sync::LazyLock<ActiveChannels> =
    std::sync::LazyLock::new(|| ActiveChannels(std::sync::Mutex::new(std::collections::HashSet::new())));

/// 长轮询主循环。**不自己重试** —— 退避重连是下一轮的事（见 BACKLOG B6-5）。
///
/// 每轮：拉消息 → 逐条跑一轮 agent → 发回微信 → 存游标。
/// 游标**每轮都存**，即使这轮没有消息（服务端可能已经推进了游标）。
async fn poll_loop(state: AppState, row: ChannelRow) {
    let Some(token) = first_token(&row) else {
        eprintln!("[channel] {} 没配凭据，不启动轮询", row.channel_id);
        return;
    };
    let base = weixin_base_of(&row).unwrap_or_else(|| weixin::DEFAULT_BASE_URL.to_string());
    let session = Session {
        token,
        bot_id: String::new(),
        user_id: String::new(),
        base_url: base,
        client: match reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[channel] {} 建 HTTP 客户端失败：{e}", row.channel_id);
                return;
            }
        },
    };

    let db = match state.db() {
        Ok(d) => d.clone(),
        Err(e) => {
            eprintln!("[channel] {} 拿不到存储：{e}", row.channel_id);
            return;
        }
    };
    let mut cursor = sync_cursor(&row);
    // 整个循环共用**一份**内存态，每处写入都基于它更新，循环末尾统一落库一次。
    //
    // 原来每处写入都从 `row.sync_state`（启动快照）出发，循环末尾又单独写一份
    // 只有游标的 JSON —— 于是会话映射与对端 context_token 会被那一写整个抹掉，
    // 而且每轮都抹一次。现象是「机器人第二天就不记得自己说过什么」。
    let mut sync = row.sync_state.clone();
    if !sync.is_object() {
        sync = json!({});
    }

    loop {
        match weixin::poll_updates(&session, &cursor).await {
            Ok(batch) => {
                cursor = batch.cursor.clone();
                for msg in &batch.messages {
                    if !peer_allowed(&row, &msg.from_user_id) {
                        continue;
                    }
                    if msg.context_token.is_empty() {
                        continue; // 没有它就回复不出去，别白跑一轮模型
                    }
                    match handle_turn(&state, &db, &row, &mut sync, msg).await {
                        Ok(reply) => {
                            for chunk in weixin::split_for_wechat(&reply) {
                                if let Err(e) = weixin::send_text(
                                    &session,
                                    &msg.from_user_id,
                                    &msg.context_token,
                                    &chunk,
                                )
                                .await
                                {
                                    eprintln!(
                                        "[channel] {} 回复失败（{}）：{e}",
                                        row.channel_id, msg.from_user_id
                                    );
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "[channel] {} 跑一轮失败（{}）：{e}",
                                row.channel_id, msg.from_user_id
                            );
                        }
                    }
                }
                // 游标每轮都落库：服务端可能已经推进过，即使这轮没消息。
                sync["cursor"] = json!(cursor);
                if let Err(e) = store::save_sync_state(&db, &row.channel_id, &sync, now_ms()).await
                {
                    eprintln!("[channel] {} 存游标失败：{e}", row.channel_id);
                }
            }
            Err(e) if e.is_session_expired() => {
                eprintln!(
                    "[channel] {} 登录已过期（iLink errcode -14），停止轮询，需要重新扫码",
                    row.channel_id
                );
                return;
            }
            Err(e) => {
                eprintln!("[channel] {} 拉取失败：{e}", row.channel_id);
                return; // 退避重连留给下一轮，先别把一个坏轮询挂死
            }
        }
    }
}

/// 收到一条消息 → 找到或建会话 → 跑一轮 → 返回要发回微信的文本。
async fn handle_turn(
    state: &AppState,
    db: &crate::db::DbBridge,
    row: &ChannelRow,
    sync: &mut Value,
    msg: &weixin::Inbound,
) -> Result<String, ApiError> {
    let sid = session_for_peer(db, row, sync, &msg.from_user_id).await?;
    // 对端的 context_token：回复时必须原样带回，否则消息挂不上会话。
    put_at(sync, &["ctx", &msg.from_user_id], json!(msg.context_token));
    // 通道来件没有登录用户，但 prepare_turn 要一个 AuthUser 拿 user_id ——
    // 那个 id 来自通道自己的 owner，与登录态无关，所以这样构造是安全的：
    // 它拿不到任何登录用户的资源，只能碰 row.owner 名下的东西。
    let user = crate::auth::AuthUser(crate::auth::AuthContext {
        user_id: quill_domain::UserId::from_bytes(row.owner),
        is_admin: false,
    });

    let mut prep = crate::api_chat::prepare_turn(
        state.clone(),
        user,
        crate::api_chat::session_hex(&sid),
        msg.text.clone(),
    )
    .await?;
    let mut sink = crate::api_chat::NullSink;
    let outcome = crate::api_chat::run_turn(&mut prep, crate::api_chat::ReplyMode::Once, &mut sink).await?;
    let done = crate::api_chat::finish_turn(&prep, outcome).await?;

    Ok(done
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string())
}

/// `from_user_id` → 会话 id。没有就建一条**并记进 `sync`**。
///
/// 记进去是必须的：不记的话每条消息都会新建一条会话，用户在网页侧栏里
/// 看到的就是一长串同名会话。
///
/// 会话标题用对端 id 的一小截：用户能在网页侧栏里认出这条会话。
async fn session_for_peer(
    db: &crate::db::DbBridge,
    row: &ChannelRow,
    sync: &mut Value,
    peer: &str,
) -> Result<[u8; 16], ApiError> {
    if let Some(existing) = peer_session(sync, peer) {
        match crate::api_chat::parse_public_id(&existing) {
            Ok(sid) => {
                // 会话可能已被用户删掉；查一下再用，别对着空气跑一轮。
                if crate::api_chat::session_exists(db, quill_domain::UserId::from_bytes(row.owner), sid).await? {
                    return Ok(sid);
                }
            }
            Err(_) => {
                // 记着个非法 id —— 换身份后残留的老数据。丢掉重建。
                eprintln!(
                    "[channel] {} 的会话映射里有非法 id，按新建处理",
                    row.channel_id
                );
            }
        }
    }
    let sid = crate::api_chat::create_session_for_channel(
        db,
        quill_domain::UserId::from_bytes(row.owner),
        &format!("微信 · {}", &peer[..peer.len().min(12)]),
    )
    .await?;
    put_at(sync, &["sessions", peer], json!(crate::api_chat::session_hex(&sid)));
    Ok(sid)
}

/// 沿路径写进一个 JSON 对象，中间缺哪层建哪层。
///
/// 存在的理由：serde_json 的 `v["a"]["b"] = x` 在 `v["a"]` 不存在时
/// **直接 panic**（`IndexMut` 遇到 Null 会炸），而这里写的路径头一段
/// （`sessions` / `ctx`）在首条消息之前必然不存在。
fn put_at(root: &mut Value, path: &[&str], value: Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut cur = root;
    for key in parents {
        if !cur.get(*key).map(Value::is_object).unwrap_or(false) {
            cur[*key] = json!({});
        }
        cur = cur.get_mut(*key).expect("刚建的对象必然存在");
    }
    cur[*last] = value;
}

// ------------------------------------------------------------------ 小工具

/// 私聊准入。口径是 Octop 的 `dm_policy`（types/channel.ts:60）。
fn peer_allowed(row: &ChannelRow, peer: &str) -> bool {
    let policy = row
        .config
        .get("dm_policy")
        .and_then(Value::as_str)
        .unwrap_or(store::DEFAULT_DM_POLICY);
    match policy {
        "open" => true,
        "disabled" => false,
        // `pairing` 按「先配对才放行」处理：白名单里有人才算配对过。
        // 没实现真正的配对流程之前，它等价于 allowlist —— 但**不放行**
        // 陌生人，所以这是安全的一侧。
        "allowlist" | "pairing" => row
            .config
            .get("allow_from")
            .and_then(Value::as_array)
            .map(|ids| ids.iter().any(|x| x.as_str() == Some(peer)))
            .unwrap_or(false),
        _ => false,
    }
}

fn first_token(row: &ChannelRow) -> Option<String> {
    row.config
        .get("accounts")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|a| {
            a.get("token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
        })
}

fn sync_cursor(row: &ChannelRow) -> String {
    row.sync_state
        .get("cursor")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn peer_session(sync: &Value, peer: &str) -> Option<String> {
    sync.get("sessions")
        .and_then(|m| m.get(peer))
        .and_then(Value::as_str)
        .map(String::from)
}

/// 配置校验。**只校验真的有意义的字段** ——
/// 不给还没实现的字段留位置，那等于画一个存了就没人读的坑。
fn validate_config(kind: &str, config: &mut Value) -> Result<(), ApiError> {
    if kind != "weixin" {
        return Ok(());
    }
    if !config.is_object() {
        *config = json!({});
    }
    if let Some(p) = config.get("dm_policy").and_then(Value::as_str) {
        if !store::is_valid_dm_policy(p) {
            return Err(ApiError::bad_request(format!(
                "dm_policy 只能是 {}。收到 {p}。",
                store::DM_POLICIES.join(" / ")
            )));
        }
    }
    if let Some(list) = config.get("accounts") {
        if !list.is_array() {
            return Err(ApiError::bad_request("accounts 必须是数组。"));
        }
        for a in list.as_array().unwrap() {
            if !a.is_object() {
                return Err(ApiError::bad_request("accounts 每一项必须是对象。"));
            }
            if let Some(t) = a.get("token").and_then(Value::as_str) {
                if t.len() > 4096 {
                    return Err(ApiError::bad_request("accounts[].token 过长。"));
                }
            }
        }
    }
    Ok(())
}

/// 通道 id 会进路由，必须是不含路径分隔符与空白的一段。
fn validate_channel_id(id: &str) -> Result<(), ApiError> {
    if id.is_empty() || id.len() > 64 {
        return Err(ApiError::bad_request("channel_id 长度必须在 1 到 64 之间。"));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request(
            "channel_id 只能包含字母、数字、连字符和下划线。",
        ));
    }
    Ok(())
}

fn default_name(kind: &str) -> String {
    match kind {
        "weixin" => "我的微信".to_string(),
        _ => kind.to_string(),
    }
}

fn req_str(body: &Value, key: &str) -> Result<String, ApiError> {
    match body.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        Some(Value::String(_)) => Err(ApiError::bad_request(format!("{key} 不能是空串。"))),
        Some(other) => Err(ApiError::bad_request(format!(
            "{key} 必须是字符串，收到 {}。",
            match other {
                Value::Null => "null",
                Value::Bool(_) => "布尔",
                Value::Number(_) => "数字",
                Value::Array(_) => "数组",
                _ => "对象",
            }
        ))),
        None => Err(ApiError::bad_request(format!("缺少必填字段 {key}。"))),
    }
}

fn opt_str(body: &Value, key: &str) -> Result<Option<String>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ApiError::bad_request(format!("{key} 必须是字符串。"))),
    }
}

fn opt_bool(body: &Value, key: &str) -> Result<Option<bool>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(ApiError::bad_request(format!("{key} 必须是布尔值。"))),
    }
}

fn opt_value(body: &Value, key: &str) -> Option<Value> {
    match body.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.clone()),
    }
}

fn ilink_err(e: weixin::ILinkError) -> ApiError {
    ApiError::service_unavailable(format!(
        "微信通道调用失败：{e}\n下一步：检查本机能否访问 ilinkai.weixin.qq.com；\
         若提示会话过期，在通道页重新扫码。"
    ))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(policy: &str, allow: &[&str]) -> ChannelRow {
        let mut config = json!({
            "dm_policy": policy,
            "accounts": [{ "account_id": "a@im.bot", "token": "T" }],
        });
        config["allow_from"] = json!(allow);
        ChannelRow {
            channel_id: "weixin-main".into(),
            owner: [7u8; 16],
            kind: "weixin".into(),
            name: "wx".into(),
            config,
            enabled: true,
            sync_state: json!({}),
            updated_at: 0,
        }
    }

    #[test]
    fn open_policy_lets_anyone_in() {
        assert!(peer_allowed(&row("open", &[]), "anyone"));
    }

    #[test]
    fn allowlist_denies_strangers() {
        let r = row("allowlist", &["friend"]);
        assert!(peer_allowed(&r, "friend"));
        assert!(!peer_allowed(&r, "stranger"), "名单外的人必须被挡住");
    }

    #[test]
    fn empty_allowlist_authorises_nobody() {
        // 这是默认态：配好通道 != 让陌生人能对话
        assert!(!peer_allowed(&row("allowlist", &[]), "anyone"));
    }

    #[test]
    fn disabled_denies_even_listed_users() {
        let r = row("disabled", &["friend"]);
        assert!(!peer_allowed(&r, "friend"));
    }

    #[test]
    fn pairing_does_not_open_the_door() {
        // 没实现真配对前，pairing 至少要是安全的一侧
        let r = row("pairing", &[]);
        assert!(!peer_allowed(&r, "anyone"));
    }

    #[test]
    fn unknown_policy_fails_closed() {
        // 认不出来的策略必须挡住，不能当成 open
        assert!(!peer_allowed(&row("garbage", &["x"]), "x"));
    }

    #[test]
    fn missing_policy_defaults_to_allowlist_not_open() {
        let mut r = row("open", &[]);
        r.config.as_object_mut().unwrap().remove("dm_policy");
        assert!(!peer_allowed(&r, "anyone"));
    }

    #[test]
    fn channel_id_rejects_path_separators() {
        assert!(validate_channel_id("weixin-main").is_ok());
        assert!(validate_channel_id("a_b-1").is_ok());
        assert!(validate_channel_id("../etc").is_err());
        assert!(validate_channel_id("a/b").is_err());
        assert!(validate_channel_id("").is_err());
    }

    #[test]
    fn config_rejects_unknown_policy() {
        let mut c = json!({ "dm_policy": "wat" });
        assert!(validate_config("weixin", &mut c).is_err());
        let mut c2 = json!({ "dm_policy": "open" });
        assert!(validate_config("weixin", &mut c2).is_ok());
    }

    #[test]
    fn config_rejects_non_array_accounts() {
        let mut c = json!({ "accounts": "nope" });
        assert!(validate_config("weixin", &mut c).is_err());
    }

    #[test]
    fn cursor_defaults_to_empty_string() {
        let r = row("open", &[]);
        assert_eq!(sync_cursor(&r), "");
    }

    #[test]
    fn put_at_creates_missing_levels() {
        // 这条防的是 panic：serde_json 的 `v["a"]["b"] = x` 在 `v["a"]`
        // 不存在时直接炸，而 `sessions` / `ctx` 在首条消息之前必然不存在。
        let mut sync = json!({});
        put_at(&mut sync, &["sessions", "u1@im.wechat"], json!("AABB"));
        put_at(&mut sync, &["ctx", "u1@im.wechat"], json!("CT-1"));
        assert_eq!(sync["sessions"]["u1@im.wechat"], "AABB");
        assert_eq!(sync["ctx"]["u1@im.wechat"], "CT-1");
    }

    #[test]
    fn put_at_overwrites_existing_keys() {
        let mut sync = json!({ "sessions": { "u1": "OLD" } });
        put_at(&mut sync, &["sessions", "u1"], json!("NEW"));
        assert_eq!(sync["sessions"]["u1"], "NEW");
    }

    #[test]
    fn cursor_advance_keeps_mappings() {
        // 回归：轮询循环曾每轮末尾写一份「只有游标」的 JSON，
        // 把会话映射与 context_token 抹掉 —— 现象是机器人隔天失忆。
        let mut sync = json!({});
        put_at(&mut sync, &["sessions", "u1"], json!("AABB"));
        put_at(&mut sync, &["ctx", "u1"], json!("CT-1"));
        sync["cursor"] = json!("buf-1");
        sync["cursor"] = json!("buf-2");
        assert_eq!(sync["cursor"], "buf-2");
        assert_eq!(sync["sessions"]["u1"], "AABB");
        assert_eq!(sync["ctx"]["u1"], "CT-1");
    }
}