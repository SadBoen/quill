//! 内置 MCP 服务器（照 goose 的 `builtin_extension.rs` +
//! `extension_manager/builtin.rs` 移植）。
//!
//! goose 把「随产品自带、不用装、不用配命令」的 MCP 服务器叫 builtin：它们跑在
//! **内存管道**上（`tokio::io::duplex`），同一个进程里既当服务器又当客户端 ——
//! 见 `vendor/goose/crates/goose/src/agents/extension_manager/builtin.rs:35-40`；
//! 「名字 → 起服务器的函数」那张注册表在
//! `vendor/goose/crates/goose/src/builtin_extension.rs:20-22`（memory 的登记处：
//! `vendor/goose/crates/goose-mcp/src/lib.rs:89-98`）：
//!
//! ```text
//! let (server_read, client_write) = tokio::io::duplex(65536);
//! let (client_read, server_write) = tokio::io::duplex(65536);
//! extension_fn(server_read, server_write);          // 服务器那一端
//! McpClient::connect((client_read, client_write))   // 客户端那一端
//! ```
//!
//! **quill 为什么要它。** `mcp_servers.transport` 的 CHECK 从第一天起就允许
//! `'builtin'`（`0001_init.sql`），但没人实现 —— `mcp_client` 一直把它当「还没铺」
//! 报。声明了做不到，正是本仓库最忌讳的那类；这一层把它补上。
//!
//! 顺带解决一件产品上的事：**记忆**（queue Q019 的产物）不必让用户手填一个
//! 「quill-cli 装在哪」的绝对路径，配一行 `transport=builtin, command=memory`
//! 就能用（queue Q111）。
//!
//! 名字写在 `command` 列里：那一列的语义是「跑什么」，而 builtin 的「什么」
//! 是**名字**而不是路径。

use tokio::io::DuplexStream;

/// 目前内置的服务器只有记忆一个。加新的就往这里加，并在 [`spawn`] 里接上。
pub const BUILTIN_SERVERS: &[&str] = &["memory"];

/// `command` 里写的名字认不认识。
pub fn is_builtin(name: &str) -> bool {
    BUILTIN_SERVERS.contains(&name)
}

/// 认识的名字清单，给错误信息用（`memory / …`）。
pub fn names() -> String {
    BUILTIN_SERVERS.join(" / ")
}

/// 在**内存管道**上起一个内置服务器，返回**客户端那一端**（读, 写）。
///
/// 名字不认识时 `None` —— 调用方如实报错并列出可用的名字，不猜一个出来。
pub fn spawn(name: &str) -> Option<(DuplexStream, DuplexStream)> {
    match name {
        "memory" => Some(crate::memory::serve_on_duplex()),
        _ => None,
    }
}
