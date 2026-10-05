
use axum::extract::Path;
use axum::routing::{get, patch, post};
use axum::Router;

use crate::api_dispatch;
use crate::api_experts;
use crate::error::ApiError;
use crate::state::AppState;

fn not_implemented(method: &'static str, path: &'static str) -> ApiError {
    ApiError::NotImplemented { method, path }
}

pub fn build_router(state: AppState) -> Router {

    let mut public = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/auth/login", post(login_stub))
        .route("/api/auth/refresh", post(refresh_stub));

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
        .route(
            "/api/teams",
            get(|_u: crate::auth::AuthUser| async { not_implemented("GET", "/api/teams") })
                .post(|_u: crate::auth::AuthUser| async { not_implemented("POST", "/api/teams") }),
        )
        .route(
            "/api/teams/{id}",
            get(|_u: crate::auth::AuthUser, _i: Path<String>| async {
                not_implemented("GET", "/api/teams/{id}")
            })
            .patch(|_u: crate::auth::AuthUser, _i: Path<String>| async {
                not_implemented("PATCH", "/api/teams/{id}")
            })
            .delete(|_u: crate::auth::AuthUser, _i: Path<String>| async {
                not_implemented("DELETE", "/api/teams/{id}")
            }),
        );

    let sessions = Router::new()
        .route(
            "/api/sessions",
            get(|_u: crate::auth::AuthUser| async { not_implemented("GET", "/api/sessions") })
                .post(|_u: crate::auth::AuthUser| async {
                    not_implemented("POST", "/api/sessions")
                }),
        )
        .route(
            "/api/sessions/{id}",
            get(|_u: crate::auth::AuthUser, _i: Path<String>| async {
                not_implemented("GET", "/api/sessions/{id}")
            })
            .delete(|_u: crate::auth::AuthUser, _i: Path<String>| async {
                not_implemented("DELETE", "/api/sessions/{id}")
            }),
        )
        .route(
            "/api/sessions/{id}/messages",
            post(|_u: crate::auth::AuthUser, _i: Path<String>| async {
                not_implemented("POST", "/api/sessions/{id}/messages")
            }),
        );

    let wiki = Router::new()
        .route(
            "/api/wiki/pages",
            get(|_u: crate::auth::AuthUser| async { not_implemented("GET", "/api/wiki/pages") }),
        )
        .route(
            "/api/wiki/pages/{*path}",
            get(|_u: crate::auth::AuthUser, _p: Path<String>| async {
                not_implemented("GET", "/api/wiki/pages/{path}")
            }),
        )
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
        .route(
            "/api/wiki/index",
            get(|_u: crate::auth::AuthUser| async { not_implemented("GET", "/api/wiki/index") }),
        )
        .route(
            "/api/wiki/log",
            get(|_u: crate::auth::AuthUser| async { not_implemented("GET", "/api/wiki/log") }),
        );

    let extensions = Router::new()
        .route(
            "/api/extensions/mcp",
            get(|_u: crate::auth::AuthUser| async {
                not_implemented("GET", "/api/extensions/mcp")
            })
            .post(|_u: crate::auth::AuthUser| async {
                not_implemented("POST", "/api/extensions/mcp")
            }),
        )
        .route(
            "/api/extensions/mcp/{name}",
            patch(|_u: crate::auth::AuthUser, _n: Path<String>| async {
                not_implemented("PATCH", "/api/extensions/mcp/{name}")
            })
            .delete(|_u: crate::auth::AuthUser, _n: Path<String>| async {
                not_implemented("DELETE", "/api/extensions/mcp/{name}")
            }),
        )
        .route(
            "/api/extensions/skills",
            get(|_u: crate::auth::AuthUser| async {
                not_implemented("GET", "/api/extensions/skills")
            })
            .post(|_u: crate::auth::AuthUser| async {
                not_implemented("POST", "/api/extensions/skills")
            }),
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

    let authed_misc = Router::new()
        .route(
            "/api/auth/logout",
            post(|_u: crate::auth::AuthUser| async { not_implemented("POST", "/api/auth/logout") }),
        )
        .route("/api/auth/me", get(me))
        .route(
            "/api/ws",
            get(|_u: crate::auth::AuthUser| async { not_implemented("GET", "/api/ws") }),
        );

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
        .merge(authed_misc)

        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)

        .layer(crate::middleware::GuardLayer)
        .with_state(state)
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
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "addr": state.config.addr.to_string(),
        "ui_assets_available": state.config.ui_assets_available(),
        "storage": storage,
        "warnings": state.config.warnings.iter()
            .map(|w| json!({ "source": w.source, "message": w.message }))
            .collect::<Vec<_>>(),
    }))
}

async fn version(_u: crate::auth::AuthUser) -> impl axum::response::IntoResponse {
    use axum::Json;
    use serde_json::json;
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "contract": "docs/PHASE2_CONTRACT.md §5.1",
    }))
}

async fn me(user: crate::auth::AuthUser) -> impl axum::response::IntoResponse {
    use axum::Json;
    use serde_json::json;
    Json(json!({
        "user_id": user.0.user_id.to_compact_hex(),
        "is_admin": user.0.is_admin,

        "approval_mode": "manual",
    }))
}

async fn login_stub() -> ApiError {
    not_implemented("POST", "/api/auth/login")
}

async fn refresh_stub() -> ApiError {
    not_implemented("POST", "/api/auth/refresh")
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
];

pub const EXTRA_ROUTES: &[(&str, &str)] = &[
    ("GET", "/healthz"),
    ("GET", "/api/ws"),
    ("GET", "/api/teams/{id}/dispatch"),
    ("POST", "/api/teams/{id}/dispatch"),
    ("GET", "/api/dispatch/inflight"),
];
