//! Quill 服务端 —— **依赖图的顶端**，本 crate 是唯一允许依赖全部 `quill-*` 的地方。
//!
//! # 职责
//!
//! 组装各 crate、注册 axum 路由、鉴权中间件、统一错误处理、优雅关闭。
//!
//! # ⚠️ 依赖方向铁律（契约 §一）
//!
//! 本 crate 依赖全部 `quill-*`；**任何其他 `quill-*` 禁止依赖它**（会成环）。
//! 该约束由 `scripts/check-crate-deps.sh`（G40）执行。
//!
//! # 模块地图
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`config`] | 环境变量装配；**配错回退 + WARN，绝不 panic**（铁律七） |
//! | [`error`] | 统一错误类型：中文人话 + 下一步该执行什么 |
//! | [`auth`] | 令牌 → `UserId`；401 不区分"用户不存在"与"令牌错误" |
//! | [`body`] | JSON 请求体提取器：把 axum 的英文拒绝謟成中文人话 |
//! | [`state`] | 构造函数注入的状态（**零 static**，契约 §2.5 规则 7 面 B） |
//! | [`db`] | 同步端口 × 异步 sqlx 的桥接：专用线程 + 连接池 |
//! | [`experts_repo`] | `ExpertRepository` 的真库实现（表 `experts`） |
//! | [`dispatch_ledger`] | `DispatchLedger` 的真库实现（表 `task_dispatches`） |
//! | [`api_experts`] | 专家 CRUD handler（契约 §5.1 的 5 条里实现 4 条） |
//! | [`api_dispatch`] | 派工账本 handler（**契约外**路由，见该模块注释） |
//! | [`middleware`] | 请求 ID + panic 兜底（500 不泄漏内部细节） |
//! | [`routes`] | 契约 §5.1 全量路由；未实现能力返回 **501**，不返回假成功 |
//!
//! # 交付边界（诚实声明）
//!
//! 契约 §5.1 的路由**全部真实注册**在 axum 上（路径错 → 404、方法错 → 405）。
//! 其中**已接真库实现**的是专家 CRUD 五条中的四条
//! （`GET/POST /api/experts`、`GET/PATCH/DELETE /api/experts/{slug}`），
//! 外加两条契约外的派工账本路由（见 [`api_dispatch`]）。
//! 其余返回 501 + 中文说明 + 下一步。
//! WebSocket 六帧协议（契约 §5.2）尚未实现，`GET /api/ws` 返回 501。
//!
//! ⚠️ **存储不启动时服务仍然起**：数据库打不开或 schema 未迁移时，
//! 相关路由返回 **503**（不是 500，更不是空列表假成功），
//! 启动横幅与 `/healthz` 的 `warnings` 同时可见（铁律七 + 铁律四）。

pub mod api_dispatch;
pub mod api_experts;
pub mod auth;
pub mod body;
pub mod config;
pub mod db;
pub mod dispatch_ledger;
pub mod error;
pub mod experts_repo;
pub mod middleware;
pub mod routes;
pub mod server;
pub mod state;

pub use config::Config;
pub use db::DbBridge;
pub use dispatch_ledger::{DispatchScope, SqlxDispatchLedger};
pub use error::ApiError;
pub use experts_repo::SqlxExpertRepository;
pub use state::AppState;
