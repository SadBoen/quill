//! 登录 / 登出 / 首管引导。
//!
//! 契约照抄 Octop `dashboard/src/api/modules/auth.ts:8-23` 的端点清单：
//!
//! | 路由 | 鉴权 | 响应 |
//! |---|---|---|
//! | `GET  /api/setup/status`         | 公开 | `{ setup_required }` |
//! | `POST /api/setup/initial-admin`  | 公开 | 201 `{ id, username, role }` |
//! | `POST /api/auth/login`           | 公开 | `{ access_token, token_type, expires_in, user }` |
//! | `POST /api/auth/refresh`         | 需令牌 | 同 login |
//! | `POST /api/auth/logout`          | 需令牌 | 200 `{ revoked }` |
//! | `GET  /api/auth/me`              | 需令牌 | `{ id, username, role, display_name }` |
//!
//! **「注册默认关闭」在这里怎么落地**：本文件**没有** `/api/auth/register`。
//! 唯一的建号入口是 `/api/setup/initial-admin`，它只在 `users` 表为空时成功
//! （`ControlPlane::create_first_owner` 强制这一点），一旦有用户就永久返回 409。
//! 所以「注册」在这套设计里等于「首次安装向导」，装完即焚，跟 Octop 一致。
//!
//! 与 Octop 的**有意差异**（都是 quill 没有的东西，不是省略）：
//! - Octop 登录带 `captcha_token`、还有 OIDC / OAuth / LDAP / 忘记密码 / 改密码。
//!   quill 后端一个都没有，抄过来就是恒定失败的字段，故不抄。理由记在 `BACKLOG.md`。
//! - Octop 的 `/api/setup/initial-admin` 返回数字 `id`；quill 的用户 id 是 16 字节
//!   UUID（外键与 `derive_user_id` 都按这个来），这里返回 32 位紧凑 hex 字符串。
//! - Octop `logout` 返 204 空体；quill 返 200 + `{ revoked }`，因为**幂等**语义需要
//!   告诉调用方「这次到底有没有真的吊销掉令牌」。
//!
//! 另有一条**不抄**的环境变量入口：`QUILL_PASSWORD_USERS`（见 `server.rs`）。它服务
//! 的是没法点浏览器的场景（容器、CI、远端机器）。它不是注册的替代品：注册照样关闭。

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::body::JsonBody;
use crate::db::DbBridge;
use crate::error::ApiError;
use crate::state::AppState;

use quill_control::{ControlError, ControlPlane, RegistrationRequest, SystemClock};

