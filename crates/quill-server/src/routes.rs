use axum::extract::Path;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post, put};
use axum::Router;

use crate::api_admin;
use crate::api_chat;
use crate::api_auth;
use crate::api_dispatch;
use crate::api_experts;
use crate::api_extensions;
use crate::api_providers;
use crate::api_teams;
use crate::error::ApiError;
use crate::state::AppState;

fn not_implemented(method: &'static str, path: &'static str) -> ApiError {
    ApiError::NotImplemented { method, path }
}

pub fn build_router(state: AppState) -> Router {
    // 公开端点只有三类：健康检查、登录、首管引导。
    // 「注册」不在其中 —— 没有 /api/auth/register。首管引导在用户表非空后
    // 自己返 409，所以这条通道装完即焚（照 Octop 的 setup_required 模型）。
    let mut public = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/setup/status", get(api_auth::setup_status))
        .route("/api/setup/initial-admin", post(api_auth::initial_admin))
        .route("/api/auth/login", post(api_auth::login));

    if state.config.enable_selftest {
        async fn selftest_panic() -> axum::response::Response {
            panic!("自检用内部信息：不得泄漏到响应体");
        }
        public = public.route("/__selftest__/panic", get(selftest_panic));
    }

    let users = Router::new()
        .route(
            "/api/users",
            get(|_u: crate::auth::AuthUser| async { not_implemented("GET", "/api/users") })
                .post(|_u: crate::auth::AuthUser| async { not_implemented("POST", "/api/users") }),
        )
        .route(
            "/api/users/{id}",
            patch(|_u: crate::auth::AuthUser, _id: Path<String>| async {
                not_implemented("PATCH", "/api/users/{id}")
            })
            .delete(|_u: crate::auth::AuthUser, _id: Path<String>| async {
                not_implemented("DELETE", "/api/users/{id}")
            }),
        );

    let experts = Router::new()
        .route(
            "/api/experts",
            get(api_experts::list).post(api_experts::create),
        )
        .route(
            "/api/experts/import",
            post(|_u: crate::auth::AuthUser| async {
                not_implemented("POST", "/api/experts/import")
            }),
        )
        .route(
            "/api/experts/export",
            get(|_u: crate::auth::AuthUser| async {
                not_implemented("GET", "/api/experts/export")
            }),
        )
        .route(
            "/api/experts/{slug}",
            get(api_experts::get_one)
                .patch(api_experts::patch)
                .delete(api_experts::delete),
        );

    let dispatch = Router::new()
        .route(
            "/api/teams/{id}/dispatch",
            get(api_dispatch::list_round).post(api_dispatch::book),
        )
        .route("/api/dispatch/inflight", get(api_dispatch::inflight));

    let teams = Router::new()
        .route("/api/teams", get(api_teams::list).post(api_teams::create))
        .route(
            "/api/teams/{id}",
            get(api_teams::get_one)
                .patch(api_teams::patch)
                .delete(api_teams::delete),
        );

    let sessions = Router::new()
        .route(
            "/api/sessions",
            get(api_chat::list).post(api_chat::create),
        )
        .route(
            "/api/sessions/{id}",
            get(api_chat::get_one).delete(api_chat::delete),
        )
        .route(
            "/api/sessions/{id}/messages",
            get(api_chat::list_messages).post(api_chat::post_message),
        )
        .route("/api/sessions/{id}/metrics", get(api_chat::metrics))
        .route("/api/sessions/{id}/context", get(api_chat::context))
        .route("/api/usage", get(api_chat::usage));

    // 只读四个端点已接通（不需要模型）；写入/检索类要模型配合，仍是 501 桩。
    let wiki = Router::new()
        .route("/api/wiki/pages", get(crate::api_wiki::list_pages))
        .route("/api/wiki/pages/{*path}", get(crate::api_wiki::get_page))
        .route(
            "/api/wiki/ingest",
            post(|_u: crate::auth::AuthUser| async { not_implemented("POST", "/api/wiki/ingest") }),
        )
        .route(
            "/api/wiki/query",
            post(|_u: crate::auth::AuthUser| async { not_implemented("POST", "/api/wiki/query") }),
        )
        .route(
            "/api/wiki/search",
            post(|_u: crate::auth::AuthUser| async { not_implemented("POST", "/api/wiki/search") }),
        )
        .route("/api/wiki/index", get(crate::api_wiki::read_index))
        .route("/api/wiki/log", get(crate::api_wiki::read_log));

    let extensions = Router::new()
        .route(
            "/api/extensions/mcp",
            get(api_extensions::list_mcp).post(api_extensions::save_mcp),
        )
        .route(
            "/api/extensions/mcp/{name}",
            // PATCH 还没接：前端目前是全量 POST，单条改用不上。
            // 留着桩是为了契约完整，但**必须继续报 501** ——
            // 悄悄改成 200 才叫假成功。
            patch(|_u: crate::auth::AuthUser, _n: Path<String>| async {
                not_implemented("PATCH", "/api/extensions/mcp/{name}")
            })
            .delete(api_extensions::delete_mcp),
        )
        .route(
            "/api/extensions/skills",
            get(api_extensions::list_skills).post(api_extensions::save_skill),
        )
        .route(
            "/api/extensions/skills/{name}",
            // PATCH 只改启用开关。**这条路由此前是缺的**，于是界面上没有任何
            // 办法把一个技能从 `enabled=false` 打开 —— 停用的技能不挂进对话
            // 工具表，模型调不到，等于后端做完了而功能没出口。
            patch(api_extensions::update_skill).delete(api_extensions::delete_skill),
        )
        .route(
            "/api/extensions/plugins",
            get(|_u: crate::auth::AuthUser| async {
                not_implemented("GET", "/api/extensions/plugins")
            }),
        )
        .route(
            "/api/extensions/bundle/import",
            post(|_u: crate::auth::AuthUser| async {
                not_implemented("POST", "/api/extensions/bundle/import")
            }),
        )
        .route(
            "/api/extensions/bundle/export",
            get(|_u: crate::auth::AuthUser| async {
                not_implemented("GET", "/api/extensions/bundle/export")
            }),
        );

    let backup = Router::new()
        .route(
            "/api/backup/export",
            post(|_u: crate::auth::AuthUser| async {
                not_implemented("POST", "/api/backup/export")
            }),
        )
        .route(
            "/api/backup/restore",
            post(|_u: crate::auth::AuthUser| async {
                not_implemented("POST", "/api/backup/restore")
            }),
        )
        .route(
            "/api/backup/verify",
            post(|_u: crate::auth::AuthUser| async {
                not_implemented("POST", "/api/backup/verify")
            }),
        );

    let upgrade = Router::new()
        .route("/api/version", get(version))
        .route(
            "/api/upgrade/check",
            get(|_u: crate::auth::AuthUser| async { not_implemented("GET", "/api/upgrade/check") }),
        )
        .route(
            "/api/upgrade/prepare",
            post(|_u: crate::auth::AuthUser| async {
                not_implemented("POST", "/api/upgrade/prepare")
            }),
        )
        .route(
            "/api/upgrade/history",
            get(|_u: crate::auth::AuthUser| async {
                not_implemented("GET", "/api/upgrade/history")
            }),
        );

    let admin = Router::new()
        .route(
            "/api/admin/config",
            get(api_admin::get).put(api_admin::put),
        )
        .route(
            "/api/admin/providers",
            get(api_providers::list).post(api_providers::create),
        )
        .route(
            "/api/admin/providers/{id}",
            put(api_providers::update).delete(api_providers::delete),
        )
        .route(
            "/api/admin/providers/{id}/default",
            put(api_providers::set_default),
        )
        .route("/api/admin/providers/{id}/models", get(api_providers::models))
        .route("/api/admin/models", get(api_providers::pool));

    let authed_misc = Router::new()
        // refresh 必须带令牌：它轮换的是**调用方自己**那一行，
        // 公开的话等于任何人都能来续期别人的会话。
        .route("/api/auth/refresh", post(api_auth::refresh))
        .route("/api/auth/logout", post(api_auth::logout))
        .route("/api/auth/me", get(api_auth::me))
        .route(
            "/api/ws",
            get(|_u: crate::auth::AuthUser| async { not_implemented("GET", "/api/ws") }),
        );

    let spa = {
        let web_dir = state.config.web_dir.clone();
        Router::new().fallback(move |uri: axum::http::Uri| {
            let web_dir = web_dir.clone();
            async move {
                if is_api_path(uri.path()) {
                    return not_found(uri).await.into_response();
                }
                crate::ui::serve(&web_dir, uri).await
            }
        })
    };

    Router::new()
        .merge(public)
        .merge(users)
        .merge(experts)
        .merge(dispatch)
        .merge(teams)
        .merge(sessions)
        .merge(wiki)
        .merge(extensions)
        .merge(backup)
        .merge(upgrade)
        .merge(admin)
        .merge(authed_misc)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(crate::middleware::GuardLayer)
        .fallback_service(spa)
        .with_state(state)
}

