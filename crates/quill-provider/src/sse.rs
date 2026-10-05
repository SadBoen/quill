use crate::error::ProviderError;

pub const DONE_SENTINEL: &str = "[DONE]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    Data(String),

    Done,
}

/// Incremental `text/event-stream` decoder: feed it arbitrary slices of the
/// response body, get back whole events. Anything after the last newline stays
/// buffered, so a `data:` line split across two reads is reassembled rather than
/// parsed as two broken halves.
#[derive(Debug, Default, Clone)]
pub struct SseDecoder {
    pending: String,

    data: Vec<String>,

    finished: bool,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `Err` only for a decoder already past `[DONE]` that is fed more
    /// data, which is the one case where continuing would silently drop output.
    pub fn push(&mut self, chunk: &str) -> Result<Vec<SseEvent>, ProviderError> {
        if self.finished {
            return Err(ProviderError::MalformedResponse {
                detail: format!(
                    "流已经在 {DONE_SENTINEL} 结束，却还有后续数据：{}",
                    crate::error::excerpt(chunk)
                ),
            });
        }
        if chunk.is_empty() {
            return Ok(Vec::new());
        }

        self.pending.push_str(chunk);
        let mut events = Vec::new();
        while let Some(nl) = self.pending.find('\n') {
            let mut line: String = self.pending.drain(..=nl).collect();
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
            self.consume_line(&line, &mut events)?;
        }
        Ok(events)
    }

