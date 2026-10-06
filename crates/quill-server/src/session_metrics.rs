//! 会话级 Token 统计。
//!
//! 界面（`ui/web/src/chat/SessionMetricsBar.tsx`）照 octop 的
//! `TrajectoryMetricsBar` 迁移，字段名与口径也照抄它的 `TrajectoryMetrics`。
//! 抄的是**形状**，不是数字 —— 每个指标能不能算出来，取决于 quill 到底记了什么。
//!
//! # 本项目的硬规矩：算不出来就是 null，不许编
//!
//! octop 那十个指标里，quill 现在只能诚实地产出一部分。剩下的宁可**不出现在
//! 界面上**（前端会按 null 过滤掉），也不能拿 0 或估算值顶上。理由很直接：
//! 用户会拿这些数字做决策，一个凭空来的「命中率 0%」比没有数字坏得多。
//!
//! | 指标 | quill 能否诚实产出 | 数据来源 |
//! |---|---|---|
//! | `turns` | ✅ | `role='user'` 的消息条数 |
//! | `steps` | ✅ | `role='assistant'` 的消息条数 |
//! | `llm_duration_ms` | ✅ | assistant 消息 `turn_ms` 求和 |
//! | `input_tokens` / `output_tokens` | ✅ | 逐轮 usage 求和 |
//! | `tok_per_s` | ✅ | output ÷ 模型耗时（两个都是真值才算）|
//! | `cache_hit_ratio` / `cache_read_tokens` | ⚠️ 条件性 | 仅当模型端真的上报了缓存 token |
//! | `tool_duration_ms` | ❌ | quill 不记录单次工具耗时 |
//! | `ttft_avg_ms` | ❌ | quill 不记录首 token 时刻 |
//!
//! 后两项永远是 null。**不是「暂时没接」，是这条数据链在 quill 里不存在** ——
//! 要接得先改写入侧（在 tools.rs 里给每次工具调用掐表），那是另一件事。

use serde_json::{json, Value};

/// 会话级指标。字段与 octop 的 `TrajectoryMetrics` 同名同义，方便对照。
///
/// 除 `turns`/`steps` 外全部可空：`null` 在这个项目里是一个有意义的值，
/// 意思是「没记 / 不知道」，前端必须原样跳过而不是显示成 0。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMetrics {
    pub turns: i64,
    pub steps: i64,
    pub llm_duration_ms: Option<i64>,
    pub tool_duration_ms: Option<i64>,
    pub ttft_avg_ms: Option<i64>,
    pub tok_per_s: Option<f64>,
    pub cache_hit_ratio: Option<f64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
}

impl SessionMetrics {
    /// 序列化成接口响应。
    ///
    /// 手写而不是 `#[derive(Serialize)]`：quill-server 没依赖 `serde`，
    /// 为一个纯数值结构引入一个新依赖不值得。
    pub fn to_json(&self) -> Value {
        json!({
            "turns": self.turns,
            "steps": self.steps,
            "llm_duration_ms": self.llm_duration_ms,
            "tool_duration_ms": self.tool_duration_ms,
            "ttft_avg_ms": self.ttft_avg_ms,
            "tok_per_s": self.tok_per_s,
            "cache_hit_ratio": self.cache_hit_ratio,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_read_tokens": self.cache_read_tokens,
        })
    }
}

/// 一条消息行里统计要用到的字段。
///
/// 刻意用「一条消息」的粒度而不是「整段 SQL 结果」：聚合口径是纯函数，
/// 才测得清楚（见本文件下方测试），而 SQL 只负责把行取出来。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MessageUsage {
    pub is_user: bool,
    pub is_assistant: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// 该轮的模型端点耗时（毫秒）。`None` = 没量到。
    pub turn_ms: Option<i64>,
    /// `None` = 模型端**没有上报**缓存读命中；`Some(0)` = 报了，确实是 0。
    ///
    /// 这个区分是本文件的核心，见 [`SessionMetrics::cache_read_tokens`]。
    pub cache_read_tokens: Option<i64>,
}

