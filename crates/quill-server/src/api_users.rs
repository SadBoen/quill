//! 用户管理的 HTTP 接线：列用户、改状态。
//!
//! 这一层原先是四根 501 桩。补的时候先把「这台实例里到底有哪些账号」搞清楚，
//! 结论直接决定了哪些能做、哪些必须继续 501：
//!
//! 1. **账号有两类，但都落在 `users` 表里。** 启动时 `bootstrap::ensure_token_user`
//!    会给 `QUILL_TOKENS` 里的每个令牌建一行（`password_algo='token-only'`），
//!    `ensure_password_user` 给 `QUILL_PASSWORD_USERS` 里的每个口令建一行。
//!    所以 `ControlPlane::list_users` 就是完整名册，不必把两类拼起来 ——
//!    拼起来反而会出现只有令牌、库里没行的那种「幽灵账号」。
//!
//! 2. **`POST /api/users` 继续 501，而且是有意的。** `bootstrap.rs` 的文件头写死了
//!    这台实例的口径：账号只能由部署者写进环境变量再重启，没有自助注册。
//!    给 owner 开一个「点一下就建号」的接口，等于把那条口径从后门绕过去了。
//!    想加账号就改配置 —— 这条 501 的「下一步」直接说清这一点。
//!
//! 3. **`DELETE /api/users/{id}` 继续 501。** `users` 表有 `deleted_at` 列，
//!    但全仓库**没有任何一处写过它**（`repo.rs` 里只有 `WHERE deleted_at IS NULL`
//!    这个读过滤）。也就是说软删除只是 schema 里预留的一半，真删会把用户的
//!!    会话、专家、团队全变成外键孤儿。在真做之前报 501，比返回一个「删掉了」
//!    却什么都没删的 200 诚实。
//!
//! 4. **停用挡不住 `QUILL_TOKENS`。** 这是本文件最要紧的一条：
//!    `CompositeTokenResolver::resolve` 先查环境变量令牌表，命中就直接放行，
//!    **完全不查库**，所以那个人的状态改成 `disabled` 也照样进得来。
//!    停用对**会话与口令**是真的生效的（`ControlPlane::authenticate` 会检查
//!    `user_status`，已登录的人下一个请求就被拒），但对持有环境变量令牌的人无效。
//!    所以响应里必须带 `has_env_token`，让界面能说出「已停用，但这个人手里
//!    还有令牌」—— 否则就是拿一个假的「停用」糊弄用户。

use axum::extract::{Path, Query, State};
use axum::Json;
use serde_json::{json, Value};

use quill_control::{ControlError, UserId, UserProfile, UserStatus};

use crate::api_auth::with_control;
use crate::api_experts::only_keys;
use crate::auth::{AuthUser, CompositeTokenResolver};
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::state::AppState;

/// 一页最多给多少条。上限不是装饰：没有它，一个 `?limit=1000000`
/// 就能让一个管理接口去序列化整张表。
pub const MAX_LIMIT: usize = 200;
const DEFAULT_LIMIT: usize = 50;

/// `GET /api/users` 的查询串。
#[derive(Debug, Default, serde::Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    offset: Option<usize>,
    limit: Option<usize>,
}

fn to_api(e: ControlError) -> ApiError {
    crate::api_auth::control_err("用户管理", e)
}

/// 这台实例的账号里，这个人是否还握着一枚 `QUILL_TOKENS` 令牌。
///
/// 取不到复合解析器时（比如测试里塞了个裸 `EnvTokenResolver`）**按 false 报**，
/// 不按 true 报：声称「挡不住」而实际挡得住，会让人白折腾一趟去改环境变量。
fn has_env_token(state: &AppState, id: UserId) -> bool {
    state
        .tokens
        .as_any_composite()
        .is_some_and(|c: &CompositeTokenResolver| c.env_login_of(id).is_some())
}

fn user_json(state: &AppState, p: &UserProfile) -> Value {
    json!({
        "id": p.id.to_compact_hex(),
        "username": p.username,
        "display_name": p.display_name,
        "role": p.role.as_str(),
        "status": p.status.as_str(),
        "created_at_ms": p.created_at_ms,
        "last_login_at_ms": p.last_login_at_ms,
        // 见文件头第 4 条：这个字段不是装饰，是「停用到底挡不挡得住」的答案。
        "has_env_token": has_env_token(state, p.id),
    })
}

fn parse_user_id(raw: &str) -> Result<UserId, ApiError> {
    UserId::parse(raw.trim()).map_err(|_| {
        ApiError::bad_request(format!(
            "{raw:?} 不是合法的用户 id。下一步：用 `GET /api/users` 返回的 id 字段原文，\
             不要自己拼。"
        ))
    })
}