/// 在 `DbBridge` 的工作线程里跑一段 ControlPlane 操作。
///
/// 不能把 `SqlitePool` 长期持有在 AppState 里：`DbBridge` 用的是「单线程 + 独占池」
/// 的模型，池的所有权留在它的 worker 里，跨线程直接用会踩数据竞争。
/// 所以每个需要 ControlPlane 的请求都进 `call` 一次，用完即还。
async fn with_control<T, F>(db: &Arc<DbBridge>, pbkdf2: quill_control::Pbkdf2Params, f: F) -> Result<T, ControlError>
where
    F: FnOnce(
            ControlPlane,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, ControlError>> + Send>>
        + Send
        + 'static,
    T: Send + 'static,
{
    // `DbBridge::call` 的错误通道是 `AgentError`（历史遗留），而账号域只认
    // `ControlError`，两个 crate 不能互相依赖成环。这里不去硬转错误类型，
    // 而是**把结果原样放进槽位带出来**：通道上只回一个 `Ok(())`，
    // 真正的业务错误不经过任何字符串转换，也不丢类型。
    let slot: Arc<std::sync::Mutex<Option<Result<T, ControlError>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let writer = Arc::clone(&slot);

    db.call(move |pool, _rt| {
        Box::pin(async move {
            let cp = ControlPlane::new_with_os_entropy(pool, Arc::new(SystemClock), pbkdf2);
            *writer.lock().expect("结果槽位不该被毒化") = Some(f(cp).await);
            Ok(())
        })
    })
    .map_err(|e| ControlError::Storage {
        detail: format!("走 DbBridge 执行账号操作时通道本身失败：{e}"),
    })?;

    let out = slot.lock().expect("结果槽位不该被毒化").take();
    match out {
        Some(r) => r,
        None => Err(ControlError::InvariantBroken {
            detail: "DbBridge 通道正常返回却没有回填结果：这说明 call 的契约被破坏了".to_string(),
        }),
    }
}

fn control_err(op: &str, e: ControlError) -> ApiError {
    // 对外不区分「用户不存在」和「口令不对」：统一走 401 且文案逐字相同，
    // 否则这个端点就成了用户名枚举器。服务端日志里保留真实原因。
    match &e {
        ControlError::CredentialsRejected | ControlError::SessionUnknown => {
            eprintln!("[auth] {op} 凭据被拒");
            ApiError::unauthorized()
        }
        ControlError::AccountLocked {
            username_norm,
            until_ms,
        } => {
            eprintln!("[auth] {op} 账号 {username_norm} 被锁定至 {until_ms}");
            ApiError::forbidden(format!(
                "账号 {username_norm} 因多次登录失败被临时锁定。\
                 下一步：等锁定结束后重试，或由管理员在实例配置里调整登录策略。"
            ))
        }
        ControlError::AccountDisabled { username_norm } => {
            eprintln!("[auth] {op} 账号 {username_norm} 已停用");
            ApiError::forbidden(format!(
                "账号 {username_norm} 已被停用，无法登录。\
                 下一步：由管理员在用户管理里把它恢复为启用。"
            ))
        }
        ControlError::FirstOwnerExists => ApiError::conflict(
            "本实例已经创建过初始管理员，注册通道已永久关闭。".to_string(),
            "用已有账号登录（POST /api/auth/login），\
             或由部署者通过 QUILL_PASSWORD_USERS 另行配置账号后重启。",
        ),
        ControlError::UsernameInvalid { raw, reason }
        | ControlError::DisplayNameInvalid { raw, reason } => ApiError::bad_request(format!(
            "{raw:?} 非法：{reason}。下一步：用户名用 3~32 个字母、数字或下划线；显示名最长 64 个字符。"
        )),
        ControlError::PasswordTooShort { min, .. } => ApiError::bad_request(format!(
            "口令至少要 {min} 个字符。下一步：换一个更长的口令。"
        )),
        ControlError::PasswordTooLong { max, .. } => ApiError::bad_request(format!(
            "口令最多 {max} 个字符。下一步：换一个更短的口令。"
        )),
        ControlError::PasswordEqualsUsername => ApiError::bad_request(
            "口令不能和用户名相同。下一步：换一个不同的口令。".to_string(),
        ),
        other => {
            eprintln!("[auth] {op} 失败：{other}");
            ApiError::internal(format!("{op}时发生内部错误（详情见服务端日志）。"))
        }
    }
}

/// Octop 的 user 对象形状（`auth.ts` 的 `OctopUser`）：只给前端显示要用的字段，
/// 不回传 `token_epoch`、失败计数这些内部状态。
fn role_str(r: quill_control::UserRole) -> &'static str {
    match r {
        quill_control::UserRole::Owner => "owner",
        quill_control::UserRole::Member => "member",
    }
}

fn status_str(s: quill_control::UserStatus) -> &'static str {
    match s {
        quill_control::UserStatus::Active => "active",
        quill_control::UserStatus::Disabled => "disabled",
    }
}

/// Octop 的登录响应：`{ access_token, token_type, expires_in, user }`。
/// quill 额外带一个 `refresh_token` 占位？——不。refresh 复用同一个会话令牌
/// （`ControlPlane::refresh` 是**轮换**同一行，不是签发第二个），所以没有第二个令牌可发。
fn session_json(s: &quill_control::AuthSession) -> Value {
    json!({
        "access_token": s.token,
        "token_type": "Bearer",
        "expires_in": ((s.expires_at_ms - s.issued_at_ms) / 1000).max(0),
        "expires_at": s.expires_at_ms,
        "user": json!({
            "id": s.user_id.to_compact_hex(),
            "username": s.username_norm,
            "role": role_str(s.role),
        }),
    })
}

