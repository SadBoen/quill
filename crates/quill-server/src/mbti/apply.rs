//! 「把这个人格应用到某个专家」—— 唯一一处改动别人数据的地方。
//!
//! ## 为什么不是 SOUL.md
//!
//! Octop 把风格段渲染进工作区的 SOUL.md
//! （`.octop-ref/octop/src/octop/api/routers/mbti.py:58`）。本项目**没有那条链路**
//! —— 人格正文是 `experts.instructions`，早就通了（`experts.instructions` →
//! `resolve_persona` → prompt）。所以这里是直接改 instructions。
//!
//! ## 为什么必须点名一个专家
//!
//! 上游挂在 agent 上（本项目的 agent 就是登录用户），所以「当前智能体」是个
//! 说得清的默认目标。本项目的人格挂在**专家**上，一个用户有多个专家。
//! 没有「当前专家」这个东西，硬造一个就等于替用户决定把 INFP 的语气写进哪个
//! 专家 —— 宁可多让他点一次。
//!
//! ## 幂等
//!
//! 段落在 instructions 里被一对标记包着（[`BEGIN`] / [`END`]）。重复应用同一个
//! 类型是**替换**而不是追加：不这么做的话，用户每点一次「应用」人格正文就长一截，
//! 点二十次就撞上 20000 字符上限，且没有任何办法在界面里删掉。

use super::profiles::{self, Profile};

pub const BEGIN: &str = "<!-- mbti -->";
pub const END: &str = "<!-- /mbti -->";

/// instructions 的上限，与迁移 `0004_expert_persona.sql` 的 CHECK 一致。
pub const MAX_INSTRUCTIONS: usize = 20000;

/// 人格段落在 instructions 里的存在情况。
pub struct Placement {
    pub text: String,
    /// `Some(code)` 表示原来就有，值是当时的类型。
    pub previous: Option<String>,
}

/// 拆出已有的人格段，返回剩下的正文。找不到**成对**标记时原样返回。
///
/// 「成对」是刻意的：只有 BEGIN 没有 END（手动编辑搞坏）时从它后面继续找，
/// 免得拿一个没闭合的 BEGIN 去匹配后面正常段的 END，把一整段正文当成人格删掉。
pub fn strip(existing: &str) -> (String, Option<String>) {
    let mut from = 0usize;
    while let Some(rel) = existing[from..].find(BEGIN) {
        let b = from + rel;
        let after = &existing[b + BEGIN.len()..];
        let Some(e) = after.find(END) else {
            from = b + BEGIN.len();
            continue;
        };
        let segment = &after[..e];
        let head = existing[..b].trim_end();
        let tail = after[e + END.len()..].trim_start();

        let code = segment
            .lines()
            .find_map(|l| {
                let rest = l.trim().strip_prefix("**MBTI")?;
                let v = rest
                    .trim()
                    .trim_start_matches([' ', ':', '*'])
                    .trim_end_matches('*')
                    .trim();
                // 行是 `**MBTI: INTJ · 建筑师**`，取头一个词就是 code。
                v.split_whitespace().next().map(str::to_string)
            })
            .filter(|c| c.len() == 4);

        let kept = match (head.is_empty(), tail.is_empty()) {
            (true, true) => String::new(),
            (true, false) => tail.to_string(),
            (false, true) => head.to_string(),
            (false, false) => format!("{head}\n\n{tail}"),
        };
        return (kept, code);
    }
    (existing.to_string(), None)
}

/// 把人格段拼进 instructions：已有就原地替换，没有就追加到末尾。
pub fn compose(existing: &str, p: &Profile, zh: bool) -> Placement {
    let (kept, previous) = strip(existing);
    let segment = render(p, zh);
    let text = if kept.is_empty() {
        segment
    } else {
        format!("{kept}\n\n{segment}")
    };
    Placement { text, previous }
}

