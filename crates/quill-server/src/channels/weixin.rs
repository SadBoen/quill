//! 微信 iLink Bot 协议客户端。
//!
//! **这不是移植，是照公开协议实现**。Octop（`github.com/TencentCloud/Octop`）
//! 的微信绑定也是走同一套 iLink 接口，它自己调的是
//! `octop_gateway.channels.weixin.login_qr` 那个外部包
//! （`.octop-ref/octop/src/octop/infra/gateway/channels/qr_bind.py:102-114`），
//! 所以接口口径一致是两边都对，而不是抄来的。
//!
//! 已核的协议事实：
//! - 基址 `https://ilinkai.weixin.qq.com`，端点前缀 `/ilink/bot/`
//! - `GET get_bot_qrcode?bot_type=3` → `{qrcode, qrcode_img_content}`
//! - `GET get_qrcode_status?qrcode=` → `wait` / `scaned` / `confirmed` / `expired`；
//!   `confirmed` 时带 `bot_token` / `ilink_bot_id` / `baseurl`
//! - `POST getupdates` 长轮询约 35 秒，`get_updates_buf` 是游标，
//!   **下一轮必须原样回传**，否则服务端重放旧消息
//! - `POST sendmessage` **必须带 `context_token`**，否则回复挂不上会话
//! - `errcode -14` 是登录过期，要重新扫码
//!
//! 协议本身带来的三条限制，实现里如实体现而不是假装没有：只能私聊
//! （iLink bot 身份收不到群事件）、不能改/删已发消息（所以回复要等一轮跑完
//! 再一次性发）、一个凭据只能有一个长轮询者。

use std::time::Duration;

use serde_json::{json, Value};

pub const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
/// `bot_type=3` 是各实现一致使用的硬编码值，含义未公开文档化。
pub const BOT_TYPE: &str = "3";
/// 长轮询的服务端挂起时长。超时**不是错误**，客户端应立即再发一轮。
pub const POLL_TIMEOUT: Duration = Duration::from_secs(35);

#[derive(Debug)]
pub enum ILinkError {
    /// HTTP 层失败（连不上、超时、非 2xx）。
    Http(String),
    /// 服务端回了非零 `ret` / `errcode`。
    Remote { code: i64, message: String },
    /// 响应体形状不对 —— 协议变了，或返回的不是 JSON。
    Shape(String),
}

impl std::fmt::Display for ILinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(m) => write!(f, "iLink 请求失败：{m}"),
            Self::Remote { code, message } => write!(f, "iLink 返回错误 {code}：{message}"),
            Self::Shape(m) => write!(f, "iLink 响应结构不符合预期：{m}"),
        }
    }
}

impl ILinkError {
    /// `errcode -14` = 会话过期，必须重新扫码。这是唯一一个「错误码有
    /// 明确处置动作」的码，所以单独认出来。
    pub fn is_session_expired(&self) -> bool {
        matches!(self, Self::Remote { code: -14, .. })
    }
}

fn http() -> Result<reqwest::Client, ILinkError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ILinkError::Http(e.to_string()))
}

/// 公共请求头。`AuthorizationType: ilink_bot_token` 与 `X-WECHAT-UIN`
/// 是协议要求的，后者每次请求都要换新值（防重放）。
fn auth_headers(
    mut rb: reqwest::RequestBuilder,
    token: &str,
) -> reqwest::RequestBuilder {
    rb = rb
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-WECHAT-UIN", wechat_uin());
    rb
}

/// `X-WECHAT-UIN`：随机 uint32 → 十进制串 → base64。
///
/// 这个值只用于防重放，服务端不校验它的语义。用 `getrandom` 而不是
/// 时间戳：时间戳在同一秒内会重复。
fn wechat_uin() -> String {
    let mut raw = [0u8; 4];
    if getrandom::fill(&mut raw).is_err() {
        raw = [0x9e; 4];
    }
    let n = u32::from_le_bytes(raw);
    b64(&n.to_string())
}