// ---------------------------------------------------------------- 公开端点

/// `GET /api/setup/status` —— 登录页据此决定「显示登录表单」还是「显示首管引导」。
///
/// `setup_required` 的口径 = `users` 表为空。这与 `create_first_owner` 的判定
/// 同源，所以「status 说需要引导」与「initial-admin 会成功」永远一致，不会出现
/// 前端引导页指着一个必然 409 的按钮。
pub async fn setup_status(State(state): State<AppState>) -> Result<Response, ApiError> {
    let db = state.db()?.clone();
    let count = with_control(&db, state.pbkdf2, |cp| {
        Box::pin(async move { Ok(cp.user_count().await?) })
    })
    .await
    .map_err(|e| control_err("查询是否需要首管引导", e))?;

    Ok(Json(json!({
        "setup_required": count == 0,
        "user_count": count,
        // 注册永远关闭。把这句话写进响应体，客户端不必靠「路由不存在」来推断。
        "registration_enabled": false,
    }))
    .into_response())
}

/// `POST /api/setup/initial-admin` —— 建第一个（也是这套流程里唯一一个能建的）管理员。
///
/// 建成后注册通道永久关闭：重复调用返回 409。
pub async fn initial_admin(
    State(state): State<AppState>,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    crate::api_experts::only_keys(
        &body,
        &["username", "password", "display_name"],
        "POST /api/setup/initial-admin",
    )?;
    let req = registration_request(&body)?;

    let db = state.db()?.clone();
    let profile = with_control(&db, state.pbkdf2, move |cp| {
        Box::pin(async move {
            let p = cp.create_first_owner(&req).await?;
            Ok(p)
        })
    })
    .await
    .map_err(|e| control_err("创建初始管理员", e))?;

    Ok((StatusCode::CREATED, Json(created_admin_json(&profile))).into_response())
}

/// Octop 的 `initial-admin` 响应要 `id` / `username` / `role` 三个字段；
/// quill 额外带上 `status` 与 `display_name`，因为登录页建完就要直接显示身份，
/// 多这一次 GET 不值当（`status` 用字符串而不是枚举数字，前端不必认识内部枚举）。
fn created_admin_json(p: &quill_control::UserProfile) -> Value {
    json!({
        "id": p.id.to_compact_hex(),
        "username": p.username,
        "display_name": p.display_name,
        "role": role_str(p.role),
        "status": status_str(p.status),
    })
}

fn registration_request(body: &Value) -> Result<RegistrationRequest, ApiError> {
    let username = need_str(body, "username")?;
    let password = need_str(body, "password")?;
    // display_name 省略时用用户名，跟 Octop 的 `display_name?` 可选语义一致。
    let display = match body.get("display_name") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        _ => username.clone(),
    };
    Ok(RegistrationRequest::new(username, display, password))
}

