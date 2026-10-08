//! JSON 请求体的取值助手。
//!
//! **为什么有这个模块**：`type_name` 曾在 `api_auth` / `api_experts` / `api_teams`
//! 里各写一份（三份逐字节相同）；`need_str` / `opt_str` / `req_str` 更是在
//! `api_auth` / `api_bundle` / `api_dispatch` / `api_experts` / `api_teams` /
//! `api_channels` 六个文件里各有一份，且语义互不相同（有的 trim、有的不 trim，
//! 报错文案也不一样）。这里是它们**唯一**的归属；以后新的取值助手也放这里。
//!
//! 收口时定下的规范语义见 queue Q007 / `BACKLOG.md` B0-4：
//! - [`need_str`]：必填，`trim()` 后非空才收，返回值是 trim 过的；
//! - [`opt_str`]：可选，字符串**原样**返回（不 trim、不拒空串）。
//!
//! 统一到这个语义是**有意的行为变更**：此前 `api_experts` / `api_teams` 的
//! 副本接受空白必填串，`api_bundle` / `api_dispatch` 的类型错文案不带实际类型，
//! `api_channels` 的 `req_str` / `opt_str` 报错既不带 `where_` 定位也不带类型。
//! 现在一律更严格、更好定位。

use serde_json::Value;

use crate::error::ApiError;

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

/// 必填字符串（规范语义，见 queue Q007 / `BACKLOG.md` B0-4）。
///
/// - 是字符串且 `trim()` 后非空 → `Ok(trim 后的 String)`；
/// - 是字符串但 `trim()` 后为空 → 400 `{where_}.{key} 不能为空。`；
/// - 不是字符串 → 400 `{where_}.{key} 必须是字符串，实际收到 {类型}。`；
/// - 字段缺失 → 400 `缺少必填字段 {where_}.{key}。`。
///
/// `where_` 是「能指到端点」的短标签（如 `"登录请求"`、`"建团请求"`）；
/// 数组元素用 `format!("members[{i}]")` 这种写法（与 `api_bundle` 一致）。
///
/// **这是有意的行为变更**：此前 `api_experts` / `api_teams` 的副本连空白必填串
/// 都收（不 trim），`api_bundle` / `api_dispatch` 的类型错文案不带实际类型。
/// 现在统一为上面这套 —— 更严格，且报错一律带 `{where_}.{key}` 定位。
pub(crate) fn need_str(body: &Value, key: &str, where_: &str) -> Result<String, ApiError> {
    match body.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        Some(Value::String(_)) => Err(ApiError::bad_request(format!("{where_}.{key} 不能为空。"))),
        Some(other) => Err(ApiError::bad_request(format!(
            "{where_}.{key} 必须是字符串，实际收到 {}。",
            type_name(other)
        ))),
        None => Err(ApiError::bad_request(format!(
            "缺少必填字段 {where_}.{key}。"
        ))),
    }
}

/// 可选字符串（规范语义，见 queue Q007 / `BACKLOG.md` B0-4）。
///
/// - 缺失或 `null` → `Ok(None)`；
/// - 是字符串 → `Ok(Some(原样，不 trim、不拒空串))`；
/// - 否则 → 400 `{where_}.{key} 必须是字符串，实际收到 {类型}。`。
///
/// 「不 trim、不拒空串」是有意的：可选字段的空白串该被当成「清除」还是
/// 「非法」，得由调用方按各自契约决定，助手不替它做主张。
///
/// **这是有意的行为变更**：此前 `api_channels::opt_str` 的报错不带类型名，
/// `api_teams::opt_str` 的文案是「必须是字符串或 null」。现在统一为上面这套，
/// 报错带 `{where_}.{key}` 定位与真实类型。
pub(crate) fn opt_str(body: &Value, key: &str, where_: &str) -> Result<Option<String>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(ApiError::bad_request(format!(
            "{where_}.{key} 必须是字符串，实际收到 {}。",
            type_name(other)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use serde_json::json;

    /// 钉住「trim」：把实现改回不 trim（例如直接 `Ok(s.clone())`），
    /// `"  alice  "` 的断言立刻变红。
    #[test]
    fn need_str_trims_surrounding_whitespace() {
        let body = json!({ "username": "  alice  " });
        let got = need_str(&body, "username", "登录请求").expect("空白包裹的名字应被接受");
        assert_eq!(got, "alice", "必填串必须 trim 后再返回");

        let body = json!({ "name": "\t增长小队\n" });
        let got = need_str(&body, "name", "建团请求").expect("制表符/换行包裹也应被接受");
        assert_eq!(got, "增长小队");
    }

    /// 钉住「trim 后为空即拒」：`api_experts` / `api_teams` 的旧副本收下这些值，
    /// 语义改回去这条就红。
    #[test]
    fn need_str_rejects_a_blank_required_string() {
        for blank in ["", "   ", "\t\n"] {
            let body = json!({ "name": blank });
            let err = need_str(&body, "name", "建团请求").expect_err("空白必填串必须判红");
            assert_eq!(err.status(), StatusCode::BAD_REQUEST);
            assert_eq!(err.code(), "bad_request");
            assert_eq!(err.detail(), "建团请求.name 不能为空。");
        }
    }

    /// 钉住「类型错报出实际类型」：漏掉 `type_name` 这条就红。
    #[test]
    fn need_str_names_the_received_type_on_a_type_error() {
        let err =
            need_str(&json!({ "room_id": 7 }), "room_id", "派工请求").expect_err("数字必须判红");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            err.detail(),
            "派工请求.room_id 必须是字符串，实际收到 数字。"
        );

        let err = need_str(&json!({ "room_id": null }), "room_id", "派工请求")
            .expect_err("null 不是字符串，必须判红");
        assert_eq!(
            err.detail(),
            "派工请求.room_id 必须是字符串，实际收到 null。"
        );
    }

    /// 钉住缺失分支的完整路径：`where_` 与 `key` 都要出现在 detail 里。
    #[test]
    fn need_str_reports_a_missing_field_with_its_full_path() {
        let err = need_str(&json!({}), "password", "注册请求").expect_err("缺失必须判红");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert_eq!(err.detail(), "缺少必填字段 注册请求.password。");
    }

    /// `opt_str` 的 null / 缺失都归为 `None`（旧副本就是这个语义，
    /// 这里钉住它别在收口时被改掉）。
    #[test]
    fn opt_str_maps_missing_and_null_to_none() {
        assert_eq!(
            opt_str(&json!({}), "model", "专家请求").expect("缺失应 OK"),
            None
        );
        assert_eq!(
            opt_str(&json!({ "model": null }), "model", "专家请求").expect("null 应 OK"),
            None
        );
    }

    /// 钉住「可选串原样返回」：擅自 trim 或拒空串，这条变红。
    #[test]
    fn opt_str_keeps_an_empty_string_untrimmed_and_accepted() {
        let got = opt_str(&json!({ "description": "" }), "description", "专家请求")
            .expect("空串应被接受");
        assert_eq!(got.as_deref(), Some(""), "空串不得被拒");

        let got = opt_str(&json!({ "description": "  " }), "description", "专家请求")
            .expect("纯空白也应被接受");
        assert_eq!(got.as_deref(), Some("  "), "可选串不得被 trim");
    }

    /// 钉住 `opt_str` 的类型错文案：带 `where_.key` 定位与真实类型。
    #[test]
    fn opt_str_rejects_a_non_string_with_the_received_type() {
        let err = opt_str(&json!({ "name": false }), "name", "通道请求").expect_err("布尔必须判红");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            err.detail(),
            "通道请求.name 必须是字符串，实际收到 布尔值。"
        );
    }
}
