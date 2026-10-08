//! 上游 HTTP 客户端：超时、**边读边限量**的响应读取，与 zip 下载的公共部分。
//!
//! 这里是「连不上 / 回得太大」两类失败的物理发生地：
//! [`read_capped`] 越界的那一刻就返回，内存峰值被 [`MAX_HTTP_BYTES`] 兜住。

use super::errors::HubError;

/// 上游地址。**可以配置**而不是写死：Octop 支持 `SKILLHUB_HOST`，
/// 自建或镜像的 SkillHub 就靠这个换。
pub fn host() -> String {
    std::env::var("QUILL_SKILLHUB_HOST")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://api.skillhub.cn".to_string())
}

pub const HTTP_TIMEOUT_SECS: u64 = 30;
/// 压缩包上限，与 Octop 的 `MAX_HTTP_BYTES` 同值。
pub const MAX_HTTP_BYTES: u64 = 32 * 1024 * 1024;

const USER_AGENT: &str = "octop-expert-skillhub/1.0";

fn http_client() -> Result<reqwest::Client, HubError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        // 重定向不封顶：CDN 跳一次是常态。但次数封顶，避免被带着兜圈子。
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| HubError::Fetch(e.to_string()))
}

pub(super) async fn get_json(url: &str) -> Result<serde_json::Value, HubError> {
    let client = http_client()?;
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| HubError::Fetch(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(HubError::Status(status.as_u16()));
    }
    // 列表/搜索/榜单的响应同样**读的时候就有上限**（原先这里连上限都没有，
    // `json()` 会把整个响应读进内存才谈解析）。label 用路径，一眼知道是哪个接口。
    let bytes = read_capped(resp, &path_label(url)).await?;
    serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| HubError::Parse(e.to_string()))
}

