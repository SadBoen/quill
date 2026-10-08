//! 配置加载时的一条「说人话的」警告（`source` + `message`）。
//!
//! **为什么在内核**：内核自己也有配置 —— `crate::llm` 从 `QUILL_LLM_*` 读模型端点
//! （这与 goose 把 config 放在内核 crate 里同一层：`vendor/goose/crates/goose/src/config/`）。
//! 警告类型必须跟着配置走，否则内核要为了一个两字段的结构体反向依赖 `quill-server`
//! （层次倒挂）。壳侧的 `quill-server::config::Warning` 是本类型的 re-export，
//! 既有调用点与 `Display` 行为都不变。

/// 一条配置警告。`Display` 的形状是 `source: message`，被 doctor / 启动日志逐条打印。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub source: String,

    pub message: String,
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.source, self.message)
    }
}
