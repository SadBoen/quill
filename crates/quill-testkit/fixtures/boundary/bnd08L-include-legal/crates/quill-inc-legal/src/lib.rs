
include!("shared.rs");

mod inline_mod;

pub fn data_dir(cfg: &Config) -> std::path::PathBuf { cfg.root.clone() }

pub struct Config { pub root: std::path::PathBuf }

pub mod inline_mod_impl { pub fn x() -> u32 { 1 } }
use inline_mod_impl as inline_mod;
