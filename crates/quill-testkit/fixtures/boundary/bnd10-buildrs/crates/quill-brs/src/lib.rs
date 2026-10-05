
pub fn data_dir(cfg: &Config) -> std::path::PathBuf { cfg.root.clone() }
pub struct Config { pub root: std::path::PathBuf }
