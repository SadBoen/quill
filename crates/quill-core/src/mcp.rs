//! MCP 服务器配置的**内核侧模型**。
//!
//! 为什么单独一个模块：这个结构原先定义在 `quill-server::mcp_repo`（存储层），
//! 但真正消费它的两端是**内核**（`mcp_client` 要拿它拉起进程）与**存储**
//! （`mcp_repo` 按列拼它）。内核搬进本 crate 后（queue Q014），结构体必须跟着
//! 内核走 —— 否则 `quill-core` 要反过来依赖 `quill-server` 才能编译，
//! 层次立刻倒挂。`mcp_repo` 现在从这里 re-export，路径不变。
//!
//! 移植出处：字段与语义照 `vendor/goose/crates/goose/src/agents/mcp_client.rs`
//! 的服务器配置结构（`McpServerConfig`）；quill 额外带了 `user_id` 隔离与
//! `asset_hash` 落库列，这两样是外壳（多用户）的要求，不属于内核原语。

/// 一行配置。字段与前端 `McpServerConfig` 一一对应（少 `enabled`，前端不暴露它）。
#[derive(Debug, Clone, PartialEq)]
pub struct McpServerRow {
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub enabled: bool,
    pub timeout_ms: i64,
    pub description: String,
    pub cwd: Option<String>,
    pub max_concurrent_calls: Option<i64>,
    /// `None` = 全禁，`Some(vec![])` = 全开，`Some(v)` = 精确列举。
    pub enabled_capabilities: Option<Vec<String>>,
    pub created_at: i64,
    pub updated_at: i64,
}
