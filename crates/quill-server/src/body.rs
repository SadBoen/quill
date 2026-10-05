//! 请求体提取器：`JsonBody` —— 把 axum 的**英文**拒绝换成中文人话。
//!
//! # 为什么需要它
//!
//! `axum::Json<Value>` 的拒绝（415 `Unsupported Media Type`、400 `Failed to
//! deserialize the JSON body`）是**纯英文的纯文本**，既没有 `next_step`，
//! 也没有请求 ID。铁律七要求「失败必须自诊断」，
//! 而一条英文的 `Expected request with Content-Type: application/json`
//! 对本项目的用户毫无诊断价值 —— 它只说明「你发的不是 JSON」，
//! 不说明「这个接口要什么形状的 JSON」。
//!
//! ⚠️ 替代方案（全局 `HandleErrorLayer`）本项目**不用**：
//! 它会把所有 handler 的错误都接管过去，与本 crate 已有的
//! [`crate::error::ApiError`] 出口形成第二条真相源。
//! 在提取器这一层就地翻译，改动面最小、也不影响已有错误路径。
//!
//! 为什么是 `Value` 而不是 `serde_json::from_value` 到具体结构体：
//! 本 crate 的依赖表里没有 `serde`（只有 `serde_json`），
//! 而手写 `Deserialize` 实现的收益远小于「字段级中文报错」
//! —— 后者由各 handler 自己给出（见 `api_experts::only_keys` 与 `need_str`）。

use axum::extract::{FromRequest, Request};
use serde_json::Value;

use crate::error::ApiError;

/// 带中文拒绝的 JSON 请求体。
#[derive(Debug, Clone)]
pub struct JsonBody(pub Value);

impl<S> FromRequest<S> for JsonBody
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<Value>::from_request(req, state).await {
            Ok(axum::Json(v)) => Ok(Self(v)),
            Err(rej) => {
                // ⚠️ 原文（英文）**只进日志**：它含有 axum 内部措辞，
                //    对用户没有诊断价值；响应体里给的是「要什么 + 下一步」。
                eprintln!("[api] 请求体提取失败：{}", rej.body_text());
                Err(ApiError::bad_request(
                    "请求体不是合法的 JSON。\
                     本接口要求 `Content-Type: application/json`，\
                     请求体是一个 JSON 对象（具体字段见接口文档）。\
                     下一步：确认请求头与 JSON 语法后重试。"
                        .to_string(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    #[tokio::test]
    async fn missing_content_type_is_rejected_in_chinese_with_next_step() {
        let req = Request::builder()
            .method("POST")
            .uri("/x")
            .body(Body::from("{}"))
            .expect("构造请求");
        let err = JsonBody::from_request(req, &())
            .await
            .expect_err("缺 Content-Type 必须判红");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
        assert!(
            err.detail().contains("Content-Type: application/json"),
            "必须说清要什么：{}",
            err.detail()
        );
        assert!(err.next_step().contains("GET /healthz"));
        assert!(
            !err.detail().contains("Unsupported Media Type"),
            "英文原文不得发给客户端：{}",
            err.detail()
        );
    }

    #[tokio::test]
    async fn valid_json_object_is_accepted() {
        let req = Request::builder()
            .method("POST")
            .uri("/x")
            .header("content-type", "application/json")
            .body(Body::from("{\"a\":1}"))
            .expect("构造请求");
        let ok = JsonBody::from_request(req, &())
            .await
            .expect("合法 JSON 应被接受");
        assert_eq!(ok.0["a"], serde_json::json!(1));
    }
}