/// 汇总一段消息得到会话级指标。
///
/// 规则，逐条都有理由：
///
/// 1. 只统计 `is_user` / `is_assistant`。工具往返、system、失败残留都不算 ——
///    模型的 token 花在 assistant 行上。
/// 2. `llm_duration_ms` 只在**每一条都量到了**时才给值。缺一条就整体为
///    `None`：把 5 条里量到的 4 条加起来当全程耗时，是少算了却看不出少算。
/// 3. `tok_per_s` 同理，两个输入必须都是真值。
/// 4. `input_tokens` / `output_tokens` 只要**至少有一条**报了真值就给值。
///    和第 2 条不同：token 是可加的，少了一条只是偏小，而耗时少算会让速度**偏快**，
///    那是个会误导人的方向。
/// 5. `cache_read_tokens` 只在**至少一条上报过**时才给值。全部为 `None` 时保持
///    `None` —— 这是整个模块最容易写错、后果也最直接的一处。
pub fn aggregate(rows: &[MessageUsage]) -> SessionMetrics {
    let turns = rows.iter().filter(|r| r.is_user).count() as i64;
    let steps = rows.iter().filter(|r| r.is_assistant).count() as i64;

    let assistants: Vec<&MessageUsage> = rows.iter().filter(|r| r.is_assistant).collect();

    // (2) 耗时必须条条齐全，否则宁可不给。
    let llm_duration_ms = if assistants.is_empty() {
        None
    } else if assistants.iter().all(|r| r.turn_ms.is_some()) {
        Some(assistants.iter().filter_map(|r| r.turn_ms).sum())
    } else {
        None
    };

    // (4) token 可加，有一条真值即可。
    let input_tokens = sum_reported(assistants.iter().map(|r| r.input_tokens));
    let output_tokens = sum_reported(assistants.iter().map(|r| r.output_tokens));

    // (3) 速度要两个真值。
    let tok_per_s = match (llm_duration_ms, output_tokens) {
        (Some(ms), Some(out)) if ms > 0 => Some(out as f64 / (ms as f64 / 1000.0)),
        _ => None,
    };

    // (5) 缓存：至少一条上报过才有话说。
    let reported_cache: Vec<i64> = assistants.iter().filter_map(|r| r.cache_read_tokens).collect();
    let cache_read_tokens = if reported_cache.is_empty() {
        None
    } else {
        Some(reported_cache.iter().sum())
    };
    let cache_hit_ratio = match (cache_read_tokens, input_tokens) {
        (Some(read), Some(input)) if input > 0 => Some(read as f64 / input as f64),
        _ => None,
    };

    SessionMetrics {
        turns,
        steps,
        llm_duration_ms,
        // (表头) quill 不记录单次工具耗时，也不记录首 token 时刻。
        // 恒为 None，是「这条数据链不存在」，不是「暂时没接」。
        tool_duration_ms: None,
        ttft_avg_ms: None,
        tok_per_s,
        cache_hit_ratio,
        input_tokens,
        output_tokens,
        cache_read_tokens,
    }
}

/// 空会话的形状：轮次和步骤是 0，其余全 None。
pub fn empty() -> SessionMetrics {
    aggregate(&[])
}

/// 求和，但要求至少有一个真值参与；否则 None。
fn sum_reported(values: impl Iterator<Item = i64>) -> Option<i64> {
    let mut total: i64 = 0;
    let mut any = false;
    for v in values {
        total = total.saturating_add(v);
        any = true;
    }
    any.then_some(total)
}

// ---------------------------------------------------------------------------
// 上下文窗口环形图
// ---------------------------------------------------------------------------
//
// 抄 octop 的 `ContextWindowRing`。但有一处**必须不一样**，而且是硬规矩：
//
// - **环的总占用是实测的**：`used_tokens` 来自模型端真实上报的
//   `input_tokens`，`max_tokens` 来自配置。Octop 的条宽也是这么来的
//   （它自己的注释写着「Provider input usage owns the total bar width」）。
// - **分段是字符数，不是 token 数。** quill 没有分词器，把字符数说成
//   token 数就是凭空造数字。所以分段只表达**相对构成**（谁占得多），
//   单位在界面上明写「字符」。Octop 那边给分段值加 `~` 前缀，
//   是同一个诚实动作的另一种写法。

/// 构成上下文的一段。`chars` 是**字符数**，见上面的说明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextSegment {
    pub key: &'static str,
    pub chars: usize,
}

/// 上下文构成（全部是字符数）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextBreakdown {
    pub system_prompt: usize,
    /// 内置工具的定义。
    pub tool_definitions: usize,
    pub skills: usize,
    pub mcp: usize,
    pub conversation: usize,
}

/// 一条工具定义的字符数。名字 + 描述 + 参数 JSON。
///
/// 参数 JSON 也算进去：它同样常驻在每一轮请求里，只算描述会低估。
pub fn tool_spec_chars(spec: &quill_provider::ToolSpec) -> usize {
    spec.name.chars().count()
        + spec.description.chars().count()
        + spec.parameters.to_string().chars().count()
}

/// 按 Octop 的配色键拆段。顺序与 Octop 的 `SEGMENT_COLORS` 一致。
///
/// 零值的段**保留**而不是剔除：界面上「技能 0 字符」是一条信息，
/// 而把没挂技能这件事藏起来，用户只会以为没统计到。
pub fn context_segments(b: &ContextBreakdown) -> Vec<ContextSegment> {
    [
        ("system_prompt", b.system_prompt),
        ("tool_definitions", b.tool_definitions),
        ("skills", b.skills),
        ("mcp", b.mcp),
        ("conversation", b.conversation),
    ]
    .into_iter()
    .map(|(key, chars)| ContextSegment { key, chars })
    .collect()
}