/// `POST /api/auth/login` —— 用户名 + 密码换会话令牌。
///
/// **限流位置是这里的关键设计**：`acquire` 必须在 `with_control` 之前跑。
/// 进了 `with_control` 就等于进了 PBKDF2，而 PBKDF2 一轮 60 万次迭代不可
/// 取消。所以顺序是：
///
/// 1. 解析请求体（纯字符串，零成本）
/// 2. `login_limiter.acquire()` —— 纯内存，命中即 429，绝不碰 PBKDF2
/// 3. `with_control(cp.login(...))` —— 唯一会烧 CPU 的地方
/// 4. 成功清空窗口，失败记一笔
///
/// 对照 `quill-control` 内部的账号锁定：它按**账号**计数，而「用户名根本
/// 不存在」这条分支上没有任何账号可锁，攻击者可以拿随机用户名无限触发
/// PBKDF2。第 2 步补的正是这个洞。
pub async fn login(
    State(state): State<AppState>,
    peer: crate::ratelimit::PeerAddr,
    headers: HeaderMap,
    JsonBody(body): JsonBody,
) -> Result<Response, ApiError> {
    crate::api_experts::only_keys(&body, &["username", "password"], "POST /api/auth/login")?;
    let username = need_str(&body, "username")?;
    let password = need_str(&body, "password")?;

    let now = crate::ratelimit::now_ms();
    let peer_segment =
        crate::ratelimit::peer_segment(&headers, peer.get(), state.config.trust_proxy);
    let limit_key = crate::ratelimit::RateLimiter::key(&username, &peer_segment);

    if let Err(retry_after) = state.login_limiter.acquire(&limit_key, now) {
        eprintln!(
            "[auth] 登录限流触发：用户名 {} 来源 {}，建议等待 {retry_after}s",
            sanitize(&username),
            peer_segment
        );
        return Err(ApiError::too_many_requests(
            format!(
                "登录尝试过于频繁，已被临时挡下。\
                 下一步：等 {} 秒后重试；若是脚本在轮询登录，请把间隔放宽到 1 秒以上。",
                retry_after
            ),
            retry_after,
        ));
    }

    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| "unknown".to_string());

    let db = state.db()?.clone();
    let session = with_control(&db, state.pbkdf2, move |cp| {
        Box::pin(async move {
            let s = cp.login(&username, &password, Some(ua.as_str()), None).await?;
            Ok(s)
        })
    })
    .await;

    // 只有走到这里才说明 PBKDF2 真的跑过了。无论结果如何都要记账：
    // 成功清零，失败累加。
    match session {
        Ok(s) => {
            state.login_limiter.record_success(&limit_key);
            Ok(Json(session_json(&s)).into_response())
        }
        Err(e) => {
            state.login_limiter.record_failure(&limit_key, now);
            Err(control_err("登录", e))
        }
    }
}

/// 打进日志的用户名清洗：攻击者会拿超长/带控制字符的用户名来撑爆日志行。
fn sanitize(s: &str) -> String {
    const MAX: usize = 32;
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX)
        .collect();
    if s.chars().filter(|c| !c.is_control()).count() > MAX {
        format!("{cleaned}…")
    } else {
        cleaned
    }
}

// ---------------------------------------------------------------- 需令牌端点

/// `GET /api/auth/me` —— 当前身份。Octop 的形状是 `{ id, username, role, display_name }`。
///
/// 之前这里只回 `user_id`（32 位 hex），侧栏就显示成 `a93dfdd8bb8…` 这种看不懂的串。
/// 现在回**真实用户名**：先查环境变量令牌的名册（零 DB 开销，覆盖 dev-token 这类
/// 直给凭据），查不到再回库按 user_id 取 profile（覆盖登录签发的会话令牌）。
pub async fn me(
    State(state): State<AppState>,
    user: crate::auth::AuthUser,
) -> Result<Response, ApiError> {
    let uid = user.0.user_id;

    // 快路径：QUILL_TOKENS 里写的是 @alice / 32 位 id，登录名就在解析器手上。
    if let Some(resolver) = state
        .tokens
        .as_any_composite()
        .and_then(|c| c.env_login_of(uid))
    {
        return Ok(Json(json!({
            "id": uid.to_compact_hex(),
            "username": resolver,
            "display_name": resolver,
            "role": if user.0.is_admin { "owner" } else { "member" },
            "is_admin": user.0.is_admin,
            "approval_mode": "manual",
        }))
        .into_response());
    }

    let db = state.db()?.clone();
    let profile = with_control(&db, state.pbkdf2, move |cp| {
        Box::pin(async move {
            let p = cp.get_user(&uid, &uid).await?;
            Ok(p)
        })
    })
    .await;

    // 取不到 profile 不该让登录态崩掉（用户可能刚从库里被删）。退化成只报 id，
    // 并在响应里如实说明原因，而不是假装一切正常。
    match profile {
        Ok(p) => Ok(Json(json!({
            "id": uid.to_compact_hex(),
            "username": p.username,
            "display_name": p.display_name,
            "role": role_str(p.role),
            "status": status_str(p.status),
            "is_admin": user.0.is_admin,
            "approval_mode": "manual",
        }))
        .into_response()),
        Err(e) => {
            eprintln!("[auth] 读取当前用户资料失败：{e}");
            Ok(Json(json!({
                "id": uid.to_compact_hex(),
                "username": uid.to_compact_hex(),
                "display_name": uid.to_compact_hex(),
                "role": if user.0.is_admin { "owner" } else { "member" },
                "is_admin": user.0.is_admin,
                "profile_unavailable": true,
                "profile_error": "账号资料读取失败，这里只能显示标识。详情见服务端日志。",
                "approval_mode": "manual",
            }))
            .into_response())
        }
    }
}

