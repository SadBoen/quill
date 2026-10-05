//! 🚨 本 fixture 故意使用全局单例（违反 G26）
//!
//! 单进程多用户要求「只用可注入的构造路径」。用全局单例会让
//! 所有用户共享同一份配置与 data_dir —— 跨用户数据泄漏。
//! 见 docs/08_测试与验收方案.md §2.1.5

/// 🚨 违规：全局单例
pub fn bad_data_dir() -> String {
    // 假设 goose::Config 有一个 global() 入口
    format!("{:?}", goose_config_global())
}

fn goose_config_global() -> &'static str {
    "Config::global()"
}