/// 上下文占用的百分比。
///
/// 抄 Octop 的 `contextUsedPercent`，包括那条反直觉但重要的规则：
/// **真的占了非零就至少显示 1%**，不许因为四舍五入显示成 0%——
/// 「0%」看着像「还没用」，实际已经占了 8000 token。
/// 但**真的没用**（used <= 0）就是 0%，不抬成 1%。
pub fn context_used_percent(used: i64, max: i64) -> u32 {
    if max <= 0 || used <= 0 {
        return 0;
    }
    let pct = ((used.min(max) as f64 / max as f64) * 100.0).round() as u32;
    if pct == 0 {
        1
    } else {
        pct
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(input: i64, output: i64, turn_ms: Option<i64>, cache: Option<i64>) -> MessageUsage {
        MessageUsage {
            is_user: false,
            is_assistant: true,
            input_tokens: input,
            output_tokens: output,
            turn_ms,
            cache_read_tokens: cache,
        }
    }

    fn user() -> MessageUsage {
        MessageUsage {
            is_user: true,
            ..Default::default()
        }
    }

    #[test]
    fn empty_session_has_turns_but_no_token_numbers() {
        let m = empty();
        assert_eq!(m.turns, 0);
        assert_eq!(m.steps, 0);
        assert_eq!(m.input_tokens, None, "没有消息就没有 token 可报");
        assert_eq!(m.output_tokens, None);
        assert_eq!(m.llm_duration_ms, None);
        assert_eq!(m.cache_read_tokens, None);
        assert_eq!(m.cache_hit_ratio, None, "分母都没有，不能报 0%");
    }

    #[test]
    fn turns_and_steps_come_from_the_matching_roles() {
        let rows = vec![user(), assistant(100, 20, Some(2000), None), user(), assistant(50, 5, Some(1000), None)];
        let m = aggregate(&rows);
        assert_eq!(m.turns, 2);
        assert_eq!(m.steps, 2);
        assert_eq!(m.input_tokens, Some(150));
        assert_eq!(m.output_tokens, Some(25));
        assert_eq!(m.llm_duration_ms, Some(3000));
    }

    #[test]
    fn tool_and_system_rows_are_not_counted_as_turns_or_steps() {
        let rows = vec![
            user(),
            assistant(100, 20, Some(2000), None),
            MessageUsage {
                is_user: false,
                is_assistant: false,
                ..Default::default()
            },
        ];
        let m = aggregate(&rows);
        assert_eq!(m.turns, 1, "工具往返不是一轮对话");
        assert_eq!(m.steps, 1);
    }

    #[test]
    fn tokens_per_second_needs_both_a_duration_and_output() {
        let with_both = aggregate(&[assistant(100, 300, Some(2000), None)]);
        assert_eq!(with_both.tok_per_s, Some(150.0), "300 token / 2 秒");

        let no_duration = aggregate(&[assistant(100, 300, None, None)]);
        assert_eq!(no_duration.tok_per_s, None, "没量到耗时就不能算速度");

        let no_output = aggregate(&[assistant(100, 0, Some(2000), None)]);
        assert_eq!(no_output.tok_per_s, Some(0.0), "真的是 0 输出，是 0.0 不是 null");
    }

    #[test]
    fn zero_duration_does_not_divide_by_zero() {
        let m = aggregate(&[assistant(100, 300, Some(0), None)]);
        assert_eq!(m.tok_per_s, None, "除零必须给 null，不能给 inf 或 NaN");
    }

    #[test]
    fn a_missing_duration_invalidates_the_whole_sum_not_just_its_row() {
        // 3 条里只量到 2 条：报 2000 会让人以为全程都是 2 秒，速度被算高。
        let rows = vec![
            assistant(100, 20, Some(2000), None),
            assistant(100, 20, None, None),
            assistant(100, 20, Some(2000), None),
        ];
        let m = aggregate(&rows);
        assert_eq!(m.llm_duration_ms, None, "缺一条就不能声称知道全程耗时");
        assert_eq!(m.tok_per_s, None, "耗时不可信，速度也就不该报");
        assert_eq!(m.input_tokens, Some(300), "token 仍可加，不受耗时缺失影响");
    }

    #[test]
    fn cache_metrics_stay_null_when_the_provider_never_reported() {
        // 本地模型绝大多数不报缓存 token。此时报 0 命中率就是凭空捏造。
        let m = aggregate(&[assistant(1000, 20, Some(2000), None)]);
        assert_eq!(m.cache_read_tokens, None);
        assert_eq!(m.cache_hit_ratio, None);
    }

    #[test]
    fn a_reported_zero_cache_hit_is_zero_not_null() {
        // 上游确实报了 cached_tokens=0 —— 这是真实值，必须显示出来。
        let m = aggregate(&[assistant(1000, 20, Some(2000), Some(0))]);
        assert_eq!(m.cache_read_tokens, Some(0));
        assert_eq!(m.cache_hit_ratio, Some(0.0));
    }

    #[test]
    fn cache_ratio_does_not_double_count_into_the_input_total() {
        // goose 的口径：cache_read 是 input 的子集，input 已含它。
        // 若把 900 再加进分母，会算出 >100% 的命中率。
        let m = aggregate(&[assistant(1000, 20, Some(2000), Some(900))]);
        assert_eq!(m.input_tokens, Some(1000));
        assert_eq!(m.cache_read_tokens, Some(900));
        let ratio = m.cache_hit_ratio.expect("应当能算");
        assert!((ratio - 0.9).abs() < 1e-9, "命中率应为 90%，实际 {ratio}");
        assert!(ratio <= 1.0, "命中率不可能超过 100%");
    }

    #[test]
    fn cache_ratio_is_null_when_there_was_no_input_to_divide_by() {
        let m = aggregate(&[assistant(0, 0, Some(1000), Some(0))]);
        assert_eq!(m.cache_read_tokens, Some(0), "上报了就是上报了");
        assert_eq!(m.cache_hit_ratio, None, "分母为 0，不能报比率");
    }

    #[test]
    fn one_reported_row_is_enough_to_report_the_cache_total() {
        let rows = vec![
            assistant(100, 20, Some(2000), None),
            assistant(100, 20, Some(2000), Some(50)),
        ];
        let m = aggregate(&rows);
        assert_eq!(m.cache_read_tokens, Some(50));
    }

    #[test]
    fn unimplemented_metrics_are_explicitly_null_never_zero() {
        let m = aggregate(&[assistant(100, 20, Some(2000), Some(10))]);
        assert_eq!(m.tool_duration_ms, None, "quill 不记录工具耗时");
        assert_eq!(m.ttft_avg_ms, None, "quill 不记录首 token 时刻");
    }

    #[test]
    fn token_sum_saturates_instead_of_wrapping() {
        let rows = vec![assistant(i64::MAX, 0, None, None), assistant(i64::MAX, 0, None, None)];
        let m = aggregate(&rows);
        assert_eq!(m.input_tokens, Some(i64::MAX), "回绕成负数比不显示更糟");
    }

    #[test]
    fn a_nonzero_context_never_rounds_down_to_zero_percent() {
        // 8000 / 32768 = 24%，无所谓。真正的坑是 1 / 32768 = 0.003%。
        assert_eq!(context_used_percent(1, 32768), 1, "占着 1 token 却显示 0%，像没占");
        assert_eq!(context_used_percent(8000, 32768), 24);
    }

    #[test]
    fn an_actually_empty_context_stays_zero_percent() {
        assert_eq!(context_used_percent(0, 32768), 0, "真没用就是 0%，不抬成 1%");
        assert_eq!(context_used_percent(-5, 32768), 0);
    }

    #[test]
    fn an_unknown_or_absurd_window_does_not_divide_by_zero() {
        assert_eq!(context_used_percent(100, 0), 0);
        assert_eq!(context_used_percent(100, -1), 0);
    }

    #[test]
    fn a_full_context_cannot_exceed_one_hundred_percent() {
        assert_eq!(context_used_percent(50000, 32768), 100);
    }

    #[test]
    fn segments_keep_zero_valued_parts_because_zero_is_information() {
        let segs = context_segments(&ContextBreakdown {
            system_prompt: 242,
            tool_definitions: 3100,
            skills: 0,
            mcp: 0,
            conversation: 1800,
        });
        let keys: Vec<&str> = segs.iter().map(|s| s.key).collect();
        assert_eq!(
            keys,
            vec!["system_prompt", "tool_definitions", "skills", "mcp", "conversation"]
        );
        // 「没挂技能」是一条要显示的信息，不能因为是 0 就把这一段藏掉。
        let skills = segs.iter().find(|s| s.key == "skills").expect("skills 段必须在");
        assert_eq!(skills.chars, 0);
    }

    #[test]
    fn a_tool_spec_counts_name_description_and_parameters() {
        let spec = quill_provider::ToolSpec::new(
            "read_note",
            "读一个笔记文件",
        )
        .with_parameters(serde_json::json!({"type": "object"}));
        let chars = tool_spec_chars(&spec);
        assert!(
            chars > "read_note".chars().count() + "读一个笔记文件".chars().count(),
            "参数 JSON 也是常驻开销，只算描述会低估：{chars}"
        );
    }
}