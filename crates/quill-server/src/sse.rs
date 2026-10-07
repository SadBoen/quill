//! SSE 的传输层：把「一个后台任务发出来的事件」变成「一条还没结束的 HTTP 响应」。
//!
//! 为什么单独一个文件：Axum 的响应体必须是 `Stream`，而 `Stream` 得自己实现
//! （`futures-core` 里那条 trait）。这层胶水与聊天逻辑无关，混进 `api_chat`
//! 里只会把那个文件撑得没法读。
//!
//! 为什么用**无界**通道：无界是 Axum 自己的 `Sse` 内部用的同一种做法。
//! 增量必须立刻让出去（攒完再发就退化成一次性响应了），而有界通道在客户端
//! 读得慢时会卡住模型那一侧 —— 对「把字显示出来」这个场景，让不出延迟
//! 比偶尔多占几 KB 更糟。

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::response::sse::Event;
use serde_json::Value;

/// 一个事件：名字 + JSON 负载。
///
/// 名字走 SSE 的 `event:` 字段而不是塞进 JSON 里，前端读事件时就能分派，
/// 不用先 `JSON.parse` 再猜。
#[derive(Debug, Clone, PartialEq)]
pub struct SseFrame {
    pub event: &'static str,
    pub data: Value,
}

impl SseFrame {
    pub fn new(event: &'static str, data: Value) -> Self {
        Self { event, data }
    }

    fn json(&self) -> String {
        serde_json::to_string(&self.data).unwrap_or_else(|_| "null".to_string())
    }

    /// 交给 Axum 之前，**这一帧在线上应当长什么样**。
    ///
    /// 它不参与发送，只用来钉住 `to_event` 的结果 —— 见下面的
    /// `an_event_encodes_to_exactly_the_frame_we_specified`。
    pub fn to_wire(&self) -> String {
        format!("event: {}\ndata: {}\n\n", self.event, self.json())
    }

    /// 变成一个 Axum 的 SSE 事件。
    ///
    /// **不要自己拼好整帧再交给 `Event::data`**：`data` 会按换行把内容拆成
    /// 多条 `data:` 字段，于是「事件名」也变成了数据的一部分，前端永远收不到
    /// `event:` 那一行。名字交给 `Event::event`、负载交给 `Event::data`，
    /// 编码交给 Axum —— 各管各的。
    pub fn to_event(&self) -> Event {
        Event::default().event(self.event).data(self.json())
    }
}

/// 把 tokio 的无界接收端包成 SSE 响应体。
///
/// 队列空时返回 `Poll::Pending` 并登记唤醒；发送端被丢掉（后台任务结束）时
/// 接收端返回 `None`，这里就顺着把流结束掉 —— 浏览器不会一直转圈。
pub struct FrameStream {
    rx: tokio::sync::mpsc::UnboundedReceiver<SseFrame>,
}

impl FrameStream {
    pub fn new(rx: tokio::sync::mpsc::UnboundedReceiver<SseFrame>) -> Self {
        Self { rx }
    }
}

impl futures_core::Stream for FrameStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // `UnboundedReceiver` 是 `Unpin`，所以 `Self` 也是，取 `&mut self` 即可。
        match self.get_mut().rx.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(frame)) => Poll::Ready(Some(Ok(frame.to_event()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 不唤醒任何东西的 waker：标准库的 `Waker::noop()` 就是它，不必自己写一个
    /// `Wake` 实现（clippy::manual_noop_waker）。用途是在单测里直接 `poll` 一条流 ——
    /// `futures-core` 只有 trait，没有 `StreamExt::next()` 可用。
    fn poll_once<S: futures_core::Stream + Unpin>(mut stream: S) -> Poll<Option<S::Item>> {
        let mut cx = Context::from_waker(std::task::Waker::noop());
        std::pin::Pin::new(&mut stream).poll_next(&mut cx)
    }

    fn payload_samples() -> Vec<Value> {
        vec![
            json!({"kind": "text", "text": "你好"}),
            // 负载里带换行、带引号、带反斜杠、带花括号都不能破帧。
            json!({"text": "第一行\n第二行"}),
            json!({"text": "带\"引号\"和 \\ 反斜杠"}),
            json!({"arguments": {"q": "登录\n注册"}, "ok": true}),
            json!([1, 2, {"deep": {"nested": "值"}}]),
        ]
    }

    #[test]
    fn the_wire_form_is_one_event_line_one_data_line_and_a_blank_separator() {
        let frame = SseFrame::new("tool_result", json!({"name": "x", "ok": false}));
        assert_eq!(
            frame.to_wire(),
            "event: tool_result\ndata: {\"name\":\"x\",\"ok\":false}\n\n"
        );
    }

    #[test]
    fn every_encoded_line_is_a_real_sse_field() {
        // 任何非字段行都会让前端要么丢掉内容、要么拼出半截 JSON —— 而且
        // **不报错**，只是界面上少了一段字，最难发现的那种。
        for payload in payload_samples() {
            let frame = SseFrame::new("delta", payload.clone());
            let wire = frame.to_wire();
            for line in wire.lines().filter(|l| !l.is_empty()) {
                assert!(
                    line.starts_with("event: ") || line.starts_with("data: "),
                    "这一行既不是 SSE 字段也不是空行，会把整帧拆坏：{line:?}（负载 {payload}）"
                );
            }
            let data = wire
                .lines()
                .find_map(|l| l.strip_prefix("data: "))
                .expect("至少要有一行 data");
            assert_eq!(
                serde_json::from_str::<Value>(data).expect("data 必须是完整 JSON"),
                payload,
                "编码器把负载改坏了：{wire:?}"
            );
        }
    }

    #[test]
    fn the_event_name_is_its_own_field_not_part_of_the_data() {
        let frame = SseFrame::new("tool_result", json!({"name": "x", "ok": false}));
        let got = format!("{:?}", frame.to_event());
        assert!(
            got.contains("event: tool_result"),
            "事件名必须落在 event: 字段上：{got}"
        );
        assert!(
            !got.contains("event: tool_result\ndata:"),
            "事件名不能被并进 data 行：{got}"
        );
    }

    #[test]
    fn the_stream_ends_when_the_sender_is_dropped() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SseFrame>();
        tx.send(SseFrame::new("delta", json!({"text": "你"})))
            .unwrap();
        drop(tx);

        let mut stream = FrameStream::new(rx);
        let first = match poll_once(&mut stream) {
            Poll::Ready(Some(item)) => item.expect("Infallible 不该出错"),
            other => panic!("应当先读到那一帧，实际：{other:?}"),
        };
        assert!(
            format!("{first:?}").contains("event: delta"),
            "帧里的事件名必须原样传到浏览器：{first:?}"
        );
        assert!(
            matches!(poll_once(&mut stream), Poll::Ready(None)),
            "发送端消失后流必须结束，否则浏览器会一直转圈"
        );
    }
}