/// 标准 base64。协议要的是标准字母表 + padding，不是 URL-safe。
fn b64(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = s.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn base(base_url: &str, path: &str) -> String {
    format!("{}/ilink/bot/{}", base_url.trim_end_matches('/'), path)
}

/// 读 `ret` / `errcode` / `errmsg` 三个字段。两者都可能缺席（成功时是
/// `null` 而不是 0），所以都要按 Option 读。
fn check(v: &Value) -> Result<(), ILinkError> {
    let code = v
        .get("ret")
        .and_then(Value::as_i64)
        .or_else(|| v.get("errcode").and_then(Value::as_i64))
        .unwrap_or(0);
    if code != 0 {
        let message = v
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or("未提供 errmsg")
            .to_string();
        return Err(ILinkError::Remote { code, message });
    }
    Ok(())
}

/// 一个已登录的通道会话。
///
/// 手动实现 `Clone`/`Debug` 而不 derive：
/// - `reqwest::Client` 不 Clone，但它内部是 `Arc`，克隆共享同一个连接池 ——
///   derive 不出来，得手写；
/// - `Debug` **不打印 `token`**。它是长期凭据，任何一次
///   `println!("{session:?}")` 都会把它写进日志。
pub struct Session {
    pub token: String,
    pub bot_id: String,
    pub user_id: String,
    pub base_url: String,
    pub client: reqwest::Client,
}

impl Clone for Session {
    fn clone(&self) -> Self {
        Self {
            token: self.token.clone(),
            bot_id: self.bot_id.clone(),
            user_id: self.user_id.clone(),
            base_url: self.base_url.clone(),
            client: self.client.clone(),
        }
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("bot_id", &self.bot_id)
            .field("user_id", &self.user_id)
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Session {
    fn base(&self) -> String {
        if self.base_url.trim().is_empty() {
            DEFAULT_BASE_URL.to_string()
        } else {
            self.base_url.trim_end_matches('/').to_string()
        }
    }
}

// ---------------------------------------------------------------- 二维码登录

/// 扫码登录的第一步：取一张二维码。
///
/// 返回的 `qrcode` 是后续轮询用的会话标识（**不是**二维码内容），
/// `image` 是给用户看的那张图。
#[derive(Debug, Clone)]
pub struct QrTicket {
    pub qrcode: String,
    pub image: String,
    /// 建议的过期时间（秒级时间戳）。协议说约 5 分钟。
    pub expires_at: i64,
}

/// `GET get_bot_qrcode?bot_type=3`
pub async fn fetch_qrcode(
    base_url: &str,
    now: i64,
) -> Result<QrTicket, ILinkError> {
    let v = http()?
        .get(base(base_url, "get_bot_qrcode"))
        .query(&[("bot_type", BOT_TYPE)])
        .send()
        .await
        .map_err(|e| ILinkError::Http(e.to_string()))?
        .json::<Value>()
        .await
        .map_err(|e| ILinkError::Shape(format!("二维码响应不是 JSON：{e}")))?;
    check(&v)?;
    let qrcode = v
        .get("qrcode")
        .and_then(Value::as_str)
        .ok_or_else(|| ILinkError::Shape("缺少 qrcode 字段".into()))?
        .to_string();
    let image = v
        .get("qrcode_img_content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(QrTicket {
        qrcode,
        image,
        // 二维码约 5 分钟有效。协议给的是相对时长，这里换算成绝对时间戳，
        // 让前端直接显示剩余秒数而不必知道这个协议细节。
        expires_at: now + QR_TTL_SECONDS,
    })
}

pub const QR_TTL_SECONDS: i64 = 300;

/// 扫码状态机。
///
/// **不 derive PartialEq**：`Confirmed` 里带着 `Session`，而 `Session`
/// 含 `reqwest::Client`（不可能比较）。要判断某个分支，看它长什么样
/// 而不是在测试里 `assert_eq!`。
#[derive(Debug, Clone)]
pub enum QrStatus {
    /// 等扫码。
    Waiting,
    /// 已扫码，等手机确认。
    Scanned,
    /// 确认成功，凭据在这里。
    Confirmed(Session),
    /// 二维码过期，要重新取一张。
    Expired,
    /// 服务端说了别的状态值。
    Unknown(String),
}

/// `GET get_qrcode_status?qrcode=`
pub async fn poll_qr_status(
    base_url: &str,
    qrcode: &str,
) -> Result<QrStatus, ILinkError> {
    let v = http()?
        .get(base(base_url, "get_qrcode_status"))
        .query(&[("qrcode", qrcode)])
        .send()
        .await
        .map_err(|e| ILinkError::Http(e.to_string()))?
        .json::<Value>()
        .await
        .map_err(|e| ILinkError::Shape(format!("扫码状态不是 JSON：{e}")))?;
    check(&v)?;

    match v.get("status").and_then(Value::as_str).unwrap_or("") {
        "wait" => Ok(QrStatus::Waiting),
        "scaned" => Ok(QrStatus::Scanned),
        "expired" => Ok(QrStatus::Expired),
        "confirmed" => {
            let token = v
                .get("bot_token")
                .and_then(Value::as_str)
                .ok_or_else(|| ILinkError::Shape("confirmed 响应缺少 bot_token".into()))?
                .to_string();
            let bot_id = v
                .get("ilink_bot_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let user_id = v
                .get("ilink_user_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // baseurl 优先用服务端返回的：跨机房调度后默认基址可能不对。
            let base_url = v
                .get("baseurl")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(DEFAULT_BASE_URL)
                .to_string();
            Ok(QrStatus::Confirmed(Session {
                token,
                bot_id,
                user_id,
                base_url,
                client: http()?,
            }))
        }
        other => Ok(QrStatus::Unknown(other.to_string())),
    }
}

// -------------------------------------------------------------------- 收发消息

/// 一条收到的消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inbound {
    pub from_user_id: String,
    pub text: String,
    /// 回复时**必须**原样带回。不带，回复挂不上会话。
    pub context_token: String,
    pub message_id: String,
}

/// 一次长轮询的结果。
#[derive(Debug, Clone, Default)]
pub struct PollBatch {
    pub messages: Vec<Inbound>,
    /// 服务端返回的新游标。**必须持久化**，否则重启后会重放整个历史。
    pub cursor: String,
}

/// `POST getupdates`（长轮询，约 35 秒）
///
/// 客户端侧超时按「这一轮没消息」处理并立刻再发一轮 —— 服务端挂满 35 秒
/// 是正常节奏，不是故障。
pub async fn poll_updates(
    session: &Session,
    cursor: &str,
) -> Result<PollBatch, ILinkError> {
    let v = auth_headers(
        session
            .client
            .post(base(&session.base(), "getupdates"))
            .timeout(POLL_TIMEOUT + Duration::from_secs(5))
            .json(&json!({
                "get_updates_buf": cursor,
                "base_info": { "channel_version": "2.1.1" },
            })),
        &session.token,
    )
    .send()
    .await
    .map_err(|e| ILinkError::Http(e.to_string()))?
    .json::<Value>()
    .await
    .map_err(|e| ILinkError::Shape(format!("长轮询响应不是 JSON：{e}")))?;
    check(&v)?;

    let next = v
        .get("get_updates_buf")
        .and_then(Value::as_str)
        .unwrap_or(cursor)
        .to_string();

    let mut messages = Vec::new();
    if let Some(raw) = v.get("msgs").and_then(Value::as_array) {
        for m in raw {
            if let Some(inb) = parse_inbound(m) {
                messages.push(inb);
            }
        }
    }
    Ok(PollBatch {
        messages,
        cursor: next,
    })
}

/// 从一条原始消息里取文本。第一版只取文本：媒体要 CDN + AES-128-ECB
/// 解密，没接之前如实跳过而不是塞个占位符。
fn parse_inbound(m: &Value) -> Option<Inbound> {
    let from_user_id = m.get("from_user_id").and_then(Value::as_str)?;
    let context_token = m
        .get("context_token")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let mut text = String::new();
    if let Some(items) = m.get("item_list").and_then(Value::as_array) {
        for item in items {
            if item.get("type").and_then(Value::as_i64) == Some(1) {
                if let Some(t) = item
                    .get("text_item")
                    .and_then(|x| x.get("text"))
                    .and_then(Value::as_str)
                {
                    text.push_str(t);
                }
            }
        }
    }
    if text.trim().is_empty() {
        return None;
    }

    Some(Inbound {
        from_user_id: from_user_id.to_string(),
        text,
        context_token,
        message_id: m
            .get("message_id")
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
    })
}

/// `POST sendmessage`
///
/// `context_token` 是必填的 —— 少了它，微信那边收到的是一条挂不上会话的
/// 悬空消息，所以这里缺失即报错，不发出去。
pub async fn send_text(
    session: &Session,
    to_user_id: &str,
    context_token: &str,
    text: &str,
) -> Result<(), ILinkError> {
    if context_token.is_empty() {
        return Err(ILinkError::Shape(
            "缺少 context_token：iLink 要求回复必须带上，否则消息挂不上会话".into(),
        ));
    }
    if text.trim().is_empty() {
        return Err(ILinkError::Shape("拒绝发送空消息".into()));
    }
    let v = auth_headers(
        session
            .client
            .post(base(&session.base(), "sendmessage"))
            .timeout(Duration::from_secs(15))
            .json(&json!({
                "msg": {
                    "from_user_id": "",
                    "to_user_id": to_user_id,
                    "message_type": 2,
                    "message_state": 2,
                    "context_token": context_token,
                    "item_list": [{ "type": 1, "text_item": { "text": text } }],
                    "base_info": { "channel_version": "2.1.1" },
                }
            })),
        &session.token,
    )
    .send()
    .await
    .map_err(|e| ILinkError::Http(e.to_string()))?
    .json::<Value>()
    .await
    .map_err(|e| ILinkError::Shape(format!("发送响应不是 JSON：{e}")))?;
    check(&v)
}

/// 微信消息长度上限。超过就按段落边界切分 —— 直接截断会把话说到一半。
pub const MAX_TEXT: usize = 4000;

/// 按段落边界切分。切不动时才硬切（单段就超长的那种）。
pub fn split_for_wechat(text: &str) -> Vec<String> {
    if text.chars().count() <= MAX_TEXT {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for para in text.split('\n') {
        let candidate_len = cur.chars().count() + para.chars().count() + 1;
        if candidate_len > MAX_TEXT && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if para.chars().count() > MAX_TEXT {
            // 单段就超长：只能硬切。
            let mut chunk = String::new();
            for ch in para.chars() {
                if chunk.chars().count() >= MAX_TEXT {
                    out.push(std::mem::take(&mut chunk));
                }
                chunk.push(ch);
            }
            cur = chunk;
        } else {
            cur.push_str(para);
            cur.push('\n');
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(b64("0"), "MA==");
        assert_eq!(b64("12"), "MTI=");
        assert_eq!(b64("123"), "MTIz");
        assert_eq!(b64("1234"), "MTIzNA==");
        assert_eq!(b64("a"), "YQ==");
    }

    #[test]
    fn base_url_joins_without_double_slash() {
        assert_eq!(
            base("https://x.example", "sendmessage"),
            "https://x.example/ilink/bot/sendmessage"
        );
        assert_eq!(
            base("https://x.example/", "sendmessage"),
            "https://x.example/ilink/bot/sendmessage"
        );
    }

    #[test]
    fn expired_code_is_recognised_only_for_minus_fourteen() {
        assert!(ILinkError::Remote { code: -14, message: String::new() }.is_session_expired());
        assert!(!ILinkError::Remote { code: -2, message: String::new() }.is_session_expired());
        assert!(!ILinkError::Http("x".into()).is_session_expired());
    }

    #[test]
    fn inbound_parses_text_and_keeps_context_token() {
        let m = json!({
            "from_user_id": "u1@im.wechat",
            "context_token": "CT-123",
            "message_id": 99,
            "item_list": [{ "type": 1, "text_item": { "text": "帮我看看代码" } }]
        });
        let inb = parse_inbound(&m).expect("文本消息应能解析");
        assert_eq!(inb.from_user_id, "u1@im.wechat");
        assert_eq!(inb.text, "帮我看看代码");
        assert_eq!(inb.context_token, "CT-123", "回复必须原样带回");
        assert_eq!(inb.message_id, "99");
    }

    #[test]
    fn inbound_skips_media_only_messages() {
        // 第一版不接媒体，媒体消息不能变成一条空文本塞给模型。
        let m = json!({
            "from_user_id": "u1@im.wechat",
            "context_token": "CT",
            "item_list": [{ "type": 2, "image_item": { "media": {} } }]
        });
        assert!(parse_inbound(&m).is_none());
    }

    #[test]
    fn inbound_ignores_message_without_sender() {
        let m = json!({ "context_token": "CT", "item_list": [] });
        assert!(parse_inbound(&m).is_none());
    }

    #[test]
    fn split_keeps_short_text_as_one_message() {
        assert_eq!(split_for_wechat("短句"), vec!["短句".to_string()]);
    }

    #[test]
    fn split_breaks_on_paragraph_boundary_not_mid_sentence() {
        let para = "一".repeat(MAX_TEXT - 10);
        let text = format!("{para}\n{para}");
        let parts = split_for_wechat(&text);
        assert!(parts.len() >= 2, "超长必须切：{}", parts.len());
        for p in &parts {
            assert!(p.chars().count() <= MAX_TEXT, "每条都不得超过上限");
        }
    }

    #[test]
    fn split_hard_cuts_a_single_oversized_paragraph() {
        let text = "长".repeat(MAX_TEXT + 500);
        let parts = split_for_wechat(&text);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.chars().count() <= MAX_TEXT));
    }

    #[test]
    fn check_treats_absent_and_null_codes_as_success() {
        assert!(check(&json!({})).is_ok());
        assert!(check(&json!({ "ret": 0, "errcode": null })).is_ok());
        assert!(check(&json!({ "ret": 1, "errmsg": "bad" })).is_err());
    }

    #[test]
    fn check_surfaces_remote_code() {
        let e = check(&json!({ "errcode": -14, "errmsg": "session expired" }))
            .expect_err("-14 应报错");
        assert!(e.is_session_expired());
    }
}