/// `POST /api/auth/refresh` —— 轮换当前会话令牌。
pub async fn refresh(
    State(state): State<AppState>,
    _user: crate::auth::AuthUser,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let token = bearer(&headers)?;
    let db = state.db()?.clone();
    let session = with_control(&db, state.pbkdf2, move |cp| {
        Box::pin(async move {
            let s = cp.refresh(&token).await?;
            Ok(s)
        })
    })
    .await
    .map_err(|e| control_err("续期会话", e))?;

    // 轮换后旧令牌立即失效；只有持有旧令牌的人能换到新令牌（refresh 内部按
    // 令牌摘要定位那一行，不接受「指定要续谁」）。
    Ok(Json(session_json(&session)).into_response())
}

/// `POST /api/auth/logout` —— 吊销当前会话令牌。
///
/// **幂等**：重复登出返回 200 + `revoked:false`，不报 401。客户端在网络超时后
/// 重试登出是常态，把「已经登出了」报成错误只会逼出无意义的告警。
/// 因此这里**故意不挂 `AuthUser` 提取器** —— 挂了的话令牌一被吊销，下次请求
/// 在进处理器之前就被 401 拦掉，`revoked:false` 这条分支永远走不到。
///
/// **不是会话令牌的令牌也走这条路**。`cp.logout` 对两种情况是**报错**而不是
/// 返回 `revoked:0`：
/// - `SessionUnknown` —— 格式对，但库里没有这一行（已登出 / 从未存在）
/// - `TokenMalformed` —— 连 64 位 hex 都不是（典型：环境变量令牌 `dev-token`）
///
/// 这两种都在这里归一成 `revoked:0` + 200。不这么做的话，拿一个环境变量
/// 令牌点「退出登录」会收到 **500**，前端只能弹「服务器内部错误」——
/// 而用户做的事完全正常，是我们的分类不对。
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let token = bearer(&headers)?;
    let db = state.db()?.clone();
    let outcome = with_control(&db, state.pbkdf2, move |cp| {
        Box::pin(async move {
            let o = cp.logout(&token).await?;
            Ok(o)
        })
    })
    .await;

    let revoked = match outcome {
        Ok(o) => o.revoked,
        Err(ControlError::SessionUnknown) | Err(ControlError::TokenMalformed) => {
            eprintln!("[auth] 登出：令牌不在有效会话里，按「本来就没登录」处理");
            0
        }
        Err(e) => return Err(control_err("登出", e)),
    };

    Ok(Json(json!({
        "revoked": revoked > 0,
        "revoked_count": revoked,
        "note": if revoked > 0 {
            "会话令牌已吊销，之后用它访问接口会返回 401。"
        } else {
            "这个令牌当前不在有效会话里（可能已经登出过，或它压根不是登录签发的会话令牌）。\
             另外：环境变量令牌（QUILL_TOKENS）不受登出影响，\
             要收回得改配置并重启。"
        }
    }))
    .into_response())
}

// ---------------------------------------------------------------- 工具

fn bearer(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(ApiError::unauthorized)
}

