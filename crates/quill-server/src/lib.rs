pub mod api_admin;
pub mod api_auth;
/// 备份：导出 / 校验 / 还原的 HTTP 接线。
pub mod api_backup;
pub mod api_bundle;
/// 外部消息通道：REST 线与长轮询后台任务。
pub mod api_channels;
pub mod api_chat;
pub mod api_chat_stream;
pub mod api_dispatch;
pub mod api_expert_market;
pub mod api_experts;
pub mod api_extensions;
/// 专家市场：从 SkillHub 技能集装成「我的专家」。
/// MBTI 人格：测评、四维光谱、应用到某个专家。
pub mod api_mbti;
pub mod api_providers;
pub mod api_teams;
/// 升级：查版本 / 升级前备份 / 升级历史。
pub mod api_upgrade;
/// 用户管理：列用户、启停账号。建号与删号**有意不做**，理由见该文件头。
pub mod api_users;
pub mod api_wiki;
pub mod auth;
pub mod body;
/// 外部消息通道：把智能体接到浏览器之外（微信等）。
pub mod channels;
/// 上下文压缩的壳侧接线（Q018）：注入 provider 与 token 估算，读
/// `compaction_threshold_tokens` 并据此决定是否压缩历史。
pub mod chat_compaction;
pub mod chat_repo;
pub mod config;
pub mod db;
pub mod dispatch_ledger;
pub mod error;
pub mod experts_repo;
pub mod general_expert;
pub mod jsonx;
/// 内核侧配置与 provider 组装（已搬到 `quill-core`；这里 re-export 保持旧路径可用）。
pub use quill_core::llm;
pub mod llm_providers;
/// MBTI 人格测评：题库、计分、落库、应用到专家人格正文。
pub mod mbti;
/// MCP 协议客户端（内核层，已搬到 `quill-core`；这里 re-export 保持旧路径可用）。
pub use quill_core::mcp_client;
pub mod mcp_repo;
/// 运行中成员的控制面：追加指令（steer）与中途取消（abort），照 goose 的
/// `Agent::steer` + `SteerOperation` 移植（queue Q023/Q024）。
pub mod member_control;
pub mod member_executor;
pub mod middleware;
/// 「路径在不在目录里」的唯一判定口径，见模块文档。
pub mod pathsafe;
pub mod ratelimit;
pub mod routes;
pub mod server;
pub mod session_metrics;
/// 技能市场的上游客户端（SkillHub）。实现在 `skillhub/` 子模块里。
pub mod skillhub;
/// SkillHub 技能包的解包与安全上限（实现在 `skillhub::unpack`；这里保留旧路径）。
pub use skillhub::unpack as skillhub_unpack;
pub mod skills_repo;
/// SSE 传输层：事件编码与「还没结束的响应体」。
pub mod sse;
pub mod state;
pub mod teams_repo;
/// 壳侧的 `ToolSources` 实现：把内核要的三份清单（专家 / SKILL / MCP）从既有
/// 查询函数里取出来；内核侧的定义在 `quill_core::tools`。
pub mod tool_sources;
/// 工具执行（内核层，已搬到 `quill-core`；这里 re-export 保持
/// `quill_server::tools::*` 旧路径全部可用）。纯逻辑（工具名 / 可见性 / 匹配 /
/// 渲染）照旧；`ToolRegistry::builtin*` / `with_skills` / `with_mcp_tools` 收的是
/// `ToolSources` 端口，壳侧实现见 `crate::tool_sources::DbToolSources`。
pub use quill_core::tools;
pub mod api_cron;
/// 定时任务的存储层（queue Q042）。
pub mod cron_repo;
/// 定时任务的调度器（queue Q042）：真会触发投递的那一半。
pub mod cron_scheduler;
pub mod ui;
/// 资料库的**模型侧接线**（queue Q057）：`KnowledgeBackend` 的唯一真实现，
/// 拿这一轮的 provider 调模型。`quill-wiki` 的 ingest / query 只依赖这个端口。
pub mod wiki_backend;

pub use config::Config;
pub use db::DbBridge;
pub use dispatch_ledger::{DispatchScope, SqlxDispatchLedger};
pub use error::ApiError;
pub use experts_repo::SqlxExpertRepository;
pub use state::AppState;
