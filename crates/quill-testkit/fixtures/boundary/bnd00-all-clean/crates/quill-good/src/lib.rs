
use std::path::PathBuf;

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