    /// Flush at end of body. Servers that close without a trailing blank line
    /// still have their final event delivered here.
    pub fn finish(&mut self) -> Result<Vec<SseEvent>, ProviderError> {
        if self.finished {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        let tail = std::mem::take(&mut self.pending);
        if !tail.is_empty() {
            let mut line = tail;
            if line.ends_with('\r') {
                line.pop();
            }
            self.consume_line(&line, &mut events)?;
        }
        self.dispatch(&mut events)?;
        Ok(events)
    }

    fn consume_line(&mut self, line: &str, events: &mut Vec<SseEvent>) -> Result<(), ProviderError> {
        if line.is_empty() {
            self.dispatch(events)?;
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }
        // SSE allows both "data: x" and "data:x"; only one leading space is stripped.
        if let Some(rest) = line.strip_prefix("data:") {
            self.data.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
        Ok(())
    }

    fn dispatch(&mut self, events: &mut Vec<SseEvent>) -> Result<(), ProviderError> {
        if self.data.is_empty() {
            return Ok(());
        }
        let joined = self.data.join("\n");
        self.data.clear();
        if joined.trim() == DONE_SENTINEL {
            self.finished = true;
            events.push(SseEvent::Done);
            return Ok(());
        }
        if joined.trim().is_empty() {
            return Ok(());
        }
        events.push(SseEvent::Data(joined));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ProviderError;

    fn data(events: &[SseEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                SseEvent::Data(d) => Some(d.as_str()),
                SseEvent::Done => None,
            })
            .collect()
    }

    #[test]
    fn a_single_whole_event_round_trips() {
        let mut d = SseDecoder::new();
        let got = d.push("data: {\"a\":1}\n\n").expect("不应报错");
        assert_eq!(data(&got), vec!["{\"a\":1}"]);
    }

    #[test]
    fn a_data_line_split_across_reads_is_reassembled() {
        let mut d = SseDecoder::new();
        assert!(d.push("data: {\"hel").expect("半截不能报错").is_empty());
        assert!(d.push("lo\":\"wor").expect("半截不能报错").is_empty());
        let got = d
            .push("ld\"}\n\n")
            .expect("补齐后不应报错")
            .into_iter()
            .chain(d.finish().expect("收尾不应报错"))
            .collect::<Vec<_>>();
        assert_eq!(data(&got), vec!["{\"hello\":\"world\"}"]);
    }

    #[test]
    fn a_crlf_stream_keeps_the_carriage_return_out_of_the_payload() {
        let mut d = SseDecoder::new();
        let got = d.push("data: x\r\n\r\n").expect("不应报错");
        assert_eq!(data(&got), vec!["x"]);
    }

    #[test]
    fn several_events_in_one_push_are_all_delivered() {
        let mut d = SseDecoder::new();
        let got = d
            .push("data: 1\n\ndata: 2\n\ndata: 3\n\n")
            .expect("不应报错");
        assert_eq!(data(&got), vec!["1", "2", "3"]);
    }

    #[test]
    fn the_done_sentinel_is_its_own_event() {
        let mut d = SseDecoder::new();
        let got = d
            .push("data: 1\n\ndata: [DONE]\n\n")
            .expect("不应报错");
        assert_eq!(data(&got), vec!["1"]);
        assert_eq!(got.last(), Some(&SseEvent::Done));
    }

    #[test]
    fn data_after_done_is_a_classified_error_not_a_panic() {
        let mut d = SseDecoder::new();
        d.push("data: [DONE]\n\n").expect("不应报错");
        let err = d.push("data: 1\n\n").expect_err("DONE 之后再喂数据必须报错");
        assert!(matches!(err, ProviderError::MalformedResponse { .. }));
        assert!(err.to_string().contains("[DONE]"), "文案要点明 DONE：{err}");
    }

    #[test]
    fn multiple_data_lines_join_into_one_event() {
        let mut d = SseDecoder::new();
        let got = d
            .push("data: {\"a\":\ndata: 1}\n\n")
            .expect("不应报错");
        assert_eq!(data(&got), vec!["{\"a\":\n1}"]);
    }

    #[test]
    fn comments_keep_alive_lines_and_other_fields_are_ignored() {
        let mut d = SseDecoder::new();
        let got = d
            .push(": keep-alive\nevent: message\nid: 7\nretry: 100\ndata: v\n\n")
            .expect("不应报错");
        assert_eq!(data(&got), vec!["v"]);
    }

    #[test]
    fn a_data_line_without_a_space_after_the_colon_still_parses() {
        let mut d = SseDecoder::new();
        let got = d.push("data:v\n\n").expect("不应报错");
        assert_eq!(data(&got), vec!["v"]);
    }

    #[test]
    fn an_empty_data_line_emits_no_event() {
        let mut d = SseDecoder::new();
        assert!(d.push("data:\n\n").expect("不应报错").is_empty());
        assert!(d.push("data:   \n\n").expect("不应报错").is_empty());
    }

    #[test]
    fn finish_flushes_a_trailing_event_with_no_blank_line() {
        let mut d = SseDecoder::new();
        assert!(d.push("data: last").expect("不应报错").is_empty());
        let got = d.finish().expect("收尾不应报错");
        assert_eq!(data(&got), vec!["last"]);
    }

    #[test]
    fn finish_after_done_yields_nothing_more() {
        let mut d = SseDecoder::new();
        d.push("data: [DONE]\n\n").expect("不应报错");
        assert!(d.finish().expect("收尾不应报错").is_empty());
    }

    #[test]
    fn an_empty_push_is_a_no_op() {
        let mut d = SseDecoder::new();
        assert!(d.push("").expect("空推送不应报错").is_empty());
        assert!(d.push("data: 1\n\n").expect("不应报错").len() == 1);
    }

    #[test]
    fn one_byte_at_a_time_yields_the_same_events_as_one_push() {
        let body = ": ping\n\ndata: {\"i\":0}\n\ndata: {\"i\":1}\n\ndata: [DONE]\n\n";
        let mut whole = SseDecoder::new();
        let expect = whole.push(body).expect("不应报错");

        let mut d = SseDecoder::new();
        let mut got = Vec::new();
        for ch in body.chars() {
            got.extend(d.push(&ch.to_string()).expect("逐字节推送不应报错"));
        }
        got.extend(d.finish().expect("收尾不应报错"));
        assert_eq!(got, expect);
    }
}
