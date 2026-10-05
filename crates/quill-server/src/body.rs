
use axum::extract::{FromRequest, Request};
use serde_json::Value;

use crate::error::ApiError;

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
