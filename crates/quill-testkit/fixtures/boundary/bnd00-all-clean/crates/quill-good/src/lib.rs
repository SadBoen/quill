//! ✅ 正向 fixture：使用【注入式】构造路径
//!
//! 🚫 绝不能写成 `Config::global()` —— 单进程多用户方案依赖
//! 「只用可注入的构造路径」，用全局单例会让所有用户共享一份配置。
//! （见 docs/08_测试与验收方案.md §2.1.5）

use std::path::PathBuf;

/// per-user 路径由调用方注入，而非从全局单例取。
pub fn user_workspace(cfg: &GooseConfig, user: &UserId) -> PathBuf {
    cfg.data_dir.join(format!("users/{user}"))
}

pub struct UserId(pub String);

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct GooseConfig {
    /// ✅ 注入的 data_dir，不是全局单例
    pub data_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn per_user_paths_are_independent() {
        let cfg = GooseConfig { data_dir: PathBuf::from("/data") };
        let a = user_workspace(&cfg, &UserId("u1".into()));
        let b = user_workspace(&cfg, &UserId("u2".into()));
        assert_ne!(a, b, "每用户工作区必须独立");
    }
}
