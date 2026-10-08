use axum::extract::Path;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post, put};
use axum::Router;

use crate::api_admin;
use crate::api_auth;
use crate::api_backup;
use crate::api_channels;
use crate::api_chat;
use crate::api_chat_stream;
use crate::api_dispatch;
use crate::api_expert_market;
use crate::api_experts;
use crate::api_extensions;
use crate::api_mbti;
use crate::api_providers;
use crate::api_teams;
use crate::api_upgrade;
use crate::api_users;
use crate::error::ApiError;
use crate::state::AppState;

fn not_implemented(method: &'static str, path: &'static str) -> ApiError {
    ApiError::NotImplemented {
        method,
        path,
        advice: "执行 `GET /healthz` 确认服务存活；该路由随对应能力落地后自动转为可用，\
                 在此之前请不要在客户端里依赖它返回成功。",
    }
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
            get(api_users::list).post(|| async { api_users::create_not_allowed() }),
        )
        .route(
            "/api/users/{id}",
            patch(api_users::set_status)
                .delete(|id: Path<String>| async move { api_users::delete_not_allowed(&id) }),
        );

    let experts = Router::new()
        .route(
            "/api/experts",
            get(api_experts::list).post(api_experts::create),
        )
        .route("/api/experts/import", post(api_experts::import))
        .route("/api/experts/export", get(api_experts::export))
        .route("/api/experts/market", get(api_expert_market::list))
        .route(
            "/api/experts/market/{slug}/install",
            post(api_expert_market::install),
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
        // 真执行那一条。**与 book 分开**：`book` 只记账（响应明写 `executed:false`），
        // 这条才真的让每个成员各跑一次模型。见 api_dispatch::run 的文档注释。
        .route("/api/teams/{id}/dispatch/run", post(api_dispatch::run))
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
        .route("/api/sessions", get(api_chat::list).post(api_chat::create))
        .route(
            "/api/sessions/{id}",
            // PATCH = 改名（Q051）。此前只挂 GET/DELETE，PATCH 落进 405 ——
            // 界面上「重命名」没有后端出口。
            get(api_chat::get_one)
                .patch(api_chat::rename)
                .delete(api_chat::delete),
        )
        .route(
            "/api/sessions/{id}/messages",
            get(api_chat::list_messages).post(api_chat::post_message),
        )
        // 流式那条路：同一个「发一句话」，但增量边生成边返回。
        // 老路由一个字没改，两条路由共用 `api_chat::run_turn` 那一份循环。
        .route(
            "/api/sessions/{id}/messages/stream",
            post(api_chat_stream::stream_message),
        )
        .route("/api/sessions/{id}/metrics", get(api_chat::metrics))
        .route("/api/sessions/{id}/context", get(api_chat::context))
        .route("/api/usage", get(api_chat::usage));

    // 只读端点与「只读索引」的检索已接通（不需要模型）：pages / pages/{path} /
    // search / index / log。**页面的手写增删改也接通了**（PUT/DELETE pages/{path}，
    // 带 expected_version 乐观并发，同样不需要模型，见 Q058）。
    // **摄入与问答也接通了**（Q057）：POST /api/wiki/{ingest,query} 走
    // `wiki_backend::ProviderKnowledge` —— 模型产出内容，`quill-wiki` 校验/写盘/建索引。
    let wiki = Router::new()
        .route("/api/wiki/pages", get(crate::api_wiki::list_pages))
        .route(
            "/api/wiki/pages/{*path}",
            get(crate::api_wiki::get_page)
                .put(crate::api_wiki::put_page)
                .delete(crate::api_wiki::delete_page),
        )
        .route("/api/wiki/ingest", post(crate::api_wiki::ingest))
        .route("/api/wiki/query", post(crate::api_wiki::query_page))
        .route("/api/wiki/search", post(crate::api_wiki::search))
        .route("/api/wiki/index", get(crate::api_wiki::read_index))
        .route("/api/wiki/log", get(crate::api_wiki::read_log));

    // 定时任务（Q042）。形状对齐 octop 的 `api/routers/cron.py:109-231`
    // （list / create / get / patch / delete），字段与语义照前端
    // `ui/web/src/automations/api.ts` 写死的那份契约。
    // **注意**：octop 挂在 `/agents/{agent_id}/cron` 下，quill 挂在 `/api/cron` ——
    // 与本项目的通道同一取舍：agent 就是登录用户自己，多一层 agent_id 只会多一个能填错的入口。
    let cron = Router::new()
        .route(
            "/api/cron",
            get(crate::api_cron::list).post(crate::api_cron::create),
        )
        .route(
            "/api/cron/{id}",
            get(crate::api_cron::get_one)
                .patch(crate::api_cron::patch)
                .delete(crate::api_cron::remove),
        );

    let extensions = Router::new()
        .route(
            "/api/extensions/mcp",
            get(api_extensions::list_mcp).post(api_extensions::save_mcp),
        )
        .route(
            "/api/extensions/mcp/{name}",
            // 单条局部更新（缺省字段 = 不变）与改名都接上了：前端主流程仍是全量
            // POST，但「只改一个字段」以前没有任何出口，且改名只能靠删了重建。
            patch(api_extensions::patch_mcp).delete(api_extensions::delete_mcp),
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
        // 技能市场。上游是外部服务 SkillHub（默认 https://api.skillhub.cn，
        // 可用 QUILL_SKILLHUB_HOST 改），照抄 Octop 的实现与其安全上限。
        .route(
            "/api/extensions/skill-hub",
            get(api_extensions::skill_hub_list),
        )
        .route(
            "/api/extensions/skill-hub/{slug}/install",
            post(api_extensions::skill_hub_install),
        )
        // 单技能这一层。上游实测存在（2026-10-06）：
        // search / showcase / download 三个端点都返回 200，
        // 而技能包那层装出来的只是编排说明，**它点名的子技能没有正文**。
        .route(
            "/api/extensions/skill-hub/skills",
            get(api_extensions::skill_hub_search),
        )
        .route(
            "/api/extensions/skill-hub/rankings",
            get(api_extensions::skill_hub_rankings),
        )
        .route(
            "/api/extensions/skill-hub/skills/{slug}/install",
            post(api_extensions::skill_hub_install_skill),
        )
        // **刻意保持 501，不是漏做。** 前端 `ui/web/src/capabilityGaps.ts` 把这条
        // 登记成「已登记路由、处理函数未实现（501）」并在技能库里如实显示给用户。
        // quill 没有插件系统，也不打算有 —— 删掉这条路由会让前端那句「已登记」变成假话
        // （会退化成 404），所以留着一个**如实标注的缺口**比删掉更诚实。
        .route(
            "/api/extensions/plugins",
            get(|_u: crate::auth::AuthUser| async {
                not_implemented("GET", "/api/extensions/plugins")
            }),
        )
        .route(
            "/api/extensions/bundle/import",
            post(crate::api_bundle::import),
        )
        .route(
            "/api/extensions/bundle/export",
            get(crate::api_bundle::export),
        );

    // 备份。目标目录由服务端按相对名解析（见 api_backup）：
    // 浏览器不能指定落盘位置。`restore` **不在进程内做** ——
    // 服务正持有 SQLite 文件，它如实说明原因并给出停服后要跑的命令。
    let backup = Router::new()
        .route("/api/backup/export", post(api_backup::export))
        .route("/api/backup/restore", post(api_backup::restore))
        .route("/api/backup/verify", post(api_backup::verify));

    // 升级。三条都只允许 admin（口径与上面的备份一致）：`prepare` 会写出
    // 整份数据根的备份、`history` 会回显服务端绝对路径、`check` 会替服务端
    // 访问外部更新源。实现见 `api_upgrade.rs`。
    //
    // `prepare` **只落升级前备份，不执行升级**：换二进制与跑迁移是停服后
    // 的事，进程内没有可执行的升级步骤 —— 响应里 `upgraded: false` 说清这点。
    let upgrade = Router::new()
        .route("/api/version", get(version))
        .route("/api/upgrade/check", get(api_upgrade::check))
        .route("/api/upgrade/prepare", post(api_upgrade::prepare))
        .route("/api/upgrade/history", get(api_upgrade::history));

    let admin = Router::new()
        .route("/api/admin/config", get(api_admin::get).put(api_admin::put))
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
        .route(
            "/api/admin/providers/{id}/models",
            get(api_providers::models),
        )
        .route("/api/admin/models", get(api_providers::pool));

    let authed_misc = Router::new()
        // 通道：列表 / 建改 / 读 / 删，以及微信扫码那两步。
        // 路由形态对齐 Octop（api/routers/channels.py:118-241），
        // 差别是它挂在 /agents/{agent_id}/ 下而我们挂在 /api/ 下 ——
        // 本项目的 agent 就是登录用户自己，多一层 agent_id 只会
        // 让「这个 id 是不是我的」多出一个能填错的入口。
        .route(
            "/api/channels",
            get(api_channels::list).post(api_channels::upsert),
        )
        .route(
            "/api/channels/{id}",
            get(api_channels::get_one).delete(api_channels::remove),
        )
        .route(
            "/api/channels/weixin/qrcode/generate",
            post(api_channels::weixin_qr_generate),
        )
        .route(
            "/api/channels/weixin/qrcode/poll",
            post(api_channels::weixin_qr_poll),
        )
        // MBTI 人格：16 型档案、28 道题、测评历史、提交、应用到专家。
        // 端点对齐 Octop 的 mbti router（api/routers/mbti.py），差别见
        // api_mbti.rs 的文件头 —— 多一个 history（我们留历史），
        // apply 强制带 expert_id（人格挂在专家上）。
        .route("/api/mbti/types", get(api_mbti::types))
        .route("/api/mbti/questions", get(api_mbti::test_questions))
        .route("/api/mbti/history", get(api_mbti::history))
        .route("/api/mbti/test", post(api_mbti::submit))
        .route("/api/mbti/apply", post(api_mbti::apply_to_expert))
        // refresh 必须带令牌：它轮换的是**调用方自己**那一行，
        // 公开的话等于任何人都能来续期别人的会话。
        .route("/api/auth/refresh", post(api_auth::refresh))
        .route("/api/auth/logout", post(api_auth::logout))
        .route("/api/auth/me", get(api_auth::me));

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
        .merge(cron)
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
    let llm_configured = state.llm.read().map(|g| g.is_some()).unwrap_or(false);
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
    ("GET", "/api/experts/market"),
    ("POST", "/api/experts/market/{slug}/install"),
    ("GET", "/api/teams"),
    ("POST", "/api/teams"),
    ("GET", "/api/teams/{id}"),
    ("PATCH", "/api/teams/{id}"),
    ("DELETE", "/api/teams/{id}"),
    ("GET", "/api/sessions"),
    ("POST", "/api/sessions"),
    ("GET", "/api/sessions/{id}"),
    // 会话改名（Q051 接通）。octop 侧确有 `PATCH .../threads/{tid}`，所以属契约路由。
    // 此前漏登记（Q110），清单与路由表对不上。
    ("PATCH", "/api/sessions/{id}"),
    ("DELETE", "/api/sessions/{id}"),
    ("POST", "/api/sessions/{id}/messages"),
    ("GET", "/api/sessions/{id}/metrics"),
    ("GET", "/api/sessions/{id}/context"),
    ("GET", "/api/usage"),
    ("GET", "/api/wiki/pages"),
    ("GET", "/api/wiki/pages/{path}"),
    // 手写页面的增删改（Q058）。前端 `ui/web/src/memory/api.ts` 的
    // `WIKI_WRITE_ROUTE` 就是 `PUT /api/wiki/pages/{path}` —— 这条契约先在前端定下，
    // 后端这次把它接上。两条都带 `expected_version` 乐观并发。
    ("PUT", "/api/wiki/pages/{path}"),
    ("DELETE", "/api/wiki/pages/{path}"),
    ("POST", "/api/wiki/ingest"),
    ("POST", "/api/wiki/query"),
    ("POST", "/api/wiki/search"),
    ("GET", "/api/wiki/index"),
    ("GET", "/api/wiki/log"),
    // 定时任务（Q042，2026-10-09 接通）。octop 的形状是 `/agents/{agent_id}/cron`
    // （`cron.py:109-231`），quill 去掉了 agent 前缀（同通道的取舍）。
    ("GET", "/api/cron"),
    ("POST", "/api/cron"),
    ("GET", "/api/cron/{id}"),
    ("PATCH", "/api/cron/{id}"),
    ("DELETE", "/api/cron/{id}"),
    ("GET", "/api/extensions/mcp"),
    ("POST", "/api/extensions/mcp"),
    ("PATCH", "/api/extensions/mcp/{name}"),
    ("DELETE", "/api/extensions/mcp/{name}"),
    ("GET", "/api/extensions/skills"),
    ("POST", "/api/extensions/skills"),
    // 技能市场。上游是外部服务，**这两条不是「本地实现」** ——
    // 断网时它会 503，而这不代表本地技能坏了。
    ("GET", "/api/extensions/skill-hub"),
    ("POST", "/api/extensions/skill-hub/{slug}/install"),
    // 技能市场的**单技能**这一层。与上面那两条不是同一个东西：
    // 技能包装的是编排说明，单技能才是能直接用的技能正文。
    ("GET", "/api/extensions/skill-hub/skills"),
    ("GET", "/api/extensions/skill-hub/rankings"),
    ("POST", "/api/extensions/skill-hub/skills/{slug}/install"),
    ("PATCH", "/api/extensions/skills/{name}"),
    ("DELETE", "/api/extensions/skills/{name}"),
    ("GET", "/api/extensions/plugins"),
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
    // 流式发消息。**不是 octop 的契约路由**（上游那条还是一次性返回），
    // 是我们为了让长回复能边写边看自己加的，所以登记在 EXTRA 而不是 CONTRACT。
    ("POST", "/api/sessions/{id}/messages/stream"),
    ("GET", "/api/teams/{id}/dispatch"),
    ("POST", "/api/teams/{id}/dispatch"),
    // 派工**真执行**。自加的路由（octop 的派工只登记不跑），所以进 EXTRA。
    ("POST", "/api/teams/{id}/dispatch/run"),
    ("GET", "/api/dispatch/inflight"),
    // 设置包导出/导入（需求 4 多端同步）。**不是 octop 契约路由** ——
    // 上游 api/routers 与 dashboard 里都没有 bundle 这个形状，逐字查过。
    // 它是 quill 自加的，却长期登记在 CONTRACT 里，是分类错位，本轮移到 EXTRA。
    ("POST", "/api/extensions/bundle/import"),
    ("GET", "/api/extensions/bundle/export"),
];
