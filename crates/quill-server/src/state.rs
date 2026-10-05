
use std::sync::Arc;

use crate::auth::TokenResolver;
use crate::config::Config;
use crate::db::DbBridge;
use crate::error::ApiError;

#[derive(Clone)]
pub struct AppState {

    pub config: Config,

    pub tokens: std::sync::Arc<dyn TokenResolver>,

    pub db: Option<Arc<DbBridge>>,

    pub db_problem: Option<String>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {

        f.debug_struct("AppState")
            .field("addr", &self.config.addr)
            .field("warnings", &self.config.warnings)
            .field("db_path", &self.config.db_path)
            .field("storage_ready", &self.db.is_some())
            .finish_non_exhaustive()
    }
}

impl AppState {

    pub fn db(&self) -> Result<&Arc<DbBridge>, ApiError> {
        self.db.as_ref().ok_or_else(|| {
            ApiError::storage_unavailable(self.db_problem.clone().unwrap_or_else(|| {
                "存储不可用（原因未记录）。下一步：执行 `quill doctor --section=db` 查看诊断。"
                    .to_string()
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn debug_does_not_leak_resolver_internals() {
        let (tokens, _) = crate::auth::EnvTokenResolver::from_env();
        let s = AppState {
            config: Config::from_env(),
            tokens: std::sync::Arc::new(tokens),
            db: None,
            db_problem: Some("测试注入：未建库".to_string()),
        };
        let d = format!("{s:?}");
        assert!(!d.contains("entry"), "不应把令牌表内容打进 Debug");
    }

    #[test]
    fn missing_storage_is_reported_as_503_with_a_fix_command() {
        let s = AppState {
            config: Config::from_env(),
            tokens: std::sync::Arc::new(crate::auth::EnvTokenResolver::default()),
            db: None,
            db_problem: Some("数据库 /var/lib/quill/quill.db 打不开".to_string()),
        };
        let err = s.db().expect_err("存储缺失必须报错");
        assert_eq!(err.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.detail().contains("/var/lib/quill/quill.db"));
        assert!(err.next_step().contains("quill doctor --section=db"));
    }
}