fn need_str(body: &Value, key: &str) -> Result<String, ApiError> {
    match body.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        Some(Value::String(_)) => Err(ApiError::bad_request(format!("字段 {key} 不能是空串。"))),
        Some(other) => Err(ApiError::bad_request(format!(
            "字段 {key} 必须是字符串，实际收到 {}。",
            type_name(other)
        ))),
        None => Err(ApiError::bad_request(format!("缺少必填字段 {key}。"))),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_is_reported_as_closed_in_setup_status() {
        // 「注册关闭」是本模块的核心承诺：status 响应体里必须显式带上这个字段，
        // 客户端不能靠「路由不存在」去推断。
        let v = json!({
            "setup_required": true,
            "user_count": 0,
            "registration_enabled": false,
        });
        assert_eq!(v["registration_enabled"], json!(false));
    }

    #[test]
    fn role_and_status_render_as_octop_strings() {
        assert_eq!(role_str(quill_control::UserRole::Owner), "owner");
        assert_eq!(role_str(quill_control::UserRole::Member), "member");
        assert_eq!(status_str(quill_control::UserStatus::Active), "active");
        assert_eq!(status_str(quill_control::UserStatus::Disabled), "disabled");
    }

    #[test]
    fn wrong_password_and_unknown_user_produce_an_identical_401() {
        // 防用户名枚举：两种失败对外必须逐字相同。
        let a = control_err("登录", ControlError::CredentialsRejected);
        let b = control_err("登录", ControlError::SessionUnknown);
        assert_eq!(a.status(), b.status());
        assert_eq!(a.detail(), b.detail());
        assert_eq!(a.code(), "unauthorized");
    }

    #[test]
    fn a_second_admin_attempt_is_409_not_500() {
        let e = control_err("创建初始管理员", ControlError::FirstOwnerExists);
        assert_eq!(e.status(), axum::http::StatusCode::CONFLICT);
        assert!(e.detail().contains("注册通道已永久关闭"));
        // 「下一步」现在走 advice 字段，不再塞进 detail ——
        // 两条一起断言，免得以后有人把建议又塞回 detail 里（那样只剩一份）。
        assert!(e.next_step().contains("QUILL_PASSWORD_USERS"));
        assert!(
            !e.detail().contains("QUILL_PASSWORD_USERS"),
            "建议不该重复出现在 detail 里：{}",
            e.detail()
        );
    }

    #[test]
    fn sanitizing_a_logged_username_bounds_its_length_and_strips_control_chars() {
        // 攻击者会拿超长、带控制字符的用户名来撑爆日志行 / 注入换行。
        assert_eq!(sanitize("alice"), "alice");
        assert_eq!(sanitize("a\nb\rc"), "abc");
        let long = "x".repeat(500);
        let s = sanitize(&long);
        assert!(s.chars().count() <= 33, "日志里的用户名必须有上界：{} 字符", s.chars().count());
        assert!(s.ends_with('…'), "截断要看得见，不能让人以为那是完整值");
    }

    #[test]
    fn session_json_matches_the_octop_contract() {
        let s = quill_control::AuthSession {
            session_id: quill_domain::SessionId::from_bytes([1u8; 16]),
            user_id: quill_domain::UserId::parse("0192b7c8-0000-7000-8000-000000000001")
                .expect("uid"),
            username_norm: "alice".to_string(),
            role: quill_control::UserRole::Owner,
            token: "tok".to_string(),
            issued_at_ms: 1_000,
            expires_at_ms: 61_000,
        };
        let v = session_json(&s);
        assert_eq!(v["access_token"], json!("tok"));
        assert_eq!(v["token_type"], json!("Bearer"));
        assert_eq!(v["expires_in"], json!(60));
        assert_eq!(v["user"]["username"], json!("alice"));
        assert_eq!(v["user"]["role"], json!("owner"));
        assert_eq!(v["user"]["id"].as_str().map(str::len), Some(32));
    }

    #[test]
    fn registration_request_defaults_display_name_to_username() {
        let r = registration_request(&json!({ "username": "alice", "password": PW })).expect("ok");
        assert_eq!(r.display_name, "alice");
        assert_eq!(r.role, quill_control::UserRole::Member);

        let r2 = registration_request(&json!({
            "username": "alice", "password": PW, "display_name": " 爱丽丝 "
        }))
        .expect("ok");
        assert_eq!(r2.display_name, "爱丽丝", "显示名两端空格应去掉");
    }

    #[test]
    fn a_missing_or_blank_field_is_400_not_a_panic() {
        assert!(registration_request(&json!({ "password": PW })).is_err());
        assert!(registration_request(&json!({ "username": "", "password": PW })).is_err());
        assert!(registration_request(&json!({ "username": 1, "password": PW })).is_err());
    }

    const PW: &str = "correct-horse-battery";
}
