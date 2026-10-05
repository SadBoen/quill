//! 🚨 本 fixture 用 include! 把违规代码藏在 crates/ 之外
//!
//! lib.rs 本身**不含任何禁用符号**，纯 grep 扫它会通过。
//! 但编译后违规代码进入二进制。

mod inner;

/// 🚨 逃逸点：include! 的目标在 crates/ 之外
include!("../../../elsewhere/hidden.rs");

pub use hidden_singleton;
