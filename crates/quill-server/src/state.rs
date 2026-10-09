use std::sync::{Arc, RwLock};

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

    /// 运行时 LLM provider。`Arc<RwLock<...>>` 让 admin PUT 后能在不重建
    /// AppState 的前提下替换正在使用的 provider（已有请求拿到的是旧 Arc，
    /// 新请求走新 Arc；旧 Arc 上的 inflight 请求跑完即释放）。
    pub llm: Arc<RwLock<Option<quill_provider::SharedProvider>>>,

    pub llm_config: Arc<RwLock<crate::llm::LlmConfig>>,

    /// `llm_providers` 的进程内副本。写路径成功后整体刷新；`/healthz` 与
    /// 「默认 provider → LlmConfig」的翻译都从这里读，不再回表。
    pub providers: Arc<RwLock<crate::llm_providers::ProviderCache>>,

    /// 登录端点的进程内滑动窗口。挂在 state 上（而不是全局 `OnceLock`），
    /// 这样每个测试实例有独立计数，不会互相污染。
    pub login_limiter: Arc<crate::ratelimit::RateLimiter>,

    /// 运行中成员的控制面（Q113）。**必须挂在 state 上**：生产里派工执行器是
    /// 每请求现建的（`api_dispatch::run`），注册表要是也跟着每请求一份，
    /// `POST /api/dispatch/{member}/steer|abort` 这类**另一个请求**就够不到
    /// 正在跑的成员。与 `login_limiter` 同一条理由不做全局 `OnceLock`
    /// （测试实例各自独立，不互相污染）。
    pub member_control: Arc<crate::member_control::MemberControl>,

    /// 口令哈希参数。生产走 `production()`（PBKDF2 60 万次，OWASP 当前推荐），
    /// 测试用 `for_tests()`。
    ///
    /// 做成字段而不是读环境变量，是刻意的：环境变量会在生产环境留下
    /// 「谁把迭代次数调低了」这条后门，而这里只是普通的依赖注入。
    pub pbkdf2: quill_control::Pbkdf2Params,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("addr", &self.config.addr)
            .field("warnings", &self.config.warnings)
            .field("db_path", &self.config.db_path)
            .field("storage_ready", &self.db.is_some())
            .field(
                "llm_ready",
                &self.llm.read().map(|g| g.is_some()).unwrap_or(false),
            )
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

    pub fn llm(&self) -> Result<quill_provider::SharedProvider, ApiError> {
        let guard = self
            .llm
            .read()
            .map_err(|_| ApiError::internal("LLM provider 读写锁被毒化（poisoned）"))?;
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| self.llm_unavailable())
    }

    /// 运行时没有 provider 的原因。默认 provider 被 `enabled=false` 停用是
    /// 一种**用户主动**的选择，报错必须点名是哪条被关了，而不是让人去查环境变量。
    fn llm_unavailable(&self) -> ApiError {
        if let Some(p) = self.default_provider_row() {
            if !p.enabled {
                return ApiError::service_unavailable(format!(
                    "默认模型供应商「{}」已停用，聊天不可用。\
                     下一步：在模型管理页把它重新启用，\
                     或 PUT /api/admin/providers/{}/default 切到另一条。",
                    p.name, p.id
                ));
            }
        }
        ApiError::service_unavailable(
            "本实例没有可用的模型服务。下一步：启动本地模型服务（例如 llama-server），\
             或用 QUILL_LLM_BASE_URL / QUILL_LLM_MODEL 指向一个 OpenAI 兼容端点后重启 quill-server；\
             也可以由 admin 通过 PUT /api/admin/config 在线配置。",
        )
    }

    /// 热替换当前 provider + 配置。写入只在请求处理路径上短暂持锁。
    pub fn replace_llm(
        &self,
        provider: Option<quill_provider::SharedProvider>,
        cfg: crate::llm::LlmConfig,
    ) {
        if let Ok(mut g) = self.llm.write() {
            *g = provider;
        }
        if let Ok(mut g) = self.llm_config.write() {
            *g = cfg;
        }
    }

    pub fn llm_config_snapshot(&self) -> crate::llm::LlmConfig {
        self.llm_config
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn provider_cache_snapshot(&self) -> crate::llm_providers::ProviderCache {
        self.providers.read().map(|g| g.clone()).unwrap_or_default()
    }

    /// 默认 provider 的整行（`/api/admin/config` 兼容路径与 set-default 判定用）。
    pub fn default_provider_row(&self) -> Option<crate::llm_providers::LlmProvider> {
        self.provider_cache_snapshot().default_provider().cloned()
    }

    /// 把默认 provider 翻译成运行时 `LlmConfig`。表里还没有 provider 时
    /// 返回 None（首启播种前 / 播种失败时都属正常）。
    pub fn default_provider_snapshot(&self) -> Option<crate::llm::LlmConfig> {
        self.default_provider_row().map(|p| p.to_llm_config())
    }

    pub fn set_provider_cache(&self, cache: crate::llm_providers::ProviderCache) {
        if let Ok(mut g) = self.providers.write() {
            *g = cache;
        }
    }

    /// 写路径收尾：重读表 → 刷新缓存 → 按新默认项热替换运行时 provider。
    ///
    /// 构造失败时**保留旧的运行时 provider** 并打日志：一次失败的管理操作
    /// 不该把正在服务的对话通道打瞎。
    pub fn reload_providers(&self) -> Result<crate::llm_providers::ProviderCache, ApiError> {
        let cache = crate::llm_providers::ProviderCache {
            providers: crate::llm_providers::list(self)?,
        };
        self.set_provider_cache(cache.clone());
        self.apply_default_provider(&cache);
        Ok(cache)
    }

    /// `enabled=false` 的默认 provider **不装进运行时**：清空 provider 槽位，
    /// 于是 `/healthz` 的 `llm.configured` 变 false、聊天直接 503，
    /// 与模型池把它剔除的行为一致。
    pub fn apply_default_provider(&self, cache: &crate::llm_providers::ProviderCache) {
        let Some(p) = cache.default_provider() else {
            eprintln!("[llm] 没有可用 provider：保持当前运行时配置不变");
            return;
        };
        let cfg = p.to_llm_config();
        if !p.enabled {
            eprintln!(
                "[llm] 默认 provider「{}」已停用：清空运行时 provider",
                p.name
            );
            self.replace_llm(None, cfg);
            return;
        }
        match crate::llm::build(&cfg) {
            Ok(provider) => self.replace_llm(Some(provider), cfg),
            Err(e) => eprintln!(
                "[llm] 默认 provider「{}」构造失败，继续沿用旧配置：{e}",
                p.name
            ),
        }
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
            llm: Arc::new(RwLock::new(None)),
            llm_config: Arc::new(RwLock::new(Default::default())),
            providers: Arc::new(RwLock::new(Default::default())),
            login_limiter: Arc::new(Default::default()),
            member_control: Default::default(),
            pbkdf2: quill_control::Pbkdf2Params::for_tests(),
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
            llm: Arc::new(RwLock::new(None)),
            llm_config: Arc::new(RwLock::new(Default::default())),
            providers: Arc::new(RwLock::new(Default::default())),
            login_limiter: Arc::new(Default::default()),
            member_control: Default::default(),
            pbkdf2: quill_control::Pbkdf2Params::for_tests(),
        };
        let err = s.db().expect_err("存储缺失必须报错");
        assert_eq!(err.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert!(err.detail().contains("/var/lib/quill/quill.db"));
        assert!(err.next_step().contains("quill doctor --section=db"));
    }

    #[test]
    fn missing_llm_is_reported_as_503_naming_the_knob_to_turn() {
        let s = AppState {
            config: Config::from_env(),
            tokens: std::sync::Arc::new(crate::auth::EnvTokenResolver::default()),
            db: None,
            db_problem: None,
            llm: Arc::new(RwLock::new(None)),
            llm_config: Arc::new(RwLock::new(Default::default())),
            providers: Arc::new(RwLock::new(Default::default())),
            login_limiter: Arc::new(Default::default()),
            member_control: Default::default(),
            pbkdf2: quill_control::Pbkdf2Params::for_tests(),
        };
        let err = s.llm().expect_err("没有模型服务必须报错");
        assert_eq!(err.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(err.code(), "provider_unavailable");
        assert!(
            err.next_step().contains("QUILL_LLM_BASE_URL"),
            "文案要点名要设的环境变量：{err}"
        );
        assert!(
            !err.next_step().contains("section=db"),
            "模型故障不能把人引到数据库：{err}"
        );
        assert!(s.db().is_err(), "对照组：没有存储时应当走 db 那条建议");
    }

    #[test]
    fn replace_llm_swaps_config_and_marks_provider_ready() {
        let s = AppState {
            config: Config::from_env(),
            tokens: std::sync::Arc::new(crate::auth::EnvTokenResolver::default()),
            db: None,
            db_problem: None,
            llm: Arc::new(RwLock::new(None)),
            llm_config: Arc::new(RwLock::new(crate::llm::LlmConfig {
                model: "old".into(),
                ..Default::default()
            })),
            providers: Arc::new(RwLock::new(Default::default())),
            login_limiter: Arc::new(Default::default()),
            member_control: Default::default(),
            pbkdf2: quill_control::Pbkdf2Params::for_tests(),
        };
        let new_cfg = crate::llm::LlmConfig {
            model: "new".into(),
            base_url: "http://example/v1".into(),
            ..Default::default()
        };
        // 不构造真的 SharedProvider — 仅验证"config 被替换 + provider slot
        // 被替换"两件事。集成语义（真的能 chat）在 http_contract.rs 里覆盖。
        s.replace_llm(None, new_cfg.clone());

        assert_eq!(s.llm_config_snapshot().model, "new");
        assert_eq!(s.llm_config_snapshot().base_url, "http://example/v1");
        // 故意没传 provider → 仍应走"未配置"分支。
        assert!(s.llm().is_err(), "provider 是 None 时必须报错");
        assert_eq!(s.llm().err().unwrap().code(), "provider_unavailable");
    }
}
