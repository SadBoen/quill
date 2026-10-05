//! ✅ 合法：include! 目标在 crates/ 内，且**不含**违规代码
//!
//! ⚠️ 修正记录（重要）：
//! 初版写的是 `include!("../shared.rs")`，但 shared.rs 就在**同目录** src/ 下，
//! `../shared.rs` 会指向 `crates/quill-inc-legal/shared.rs`（**不存在**）。
//! G26-3 判它"逃出 crates/"是对的 —— **是我的 fixture 写错了**。
//!
//! 这条与 devops 的经历同构：他的自检报"闸门误报"，诊断后是**测试用例写错**。
//! 当"闸门误报"与"测试写错"都可能时，**先诊断再改闸门** ——
//! 否则会为了迁就错误的测试用例而削弱真实的防护。

/// 合法：include! 目标就在同目录，仍在 crates/ 内
include!("shared.rs");

mod inline_mod;

/// ✅ 注入式构造，不使用全局单例
pub fn data_dir(cfg: &Config) -> std::path::PathBuf { cfg.root.clone() }

pub struct Config { pub root: std::path::PathBuf }

pub mod inline_mod_impl { pub fn x() -> u32 { 1 } }
use inline_mod_impl as inline_mod;