/// 人格段正文。
fn render(p: &Profile, zh: bool) -> String {
    let mut out = String::with_capacity(512);
    out.push_str(BEGIN);
    out.push('\n');
    out.push_str(&format!(
        "**MBTI: {} · {}**\n\n",
        p.code,
        if zh { p.name_zh } else { p.name_en }
    ));

    let labels = if zh {
        [
            "回答风格",
            "闲聊",
            "遇到分歧",
            "创意",
            "情绪",
            "规划",
        ]
    } else {
        [
            "Answer style",
            "Casual chat",
            "Conflict",
            "Creativity",
            "Emotion",
            "Planning",
        ]
    };
    for ((key, text), label) in profiles::behavior_items(p, zh).into_iter().zip(labels) {
        out.push_str(&format!("- {label}（{key}）：{text}\n"));
    }
    out.push_str(END);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> &'static Profile {
        profiles::get("INTJ").expect("INTJ 档案必须在")
    }

    #[test]
    fn compose_on_empty_yields_exactly_the_segment() {
        let r = compose("", p(), true);
        assert!(r.previous.is_none());
        assert!(r.text.starts_with(BEGIN));
        assert!(r.text.ends_with(END));
    }

    #[test]
    fn applying_the_same_type_twice_does_not_grow_the_text() {
        let once = compose("", p(), true).text;
        let twice = compose(&once, p(), true);
        assert_eq!(twice.text, once, "同类型重复应用必须幂等");
        assert_eq!(once.matches(BEGIN).count(), 1);
    }

    #[test]
    fn switching_type_replaces_and_reports_the_previous_one() {
        let first = compose("", p(), true);
        let infp = profiles::get("INFP").expect("INFP 档案必须在");
        let second = compose(&first.text, infp, true);
        assert_eq!(second.previous.as_deref(), Some("INTJ"));
        assert!(second.text.contains("INFP"), "新类型要写进去");
        assert!(!second.text.contains("MBTI: INTJ"), "旧类型要被换掉");
        assert_eq!(second.text.matches(BEGIN).count(), 1);
    }

    #[test]
    fn user_written_text_around_the_segment_survives_replacement() {
        let base = "先问口径。\n\n不要写代码。";
        let with_seg = compose(base, p(), true).text;
        let infp = profiles::get("INFP").unwrap();
        let again = compose(&with_seg, infp, true).text;
        assert!(again.starts_with("先问口径。"));
        assert!(again.contains("不要写代码。"));
        assert!(again.contains("MBTI: INFP"));
        assert!(!again.contains("MBTI: INTJ"));
    }

    #[test]
    fn strip_leaves_plain_instructions_untouched() {
        let (kept, code) = strip("就按默认来。");
        assert_eq!(kept, "就按默认来。");
        assert_eq!(code, None);
    }

    #[test]
    fn a_half_written_marker_is_treated_as_absent_rather_than_trusted() {
        // 只有 BEGIN 没有 END：手改坏的正文。
        let (kept, code) = strip("前面\n\n<!-- mbti -->\n**MBTI: ESTJ**\n");
        assert_eq!(code, None);
        assert!(kept.contains("前面"));
        // 重新 compose 会得到一段完整且只有一个标记的正文。
        let r = compose("前面\n\n<!-- mbti -->\n**MBTI: ESTJ**\n", p(), true);
        assert_eq!(r.text.matches(BEGIN).count(), 2, "坏的那段留着也算一处，但它没被当成已存在的人格");
        assert_eq!(r.previous, None);
    }

    #[test]
    fn every_behavior_item_is_rendered_in_both_languages() {
        for p in profiles::PROFILES {
            for zh in [true, false] {
                let seg = render(p, zh);
                for (key, _) in profiles::behavior_items(p, zh) {
                    assert!(seg.contains(key), "{} 缺 {key}", p.code);
                }
                assert!(seg.contains(p.code), "{} 段里应写出自己的 code", p.code);
            }
        }
    }
}