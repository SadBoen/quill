//! JSON 请求体的取值助手。
//!
//! **为什么有这个模块**：`type_name` 曾在 `api_auth` / `api_experts` / `api_teams`
//! 里各写一份（三份逐字节相同）。这里是它唯一的归属；以后新的取值助手也放这里。
//!
//! **注意 `need_str` / `opt_str` 尚未收进来** —— 各处的版本**语义不同**：
//! - `api_auth::need_str` 会 `trim()` 并拒空串；`api_experts`/`api_teams` 连空串都收。
//! - `api_channels::opt_str` 报「{key} 必须是字符串」，另两处带「实际收到 {类型}」，
//!   `api_teams` 还多一句「或 null」。
//!
//! 硬合并会**悄悄改掉用户可见的校验行为与报错文案**，所以先只收无争议的 `type_name`。
//! 收口那两个要先定「哪一套是规范语义」，见 `BACKLOG.md`。

use serde_json::Value;

/// JSON 值的中文类型名，用在「实际收到 XX」这类报错里。
pub(crate) fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "布尔值",
        Value::Number(_) => "数字",
        Value::String(_) => "字符串",
        Value::Array(_) => "数组",
        Value::Object(_) => "对象",
    }
}
