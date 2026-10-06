pub mod api_admin;
pub mod api_auth;
/// 备份：导出 / 校验 / 还原的 HTTP 接线。
pub mod api_backup;
pub mod api_chat;
pub mod api_dispatch;
/// 专家市场：从 SkillHub 技能集装成「我的专家」。
pub mod api_expert_market;
pub mod api_experts;
pub mod api_extensions;
pub mod api_providers;
pub mod api_teams;
/// 用户管理：列用户、启停账号。建号与删号**有意不做**，理由见该文件头。
pub mod api_users;
pub mod api_wiki;
pub mod auth;
pub mod body;
pub mod config;
pub mod db;
pub mod dispatch_ledger;
pub mod error;
pub mod experts_repo;
pub mod general_expert;
pub mod llm;
pub mod llm_providers;
pub mod mcp_client;
pub mod mcp_repo;
pub mod middleware;
/// 「路径在不在目录里」的唯一判定口径，见模块文档。
pub mod pathsafe;
pub mod ratelimit;
pub mod routes;
pub mod server;
pub mod session_metrics;
/// 技能市场的上游客户端（SkillHub）。
pub mod skillhub;
/// SkillHub 技能包的解包与安全上限。
pub mod skillhub_unpack;
pub mod skills_repo;
pub mod state;
pub mod teams_repo;
pub mod tools;
pub mod ui;

pub use config::Config;
pub use db::DbBridge;
pub use dispatch_ledger::{DispatchScope, SqlxDispatchLedger};
pub use error::ApiError;
pub use experts_repo::SqlxExpertRepository;
pub use state::AppState;