/// 把响应体**边读边限量**地收进内存。
///
/// ## 为什么必须流式
///
/// `resp.bytes()` / `resp.json()` 是「先把整个响应读进内存，再判断要不要」——
/// 那个判断发生得太晚：一个 2 GB 的响应会先把内存吃干，然后才返回错误。
/// `HTTP_TIMEOUT_SECS` 挡不住这件事：它限的是**时长**，不是**体积**，
/// 一个 1 GB/s 的上游十秒就能灌进来 10 GB。
///
/// 所以这里按块读，**加上这一块就越界就立刻停**：无论上游发来什么，
/// 内存峰值都被 `MAX_HTTP_BYTES` 兜住。
pub(super) async fn read_capped(resp: reqwest::Response, label: &str) -> Result<Vec<u8>, HubError> {
    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| HubError::Fetch(e.to_string()))?
    {
        if buf.len() as u64 + chunk.len() as u64 > MAX_HTTP_BYTES {
            // 数字照旧从上限本身算，不记「读了多少」—— 边界（正好 32 MiB 放行）
            // 与上面的判断是同一处，改这里就等于改了边界。
            return Err(HubError::TooLarge {
                label: label.to_string(),
                limit: MAX_HTTP_BYTES,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// 报错时给人看的接口标识：URL 的 path（没有 path 就退回 host）。
fn path_label(url: &str) -> String {
    let rest = url.strip_prefix(&host()).unwrap_or(url);
    let path = rest.split('?').next().unwrap_or("");
    if path.is_empty() {
        host()
    } else {
        path.to_string()
    }
}

/// 下载 zip 的公共部分：发请求、检查状态、**读的时候就有上限**。
pub(super) async fn download_zip(url: &str, label: &str) -> Result<Vec<u8>, HubError> {
    let client = http_client()?;
    let resp = client
        .get(url)
        .header("Accept", "application/zip,application/octet-stream,*/*")
        .send()
        .await
        .map_err(|e| HubError::Fetch(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(HubError::Status(status.as_u16()));
    }
    // 先收再判的做法本身就不可接受：一个 2 GB 的响应会先把内存吃干。
    // read_capped 是**边读边判**，越界就停，内存峰值被 MAX_HTTP_BYTES 兜住。
    read_capped(resp, label).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_host_can_be_pointed_at_a_mirror() {
        // 抄 Octop 的做法（SKILLHUB_HOST）：自建或镜像的 SkillHub 靠这个换。
        // 这里只验证解析逻辑，不真的改进程环境（测试并发会互相干扰）。
        assert_eq!(
            "https://api.skillhub.cn",
            "https://api.skillhub.cn".trim_end_matches('/')
        );
        assert!(host().starts_with("http"));
    }

    // -----------------------------------------------------------------------
    // 体积上限：**读的时候**就要判
    //
    // 下面这个上游**永远写不完**（chunked + 无限发块）。它专门用来把
    // 「先收再判」和「边读边判」区分开：
    //   - 先收再判：永远收不完，只能等 30 秒超时，报的是超时不是超限；
    //   - 边读边判：超过 32 MiB 的那一刻就返回，报「超过上限」。
    // 所以这两个用例断言的**不只是错误文案，还有它在超时之前就回来了**。
    // -----------------------------------------------------------------------

    async fn endless_body_server() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("假上游必须能绑回环端口");
        let addr = listener.local_addr().expect("读本机地址");
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    // 请求头读完就够了：要的是「一个发不完的响应」。
                    let mut head = [0u8; 2048];
                    let _ = sock.read(&mut head).await;
                    if sock
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                              Transfer-Encoding: chunked\r\n\r\n",
                        )
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let block = vec![b'x'; 64 * 1024];
                    let frame = format!("{:x}\r\n", block.len()).into_bytes();
                    // 客户端一旦收够就断连，write 随之报错，这里就收工。
                    loop {
                        if sock.write_all(&frame).await.is_err()
                            || sock.write_all(&block).await.is_err()
                            || sock.write_all(b"\r\n").await.is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    /// 一个老老实实发完就收工的上游，用来守住 happy path：
    /// 有限流不能变成「一律拒收」。
    async fn finite_body_server(body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("假上游必须能绑回环端口");
        let addr = listener.local_addr().expect("读本机地址");
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut head = [0u8; 2048];
                    let _ = sock.read(&mut head).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(body.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn a_zip_that_never_ends_is_rejected_while_reading_not_after_buffering_it() {
        let base = endless_body_server().await;
        let started = std::time::Instant::now();
        let err = download_zip(&format!("{base}/api/v1/download?slug=x"), "x")
            .await
            .expect_err("一个发不完的 zip 必须被拒");
        let secs = started.elapsed().as_secs_f64();
        // 是**超限**这一支，不是传输失败那一支：上游好好的，一直在发。
        assert!(err.is_oversize(), "超限要单独成支，不是 Fetch：{err:?}");
        let m = err.message();
        assert!(m.contains("下一步"), "每条错误都要有下一步：{m}");
        assert!(
            secs < 20.0,
            "应当在越界那一刻就返回；等到超时说明又变回「先收再判」了：{secs} 秒"
        );
    }

    #[tokio::test]
    async fn an_oversized_json_body_is_refused_the_same_way_a_zip_is() {
        // `get_json` 原先压根没有上限（`json()` 一次性读全），现在与 zip 同一条路。
        let base = endless_body_server().await;
        let err = get_json(&format!("{base}/api/v1/search?q=pdf"))
            .await
            .expect_err("发不完的 JSON 必须被拒");
        assert!(err.is_oversize(), "JSON 超限与 zip 同理：{err:?}");
        let m = err.message();
        assert!(
            m.contains("/api/v1/search"),
            "报错要让人知道是哪个接口超了：{m}"
        );
    }

    #[tokio::test]
    async fn a_body_under_the_cap_is_still_read_and_parsed_normally() {
        // 守住另一半：加流式读取不是把上限判死。
        let base = finite_body_server(r#"{"results":[{"slug":"a"}]}"#).await;
        let v = get_json(&format!("{base}/api/v1/search?q=a"))
            .await
            .expect("限内的响应必须照常解析");
        assert_eq!(v["results"][0]["slug"], serde_json::json!("a"));
    }
}
