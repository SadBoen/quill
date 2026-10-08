//! quill 的 **Agent 内核层**（最高指示第 3 条：照 `vendor/goose` 抄）。
//!
//! 这一层只放**与 HTTP 无关的 agent 逻辑**：MCP 协议客户端、工具执行、
//! provider 组装、对话循环。现状与目标见 `docs/ARCHITECTURE.md` §3.1 / §3.3：
//! 这些逻辑原先寄生在 `quill-server`（HTTP 层）里，导致内核**没法被单测、
//! 没法被 CLI 复用**。本 crate 是它们唯一的归属（queue Q011–Q015）。
//!
//! **搬运纪律**（`docs/ARCHITECTURE.md` §3.2 第一步）：先把散在 `quill-server`
//! 里的内核逻辑**原样搬进来**，行为一个字不改、测试一个不减；补 goose 缺失能力
//! （压缩 / 记忆 / 状态机）是第二步的事。
//!
//! **每个模块的头部都要写明它移植自 `vendor/goose` 的哪个文件**（Q016）。
//! 没写清楚出处的模块，等于把「照 goose 抄」这句话变成不可核对的口号。

pub mod mcp;
pub mod mcp_client;