/// 前端是单页应用，未知路径要回 index.html；
/// 但 `/api` 下未注册的路径必须继续报 404 JSON，否则前端拿到的不是它能解析的形状。
fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/") || path.starts_with("/__selftest__")
}

async fn healthz(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl axum::response::IntoResponse {
    use axum::Json;
    use serde_json::json;

    let storage = match (&state.db, &state.db_problem) {
        (Some(db), _) if !db.is_alive() => json!({
            "ready": false,
            "db_path": db.path(),
            "detail": "存储线程已退出：所有数据库调用都会失败，需要重启服务。",
        }),
        (Some(db), _) => match db.missing_tables() {
            Ok(missing) if missing.is_empty() => json!({
                "ready": true,
                "db_path": db.path(),
                "missing_tables": [],
            }),
            Ok(missing) => json!({
                "ready": false,
                "db_path": db.path(),
                "missing_tables": missing,
                "detail": "数据库可打开但 schema 未迁移，相关路由会返回 503。",
            }),
            Err(e) => json!({
                "ready": false,
                "db_path": db.path(),
                "detail": format!("探测 schema 失败：{e}"),
            }),
        },
        (None, Some(problem)) => json!({ "ready": false, "detail": problem }),
        (None, None) => json!({ "ready": false, "detail": "存储未装配且无原因记录" }),
    };
    let llm_cfg = state.llm_config_snapshot();
    let llm_configured = state
        .llm
        .read()
        .map(|g| g.is_some())
        .unwrap_or(false);
    let provider_cache = state.provider_cache_snapshot();
    let mut warnings = state
        .config
        .warnings
        .iter()
        .map(|w| json!({ "source": w.source, "message": w.message }))
        .collect::<Vec<_>>();
    if let Some(p) = provider_cache.default_provider() {
        if !p.enabled {
            warnings.push(json!({
                "source": "llm_providers",
                "message": format!(
                    "默认模型供应商「{}」已停用，聊天不可用。下一步：在模型管理页把它重新启用，\
                     或 PUT /api/admin/providers/{}/default 切到另一条。",
                    p.name, p.id
                ),
            }));
        }
    }
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "addr": state.config.addr.to_string(),
        "ui_assets_available": state.config.ui_assets_available(),
        "storage": storage,
        "llm": {
            "configured": llm_configured,
            "base_url": llm_cfg.base_url,
            "model": llm_cfg.model,
            "max_tokens": llm_cfg.max_tokens,
            "max_context_tokens": llm_cfg.max_context_tokens,
            "compaction_threshold_tokens": llm_cfg.compaction_threshold_tokens,
            "default_provider_id": provider_cache.default_provider_id(),
            "provider_count": provider_cache.providers.len(),
        },
        "warnings": warnings,
    }))
}

