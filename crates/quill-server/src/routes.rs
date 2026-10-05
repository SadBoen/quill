//! 路由表 —— 逐条对齐 `docs/PHASE2_CONTRACT.md` §5.1。
//!
//! # 交付口径：登记 ≠ 实现
//!
//! 本 crate 交付的是**可启动的 HTTP 骨架**：每条契约路由都真实注册在 axum 上，
//! 因此「路径拼错」与「方法不对」会得到 404/405 而不是被兜底成 200。
//! 能力未实现的路由返回 **501 + 中文说明 + 下一步**，**绝不返回假成功**
//! （AGENTS.md 铁律二十八：静态检查不能替代实测；假成功比报错更危险）。
//!
//! # 为什么现在能返回 501 而不是等实现完再注册路由
//!
//! 契约 §5.1 的路由表是**已裁决**的权威口径（`check-doc-refs.sh` 校验权威链）。
//! 把表先落成真实路由，前端就能立刻对着真实状态码开发；
//! 若等实现完才注册，前端会对"一直 404"的路径写下错误的重试逻辑。

use axum::extract::Path;
use axum::routing::{get, patch, post};
use axum::Router;

use crate::api_dispatch;
use crate::api_experts;
use crate::error::ApiError;
use crate::state::AppState;

/// 未实现路由的统一响应。
///
/// ⚠️ 501 的文案必须**点名具体路由**（`method` + `path`），
/// 否则用户拿到"某功能未实现"却不知道是哪条 —— 那是不可诊断的失败。
fn not_implemented(method: &'static str, path: &'static str) -> ApiError {
    ApiError::NotImplemented { method, path }
}

/// 构造完整应用路由。
pub fn build_router(state: AppState) -> Router {
    // ── 免鉴权（契约 §5.1：认证「除 /api/auth/login」）──
    let mut public = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/auth/login", post(login_stub))
        .route("/api/auth/refresh", post(refresh_stub));

    // ⚠️ 诊断专用 panic 路由：**默认不存在**。
    //    存在的理由：500 兜底必须能被**真实进程**验证（铁律二十八：
    //    闸门/测试不能替代实测），而一个"故意 panic"的路由若默认开启，
    //    就等于一个可被外部触发的自我 DoS 开关。
    //    故由 `Config::enable_selftest` 控制（进程入口读 `QUILL_ENABLE_SELFTEST`），
    //    集成测试则可**按实例**精确开启，不污染并行用例。
    if state.config.enable_selftest {
        // ⚠️ 必须用**具名函数 + 显式返回类型**：`panic!` 是 diverging 表达式，
        //    只有当函数的返回类型**显式写出**时，`get()` 才能推出它实现
        //    `Handler`。写成 `|| async { panic!(..) }` 则依赖 never type 回落到
        //    `()`，实测被 `dependency_on_unit_never_type_fallback` 判红
        //    （且在 Rust 2024 会变成硬错误）—— 注释与实现不一致是最容易复发的一种错。
        async fn selftest_panic() -> axum::response::Response {
            panic!("自检用内部信息：不得泄漏到响应体");
        }
        public = public.route("/__selftest__/panic", get(selftest_panic));
    }

    // ── 需鉴权：用户管理（owner only）──
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

    // ── 需鉴权：专家（**已接真库**：quill-agent 专家领域 + quill-store 表）──
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

    // ── 需鉴权：派工账本（**契约外**路由，见 api_dispatch 模块注释）──
    let dispatch = Router::new()
        .route(
            "/api/teams/{id}/dispatch",
            get(api_dispatch::list_round).post(api_dispatch::book),
        )
        .route("/api/dispatch/inflight", get(api_dispatch::inflight));

    // ── 需鉴权：专家团 ──
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

    // ── 需鉴权：会话 ──
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

    // ── 需鉴权：资料库（V1 只做 ingest + query，禁向量检索）──
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

    // ── 需鉴权：扩展（约束 1：MCP/SKILL/插件配置必须存服务端）──
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

    // ── 需鉴权：备份（owner only，⚠️ restore 破坏性）──
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

    // ── 需鉴权：升级（V1 = 手动受控）──
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

    // ── 需鉴权：登出 + 自身信息 + WebSocket（§5.2 六帧协议尚未实现）──
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

    // ⚠️ 顺序说明：`merge` 而非 `nest` 是为了让路径保持**完整**，
    //    避免 `nest("/api", ...)` 之后逐条改写路径导致与契约 §5.1 不一致。
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
        // 未登记路径 → 404（中文 + 下一步），不静默兜底成 200。
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        // ⚠️ 兜底层放最外：所有路由（含 fallback）的 panic 都被捕获。
        //
        // ⚠️⚠️ **axum 的 `.layer()` 只覆盖它之前注册的路由**。
        //    本函数因此**必须**把 `.layer()` 放在**所有** `.merge()` / `.route()` 之后、
        //    `.with_state()` 之前 —— 顺序写反的话，新增路由会静默失去 panic 保护，
        //    而这种失效**没有任何编译期或运行期信号**（假闸门的典型形态）。
        //    `tests/http_contract.rs::panic_in_handler_becomes_500_and_leaks_nothing_internally`
        //    刻意在 `build_router` 之后追加路由并**自己补一层**，就是在测这个陷阱。
        .layer(crate::middleware::GuardLayer)
        .with_state(state)
}

