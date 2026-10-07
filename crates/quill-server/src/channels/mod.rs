//! 外部消息通道（微信等）：把智能体接到浏览器之外。
//!
//! 模块分三层，各管一件事：
//! - [`store`] —— 落库。一行 = 一条命名通道，配置整体存 JSON。
//! - [`weixin`] —— iLink 协议客户端。**照公开协议实现，不是移植**：
//!   Octop 自己也是调 `octop_gateway.channels.weixin.login_qr` 这个外部包
//!   走同一套接口（`.octop-ref/octop/src/octop/infra/gateway/channels/qr_bind.py:102`）。
//! - `api_channels.rs`（在 crate 根）—— HTTP 线与长轮询后台任务。

pub mod store;
pub mod weixin;