async fn version(_u: crate::auth::AuthUser) -> impl axum::response::IntoResponse {
    use axum::Json;
    use serde_json::json;
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn not_found(uri: axum::http::Uri) -> ApiError {
    ApiError::not_found(uri.path().to_string())
}

async fn method_not_allowed(method: axum::http::Method, uri: axum::http::Uri) -> ApiError {
    ApiError::method_not_allowed(method.to_string(), uri.path().to_string())
}

pub const CONTRACT_ROUTES: &[(&str, &str)] = &[
    ("POST", "/api/auth/login"),
    ("POST", "/api/auth/logout"),
    ("POST", "/api/auth/refresh"),
    ("GET", "/api/auth/me"),
    ("GET", "/api/users"),
    ("POST", "/api/users"),
    ("PATCH", "/api/users/{id}"),
    ("DELETE", "/api/users/{id}"),
    ("GET", "/api/experts"),
    ("POST", "/api/experts"),
    ("GET", "/api/experts/{slug}"),
    ("PATCH", "/api/experts/{slug}"),
    ("DELETE", "/api/experts/{slug}"),
    ("POST", "/api/experts/import"),
    ("GET", "/api/experts/export"),
    ("GET", "/api/teams"),
    ("POST", "/api/teams"),
    ("GET", "/api/teams/{id}"),
    ("PATCH", "/api/teams/{id}"),
    ("DELETE", "/api/teams/{id}"),
    ("GET", "/api/sessions"),
    ("POST", "/api/sessions"),
    ("GET", "/api/sessions/{id}"),
    ("DELETE", "/api/sessions/{id}"),
    ("POST", "/api/sessions/{id}/messages"),
    ("GET", "/api/sessions/{id}/metrics"),
    ("GET", "/api/sessions/{id}/context"),
    ("GET", "/api/usage"),
    ("GET", "/api/wiki/pages"),
    ("GET", "/api/wiki/pages/{path}"),
    ("POST", "/api/wiki/ingest"),
    ("POST", "/api/wiki/query"),
    ("POST", "/api/wiki/search"),
    ("GET", "/api/wiki/index"),
    ("GET", "/api/wiki/log"),
    ("GET", "/api/extensions/mcp"),
    ("POST", "/api/extensions/mcp"),
    ("PATCH", "/api/extensions/mcp/{name}"),
    ("DELETE", "/api/extensions/mcp/{name}"),
    ("GET", "/api/extensions/skills"),
    ("POST", "/api/extensions/skills"),
    ("PATCH", "/api/extensions/skills/{name}"),
    ("DELETE", "/api/extensions/skills/{name}"),
    ("GET", "/api/extensions/plugins"),
    ("POST", "/api/extensions/bundle/import"),
    ("GET", "/api/extensions/bundle/export"),
    ("POST", "/api/backup/export"),
    ("POST", "/api/backup/restore"),
    ("POST", "/api/backup/verify"),
    ("GET", "/api/version"),
    ("GET", "/api/upgrade/check"),
    ("POST", "/api/upgrade/prepare"),
    ("GET", "/api/upgrade/history"),
    ("GET", "/api/admin/config"),
    ("PUT", "/api/admin/config"),
    ("GET", "/api/admin/providers"),
    ("POST", "/api/admin/providers"),
    ("PUT", "/api/admin/providers/{id}"),
    ("DELETE", "/api/admin/providers/{id}"),
    ("PUT", "/api/admin/providers/{id}/default"),
    ("GET", "/api/admin/providers/{id}/models"),
    ("GET", "/api/admin/models"),
];

pub const EXTRA_ROUTES: &[(&str, &str)] = &[
    ("GET", "/healthz"),
    ("GET", "/api/ws"),
    ("GET", "/api/teams/{id}/dispatch"),
    ("POST", "/api/teams/{id}/dispatch"),
    ("GET", "/api/dispatch/inflight"),
];
