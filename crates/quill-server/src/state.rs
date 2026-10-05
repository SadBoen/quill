//! 应用状态 —— 经**构造函数注入**进入路由，不走任何全局单例。
//!
//! # 边界规则 7 面 B（契约 §2.5）
//!
//! 「状态经构造函数注入」是多用户隔离成立的前提。一旦有人把状态写回
//! `static` / `OnceLock`，所有用户共享同一份 → 运行时静默串数据，
//! 且只在特定并发下显现（`scripts/check-boundary-singletons.sh` 拦的是
//! 上游单例 API 调用，本文件是同一原则的自我约束）。
//!
//! 因此：`AppState` 只经 `Router::with_state` 进入，**本文件零 `static`**。
//!
//! # 存储为什么也是注入的字段
//!
//! [`DbBridge`]（数据库桥接）同样是**按实例**持有的：它内部有连接池与一条
//! 工作线程，放进 `static` 就等于给所有请求共享一条「谁都能关」的连接 ——
//! 且测试无法按实例隔离（铁律十四：环境/全局态会让「通过」失去含义）。
//!
//! 打不开数据库时**不 panic**（铁律七）：`db` 为 `None`、
//! `db_problem` 带中文原因与修复命令，由 [`AppState::db`] 统一转成 503。

use std::sync::Arc;

use crate::auth::TokenResolver;
use crate::config::Config;
use crate::db::DbBridge;
use crate::error::ApiError;

/// 服务状态。
#[derive(Clone)]
pub struct AppState {
    /// 启动配置。
    pub config: Config,
    /// 令牌解析器（`quill-control` 落地后换实现，路由层不动）。
    pub tokens: std::sync::Arc<dyn TokenResolver>,
    /// 数据库桥接；`None` = 打不开（原因在 `db_problem`）。
    pub db: Option<Arc<DbBridge>>,
    /// 存储不可用的中文原因与修复命令（`db` 为 `Some` 时必为 `None`）。
    pub db_problem: Option<String>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // ⚠️ 刻意不打 tokens 的内部内容。
        f.debug_struct("AppState")
            .field("addr", &self.config.addr)
            .field("warnings", &self.config.warnings)
            .field("db_path", &self.config.db_path)
            .field("storage_ready", &self.db.is_some())
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// 取数据库桥接；不可用时返回 **503**（而不是 500，更不是假成功）。
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

    /// ⚠️ 反向用例：存储缺失必须**显式报 503**。
    /// 若它悄悄返回 200 空列表，用户会以为「我真的没有专家」——
    /// 那是把故障伪装成数据（第 2 类「假成功」）。
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