/// `GET /healthz` —— 唯一的免鉴权 200。
///
/// ⚠️ **为什么把 `warnings` 暴露出来**：铁律四要求「静默不等于无痕」。
/// 配置回退（`QUILL_WEB_DIR` 不存在）只写日志的话，运维会误以为一切正常。
/// 这里让它对外可见。
async fn healthz(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl axum::response::IntoResponse {
    use axum::Json;
    use serde_json::json;
    // ⚠️ 存储状态**必须**在这里可见（铁律四：静默 ≠ 无痕）。
    //    「没建库」「建了但没迁移」「一切就绪」三者在屏幕上必须能区分 ——
    //    只报一个布尔值会把前两者压成同一个「不可用」，运维就会去看错方向。
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

/// `GET /api/version` —— 已实现：版本号是进程属性，不需要任何下游 crate。
async fn version(_u: crate::auth::AuthUser) -> impl axum::response::IntoResponse {
    use axum::Json;
    use serde_json::json;
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "contract": "docs/PHASE2_CONTRACT.md §5.1",
    }))
}

/// `GET /api/auth/me` —— 已实现：回显当前解析出的身份。
///
/// ⚠️ 这是**鉴权中间件的真实验证端点**：令牌对不对、admin 标记有没有落上，
/// 都必须能在这里观察到（铁律二十一：状态由工具给出，不由人转述）。
async fn me(user: crate::auth::AuthUser) -> impl axum::response::IntoResponse {
    use axum::Json;
    use serde_json::json;
    Json(json!({
        "user_id": user.0.user_id.to_compact_hex(),
        "is_admin": user.0.is_admin,
        // 契约 §5.1 §四：ExpertId 等为可读 slug；此处 UserId 为 128 位。
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

/// 契约 §5.1 全量路由清单（供测试断言"没有漏登记"）。
///
/// ⚠️ 把它写成**数据**而不是散落在各 handler 里，是为了让"契约里有的路由
/// 少了"能被一条测试发现，而不是靠人对着两份文档逐行核对。
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

/// 契约里**未**列出但本服务额外提供的路由（豁免必须显式登记，不能默默多）。
///
/// 铁律十二：豁免必须精确到具体项，且"豁免之外必须有反向断言"——
/// `tests/http_contract.rs` 会断言本清单之外的路径返回 404。
///
/// ⚠️ 三条派工路由**不在**契约 §5.1 的表里（该表只到 `/api/teams/{id}`）。
/// 理由与语义见 `api_dispatch` 模块注释：派工在契约里被归为 agent 域内部状态，
/// HTTP 只提供账本只读视图 + 预记账。
pub const EXTRA_ROUTES: &[(&str, &str)] = &[
    ("GET", "/healthz"),
    ("GET", "/api/ws"),
    ("GET", "/api/teams/{id}/dispatch"),
    ("POST", "/api/teams/{id}/dispatch"),
    ("GET", "/api/dispatch/inflight"),
];
