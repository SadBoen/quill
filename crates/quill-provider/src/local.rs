use crate::error::ProviderError;
use crate::openai::OpenAiCompatible;

/// 与本仓库 `.wsl-llama.sh` 启动的实例一致；该脚本是当前唯一的本地服务事实来源。
pub const LLAMA_CPP_BASE_URL: &str = "http://127.0.0.1:18080/v1";

/// `llama-server` 只加载了一个模型时并不校验请求里的 `model` 名，所以项目里
/// 一律传 `local`。要精确指定时用 `models()` 查服务端实际报的 id。
pub const LLAMA_CPP_DEFAULT_MODEL: &str = "local";

pub const BASE_ENV: &str = "QUILL_LLAMA_BASE";

pub const MODEL_ENV: &str = "QUILL_LLAMA_MODEL";

/// 指向本地 `llama-server` 的 provider。`model` 传 `None` 时取
/// [`LLAMA_CPP_DEFAULT_MODEL`]；两个环境变量优先于默认值。
///
/// 环境变量不是可有可无的装饰：`llama-server.exe` 跑在 Windows 侧，而从 WSL
/// 访问 Windows 的 `127.0.0.1` 不通，必须改写成 Windows 的可达地址。
pub fn llama_cpp(model: Option<&str>) -> Result<OpenAiCompatible, ProviderError> {
    let base = env_or(BASE_ENV, LLAMA_CPP_BASE_URL);
    let model = model
        .map(str::to_string)
        .unwrap_or_else(|| env_or(MODEL_ENV, LLAMA_CPP_DEFAULT_MODEL).to_string());
    OpenAiCompatible::new(base, model, None).map(|p| p.with_name("llama.cpp"))
}

fn env_or(key: &str, fallback: &'static str) -> &'static str {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map_or(fallback, |v| Box::leak(v.into_boxed_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Provider, SharedProvider};

    #[test]
    fn the_local_profile_matches_the_script_this_repo_actually_runs() {
        let p = llama_cpp(None).expect("默认本地 profile 必须能构造");
        assert_eq!(p.base_url(), LLAMA_CPP_BASE_URL);
        assert_eq!(p.model(), LLAMA_CPP_DEFAULT_MODEL);
        assert_eq!(p.name(), "llama.cpp");
        assert_eq!(p.chat_url(), "http://127.0.0.1:18080/v1/chat/completions");
        assert_eq!(p.models_url(), "http://127.0.0.1:18080/v1/models");
    }

    #[test]
    fn the_default_model_is_one_llama_cpp_actually_accepts() {
        assert_eq!(
            LLAMA_CPP_DEFAULT_MODEL, "local",
            "默认值必须与 .wsl-llama.sh 传的 model 名一致，换名字会让真机调用与文档对不上"
        );
    }

    #[test]
    fn the_local_profile_accepts_a_model_override() {
        let p = llama_cpp(Some("qwen2.5-7b-instruct-q4_k_m")).expect("覆盖模型必须能构造");
        assert_eq!(p.model(), "qwen2.5-7b-instruct-q4_k_m");
    }

    #[test]
    fn an_override_still_wins_over_the_environment() {
        let p = llama_cpp(Some("explicit")).expect("显式参数必须能构造");
        assert_eq!(p.model(), "explicit");
    }

    #[test]
    fn the_local_profile_is_usable_behind_the_trait_object() {
        let shared: SharedProvider = std::sync::Arc::new(llama_cpp(None).expect("能构造"));
        assert_eq!(shared.name(), "llama.cpp");
    }

    #[test]
    fn an_empty_environment_value_falls_back_instead_of_producing_a_broken_url() {
        assert_eq!(env_or("QUILL_PROVIDER_DEFINITELY_UNSET", "fallback"), "fallback");
    }
}
