//! 🚨 本 fixture 的违规代码在 build.rs 里
//!
//! src/lib.rs 本身**不含任何禁用符号**，扫 src/ 会通过。

// ✅ 表面合规：注入式构造
pub fn data_dir(cfg: &Config) -> std::path::PathBuf { cfg.root.clone() }
pub struct Config { pub root: std::path::PathBuf }
