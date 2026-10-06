//! `POST /api/sessions/{id}/messages/stream` —— 同一个「发一句话」，但边生成边返回。
//!
//! 与老路由 `POST /api/sessions/{id}/messages` 的关系：**共用同一份循环**
//! （`api_chat::run_turn`）。老路由一个字都没改，仍然是「等模型答完再一次性
//! 返回 JSON」；流式只是多了一条把增量让出去的出口。
//!
//! 事件协议（`event:` 名的顺序就是一轮的顺序）：
//!
//! | 事件 | 什么时候 | 负载 |
//! |---|---|---|
//! | `user_message` | 用户消息已落库 | 与老路由 `user_message` 同形 |
//! | `delta` | 模型吐字 | `{"kind":"text","text":"…"}`，`kind` 另有 `reasoning` |
//! | `tool_call` | 模型要调工具 | `{"name","arguments"}` |
//! | `tool_result` | 工具跑完 | `{"name","ok"}`（正文在 `done` 里） |
//! | `discard` | 这一轮的正文会被覆盖 | `{"round":n}` —— 前端必须把已显示的这段抹掉 |
//! | `done` | 整轮结束 | **与老路由的响应体逐字段同形** |
//! | `error` | 出错了 | `{"code","detail","next_step"}` |
//!
//! 为什么 `done` 直接复用老路由那一份而不是另拼：两份拼装迟早只改一边，
//! 而前端在流式那条路上就看不到 `usage` / `tool_calls` 了。

use axum::extract::{Path, State};
use axum::response::sse::KeepAlive;
use axum::response::{IntoResponse, Sse};
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

use crate::api_chat::{self, ReplyMode, RoundSink};
use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::sse::{FrameStream, SseFrame};
use crate::state::AppState;

/// 一条消息的流式发送。
///
/// **响应头在跑模型之前就发出去了** —— 所以准备阶段的错误（内容为空、
/// 会话不存在、没配模型）仍然走正常的 4xx/5xx JSON 信封，用户看到的是老样子；
/// 一旦进入流里再出错，就只能靠 `error` 事件说，状态码已经改不了了。
pub async fn stream_message(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    JsonBody(body): JsonBody,
) -> Result<impl IntoResponse, ApiError> {
    let content = api_chat::take_content(&body)?;
    let mut prep = api_chat::prepare_turn(state, user, id, content).await?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<SseFrame>();
    let mut emitter = Emitter {
        tx,
        text: String::new(),
        reasoning: String::new(),
    };

    // 先把用户消息发出去：它**已经落库了**（`prepare_turn` 里写的），
    // 前端拿到它才能立刻显示自己刚发的那条。无界通道的 send 不阻塞，
    // 所以在这行发出去与在后台任务里发的效果完全一样。
    emitter.send(SseFrame::new(
        "user_message",
        api_chat::user_message_json(&prep),
    ));

    tokio::spawn(async move {
        // `run_turn` 与 `finish_turn` 的任何 Err 在这里都已经没法改成状态码了，
        // 只能变成一帧 `error`；这也是它们把中文 detail 与「下一步」原样带出来的
        // 最后一个机会。
        let outcome = match api_chat::run_turn(&mut prep, ReplyMode::Streamed, &mut emitter).await {
            Ok(outcome) => outcome,
            Err(e) => {
                emitter.fail(e);
                return;
            }
        };
        match api_chat::finish_turn(&prep, outcome).await {
            Ok(body) => emitter.done(body),
            Err(e) => emitter.fail(e),
        }
    });

    Ok(Sse::new(FrameStream::new(rx)).keep_alive(KeepAlive::default()))
}

/// 把循环里发生的事变成 SSE 帧。
struct Emitter {
    tx: UnboundedSender<SseFrame>,
    /// 本轮已经发出去的正文/思考。`discard` 要抹掉的就是它们 ——
    /// 记在这里是为了**只抹这一轮**，不会把上一轮真正答完的话也擦掉。
    text: String,
    reasoning: String,
}

impl Emitter {
    fn send(&mut self, frame: SseFrame) {
        // 接收端已经走了（用户关掉了页面）时返回 false：这之后的一切都白做，
        // 但不当作错误 —— 用户主动关页面不是失败。
        let _ = self.tx.send(frame);
    }

    fn done(&mut self, body: Value) {
        self.send(SseFrame::new("done", body));
    }

    fn fail(&mut self, e: ApiError) {
        self.send(SseFrame::new(
            "error",
            json!({
                "code": e.code(),
                "detail": e.detail(),
                "next_step": e.next_step(),
            }),
        ));
    }
}

impl RoundSink for Emitter {
    fn round_start(&mut self, _round: usize) {
        // 新一轮开始：上一轮的残留不算数。
        self.text.clear();
        self.reasoning.clear();
    }

    fn text(&mut self, delta: &str) {
        self.text.push_str(delta);
        self.send(SseFrame::new(
            "delta",
            json!({"kind": "text", "text": delta}),
        ));
    }

    fn reasoning(&mut self, delta: &str) {
        self.reasoning.push_str(delta);
        self.send(SseFrame::new(
            "delta",
            json!({"kind": "reasoning", "text": delta}),
        ));
    }

    fn tool_call(&mut self, call: &quill_provider::ToolCall) {
        self.send(SseFrame::new(
            "tool_call",
            json!({"name": call.name, "arguments": call.arguments}),
        ));
    }

    fn tool_result(&mut self, call: &quill_provider::ToolCall, ok: bool) {
        self.send(SseFrame::new(
            "tool_result",
            json!({"name": call.name, "ok": ok}),
        ));
    }

    fn discard(&mut self, round: usize) {
        let text = std::mem::take(&mut self.text);
        let reasoning = std::mem::take(&mut self.reasoning);
        self.send(SseFrame::new(
            "discard",
            json!({
                "round": round,
                // 前端要按**原文**抹掉自己那份；只给个数就够不上了
                // （前端那份可能已经被用户选中复制过）。
                "text": text,
                "reasoning": reasoning,
            }),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_discard_tells_the_frontend_exactly_how_much_to_erase() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SseFrame>();
        let mut emitter = Emitter {
            tx,
            text: String::new(),
            reasoning: String::new(),
        };
        emitter.round_start(0);
        emitter.text("先说一句");
        emitter.reasoning("想一下");
        let text_frame = rx.try_recv().expect("应当先来正文增量");
        assert_eq!(text_frame.event, "delta");
        assert_eq!(text_frame.data["text"], "先说一句");
        assert_eq!(rx.try_recv().expect("应当接着来思考增量").data["kind"], "reasoning");

        emitter.discard(1);
        let frame = rx.try_recv().expect("应当有一帧 discard");
        assert_eq!(frame.event, "discard");
        assert_eq!(frame.data["round"], 1);
        assert_eq!(
            frame.data["text"], "先说一句",
            "前端只能靠自己那份来算该抹多少，所以原文必须带上：{frame:?}"
        );
    }

    #[test]
    fn a_new_round_clears_what_the_previous_one_left_behind() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SseFrame>();
        let mut emitter = Emitter {
            tx,
            text: String::new(),
            reasoning: String::new(),
        };
        emitter.round_start(0);
        emitter.text("第一轮");
        let _ = rx.try_recv();

        emitter.round_start(1);
        emitter.discard(1);
        let frame = rx.try_recv().expect("应当有一帧 discard");
        assert!(
            frame.data["text"] == "",
            "第二轮的 discard 不该把第一轮的字抹掉（它已经被 discard 过一次）：{frame:?}"
        );
    }
}