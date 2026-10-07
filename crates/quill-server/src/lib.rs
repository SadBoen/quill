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
/// 用户管理：列用户、启停账号。建号与删号**有意不做**，理由见该文件头。
pub mod api_users;
pub mod api_wiki;
pub mod auth;
pub mod body;
/// 外部消息通道：把智能体接到浏览器之外（微信等）。
pub mod channels;
pub mod chat_repo;
pub mod config;
pub mod db;
pub mod dispatch_ledger;
pub mod error;
pub mod experts_repo;
pub mod general_expert;
pub mod jsonx;
pub mod llm;
pub mod llm_providers;
/// MBTI 人格测评：题库、计分、落库、应用到专家人格正文。
pub mod mbti;
pub mod mcp_client;
pub mod mcp_repo;
pub mod member_executor;
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
/// SSE 传输层：事件编码与「还没结束的响应体」。
pub mod sse;
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
