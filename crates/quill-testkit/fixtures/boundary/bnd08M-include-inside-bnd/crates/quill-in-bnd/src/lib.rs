//! ✅ 合法：include! 目标**仍在 crates/ 内**（只回退到 crates 层）
//!
//! src/ → ../../ → crates/ → elsewhere
//! ⇒ crates/elsewhere/helper.rs —— 仍在边界内 → 必须放行

/// ✅ 真实调用（无字符串诱饵），但因在 crates/ 内而合法
include!("../../elsewhere/helper.rs");

/// ✅ 注入式构造
pub fn data_dir(cfg: &Config) -> std::path::PathBuf { cfg.root.clone() }
pub struct Config { pub root: std::path::PathBuf }