/// `GET /api/users?offset=&limit=` → 200 `{ users, total }`。
///
/// 真的读库，不是占位。`total` 是**过滤之后**的总数，翻页时界面才知道
/// 「还有没有下一页」—— 页面上那个「下一页」按钮此前是个纯装饰：
/// 它只改前端 state，服务端压根不认。
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let db = state.db()?;
    let pbkdf2 = state.pbkdf2;
    let actor = user.0.user_id;
    let all = with_control(db, pbkdf2, move |cp| {
        Box::pin(async move { cp.list_users(&actor).await })
    })
    .await
    .map_err(to_api)?;

    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = q.offset.unwrap_or(0);
    let total = all.len();
    let page: Vec<Value> = all
        .iter()
        .skip(offset)
        .take(limit)
        .map(|p| user_json(&state, p))
        .collect();

    Ok(Json(json!({ "users": page, "total": total })))
}

/// `PATCH /api/users/{id}`，请求体 `{ "status": "active" | "disabled" }`。
///
/// 只做启停，不做改角色、不做改口令：这两样要么需要改部署配置才算数，
/// 要么需要重算口令摘要，都不是这个接口该顺手做的事。
pub async fn set_status(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    only_keys(&body, &["status"], "PATCH /api/users/{id}")?;
    let raw = body.get("status").and_then(Value::as_str).ok_or_else(|| {
        ApiError::bad_request(
            "缺少字段 status。可选值只有 \"active\" 与 \"disabled\"。".to_string(),
        )
    })?;
    let want = UserStatus::parse(raw).map_err(|_| {
        ApiError::bad_request(format!(
            "status 取值 {raw:?} 不认识。下一步：只用 \"active\"（启用）或 \
             \"disabled\"（停用）—— 本接口不提供改角色与改口令。"
        ))
    })?;

    let target = parse_user_id(&id)?;
    let actor = user.0.user_id;

    if actor == target && want == UserStatus::Disabled {
        return Err(ApiError::conflict(
            "不能把自己停用 —— 停用之后这条会话立刻失效，你会把自己锁在门外，\
             而且没有第二个 owner 能把你放回来。"
                .to_string(),
            "让别人停用你，或直接改用另一个 owner 账号；\
             `users` 表的 token_epoch 也没有被这次操作改动。",
        ));
    }

    let db = state.db()?;
    let pbkdf2 = state.pbkdf2;
    with_control(db, pbkdf2, move |cp| {
        Box::pin(async move { cp.set_user_status(&actor, &target, want).await })
    })
    .await
    .map_err(to_api)?;

    // 回读一次，返回的是**库里现在真实的那个用户**，而不是把请求原样回显 ——
    // 回显只能证明「我们收到了你想改的」，证明不了「改成了」。
    let db = state.db()?;
    let pbkdf2 = state.pbkdf2;
    let who = actor;
    let p = with_control(db, pbkdf2, move |cp| {
        Box::pin(async move { cp.get_user(&who, &target).await })
    })
    .await
    .map_err(to_api)?;

    let mut out = user_json(&state, &p);
    let env_token = has_env_token(&state, p.id);
    if p.status == UserStatus::Disabled && env_token {
        // 这句话是本接口存在的理由。不说，界面上的「已停用」就是假的。
        out["warning"] = json!(
            "账号已停用：口令登录与已登录的会话都被挡住了。\
             但这个人还握着 QUILL_TOKENS 里的令牌，仍然可以直接进来 —— \
             令牌鉴权不查库（见 auth.rs 的复合解析器）。\
             要真正挡在外面，请把令牌从环境变量里去掉并重启服务。"
        );
    }
    Ok(Json(out))
}

/// `POST /api/users` —— 有意不做。理由见文件头第 2 条。
pub fn create_not_allowed() -> ApiError {
    ApiError::NotImplemented {
        method: "POST",
        path: "/api/users",
        advice: "这台实例**刻意不提供接口建账号**：账号只来自部署配置 \
             （QUILL_TOKENS / QUILL_PASSWORD_USERS）或首个 owner 的引导流程，\
             注册通道是关闭的（见 crates/quill-control/src/bootstrap.rs 的文件头）。\
             下一步：要加账号就改那两个环境变量之一并重启服务；\
             已经登录的人可以在 `/admin/users` 里停用别人的账号。",
    }
}

/// `DELETE /api/users/{id}` —— 有意不做。理由见文件头第 3 条。
pub fn delete_not_allowed(_id: &str) -> ApiError {
    ApiError::NotImplemented {
        method: "DELETE",
        path: "/api/users/{id}",
        advice: "这台实例不提供删除账号：`users.deleted_at` 这一列全仓库没有任何一处写入，\
             真删会让这个用户的会话、专家、团队全部变成外键孤儿。\
             下一步：用 `PATCH /api/users/{id}` 把账号停用 —— \
             停用会挡住口令登录并让已登录的会话失效，账号与它的数据都还在；\
             真要清理数据，请等软删除落地后再用这条路。",
    }
}