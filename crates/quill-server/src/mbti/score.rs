//! 计分：把 28 题的作答折成 4 根轴，再折成一个四位 code。
//!
//! 算法照抄 Octop 的 `_score_answers`
//! （`.octop-ref/octop/src/octop/api/routers/mbti.py:617-673`，**只读对齐**），
//! 三条容易记错的：
//!
//! 1. **强度百分比不是「选 A 的比例」。** 是 `50 + 胜出极占比 * 35`，
//!    再夹到 `[50, 85]`。全选一边也只有 85%。别「修正」成真实比例。
//! 2. **平手算先出的那一极**（`a_count >= b_count`）。四题对四题会得 E/S/T/J。
//! 3. **某轴一题没答**，退化成该轴第一极 + 50 分，而不是不计入。
//!
//! 门槛 20 题同上游：答太少没有意义，直接报错而不是给个结果。

use serde_json::{json, Map, Value};

use super::profiles::{self, Dimensions, Pole, Profile};
use super::questions::{self, AXES, MIN_ANSWERS};

/// 一道题没答够 —— 调用方该返回 400。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooFewAnswers {
    pub answered: usize,
    pub required: usize,
}

impl std::fmt::Display for TooFewAnswers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "至少要答 {} 题才出结果，目前只答了 {} 题。",
            self.required, self.answered
        )
    }
}

#[derive(Debug)]
pub struct Scored {
    pub code: String,
    pub dimensions: Dimensions,
    /// 命中的档案。code 拼错或档案缺失时是 `None` —— 这属程序问题，
    /// 不是用户问题，所以让调用方决定报 500 还是别的。
    pub profile: Option<&'static Profile>,
}

/// 算四根轴的强度，不拼 code（档案卡展示用）。
pub fn dimensions_from(answers: &Map<String, Value>) -> Result<Dimensions, TooFewAnswers> {
    let (counts, _answered) = tally(answers);
    Ok(Dimensions {
        ei: axis(&counts, "EI"),
        sn: axis(&counts, "SN"),
        tf: axis(&counts, "TF"),
        jp: axis(&counts, "JP"),
    })
}

/// 完整计分。
///
/// `answers` 是 `{"1": "A", "2": "B", ...}`。**认不出的题号与选项一律跳过**
/// （照上游 `mbti.py:630-640`）：前端漏传一题不该让整次测评失败，
/// 但漏得太多要由 `MIN_ANSWERS` 拦住。
pub fn score(answers: &Map<String, Value>) -> Result<Scored, TooFewAnswers> {
    let (counts, answered) = tally(answers);
    if answered < MIN_ANSWERS {
        return Err(TooFewAnswers {
            answered,
            required: MIN_ANSWERS,
        });
    }

    let dimensions = Dimensions {
        ei: axis(&counts, "EI"),
        sn: axis(&counts, "SN"),
        tf: axis(&counts, "TF"),
        jp: axis(&counts, "JP"),
    };
    let mut code = String::with_capacity(4);
    for d in [
        &dimensions.ei,
        &dimensions.sn,
        &dimensions.tf,
        &dimensions.jp,
    ] {
        code.push_str(d.pole);
    }
    Ok(Scored {
        profile: profiles::get(&code),
        code,
        dimensions,
    })
}

/// 每轴两极的票数，外加有效作答总数。
fn tally(answers: &Map<String, Value>) -> ([[i64; 2]; 4], usize) {
    let mut counts = [[0i64; 2]; 4];
    let mut answered = 0usize;

    for (k, v) in answers {
        let Ok(id) = k.parse::<i64>() else { continue };
        let Some(q) = questions::get(id) else {
            continue;
        };
        let Some(axis_idx) = AXES.iter().position(|a| a.0 == q.dimension) else {
            continue;
        };

        let choice = match v.as_str().map(str::trim).map(str::to_ascii_uppercase) {
            Some(c) if c == "A" => 0usize,
            Some(c) if c == "B" => 1usize,
            _ => continue,
        };
        // 防御：题库里若有题的两极与轴对不上，这题当没答。题库自测已经盯住
        // 这条，真发生了是代码坏了，不该 panic 把整个接口带下去。
        let pole = [q.a_pole, q.b_pole][choice];
        if pole != AXES[axis_idx].1 && pole != AXES[axis_idx].2 {
            continue;
        }
        counts[axis_idx][choice] += 1;
        answered += 1;
    }
    (counts, answered)
}

/// 单根轴：谁赢、赢多少。
fn axis(counts: &[[i64; 2]; 4], name: &str) -> Pole {
    let idx = AXES.iter().position(|a| a.0 == name).unwrap_or(0);
    let first = counts[idx][0];
    let second = counts[idx][1];
    let (first_pole, second_pole) = (AXES[idx].1, AXES[idx].2);

    let total = first + second;
    if total == 0 {
        return Pole {
            pole: first_pole,
            pct: 50,
        };
    }
    let dominant_is_first = first >= second;
    let dominant = if dominant_is_first { first } else { second };
    let pct = (50.0_f64 + (dominant as f64 / total as f64) * 35.0).round() as i64;
    Pole {
        pole: if dominant_is_first {
            first_pole
        } else {
            second_pole
        },
        pct: pct.clamp(50, 85),
    }
}

/// 结果转前端形状。
pub fn dimensions_json(d: &Dimensions) -> Value {
    json!({
        "ei": [d.ei.pole, d.ei.pct],
        "sn": [d.sn.pole, d.sn.pct],
        "tf": [d.tf.pole, d.tf.pct],
        "jp": [d.jp.pole, d.jp.pct],
    })
}

/// 档案转前端形状。`zh` 决定取中文还是英文字段。
pub fn profile_json(p: &Profile, zh: bool) -> Value {
    let mut behaviors = Map::new();
    for (key, text) in profiles::behavior_items(p, zh) {
        behaviors.insert(key.to_string(), Value::String(text.to_string()));
    }
    json!({
        "code": p.code,
        "name": if zh { p.name_zh } else { p.name_en },
        "nickname": p.nickname_zh,
        "summary": if zh { p.summary_zh } else { p.summary_en },
        "descriptors": if zh { p.descriptors_zh } else { p.descriptors_en },
        "color": p.color,
        "symbol": p.symbol,
        "dimensions": dimensions_json(&p.dimensions),
        "behavior": Value::Object(behaviors),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(picks: &[(&str, &str)]) -> Map<String, Value> {
        picks
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect()
    }

    /// 每题都答，两极各半。
    fn half_and_half() -> Map<String, Value> {
        let mut m = Map::new();
        for q in questions::QUESTIONS {
            m.insert(q.id.to_string(), Value::String("A".to_string()));
        }
        m
    }

    #[test]
    fn all_a_gives_the_first_pole_of_every_axis_at_the_cap() {
        let s = score(&half_and_half()).expect("28 题够门槛");
        assert_eq!(s.code, "ESTJ");
        for d in [
            &s.dimensions.ei,
            &s.dimensions.sn,
            &s.dimensions.tf,
            &s.dimensions.jp,
        ] {
            assert_eq!(d.pct, 85, "全选一边应顶到上限 85");
        }
    }

    #[test]
    fn all_b_gives_the_second_pole_of_every_axis() {
        let mut m = Map::new();
        for q in questions::QUESTIONS {
            m.insert(q.id.to_string(), Value::String("B".to_string()));
        }
        let s = score(&m).unwrap();
        assert_eq!(s.code, "INFP");
    }

    #[test]
    fn a_tied_axis_takes_the_first_pole_and_two_thirds_strength() {
        // 每轴只答两题，一 A 一 B。其余五题不答（这条不设门槛，走 dimensions_from）。
        //
        // 平手时百分比**不是 50** —— 是 `50 + 1/2*35 = 67.5 → 68`。
        // 50 只在某轴一题没答时出现（见下面那条用例）。这是上游的口径，
        // 记在这里是因为它最容易被「顺手改成 50」。
        let mut per_axis: std::collections::BTreeMap<&str, Vec<i64>> = Default::default();
        for q in questions::QUESTIONS {
            per_axis.entry(q.dimension).or_default().push(q.id);
        }
        let mut tie = Map::new();
        for ids in per_axis.values() {
            tie.insert(ids[0].to_string(), Value::String("A".to_string()));
            tie.insert(ids[1].to_string(), Value::String("B".to_string()));
        }
        let d = dimensions_from(&tie).unwrap();
        // 平手取先出的那一极（E/S/T/J），不是后出的。
        assert_eq!((d.ei.pole, d.ei.pct), ("E", 68));
        assert_eq!((d.sn.pole, d.sn.pct), ("S", 68));
        assert_eq!((d.tf.pole, d.tf.pct), ("T", 68));
        assert_eq!((d.jp.pole, d.jp.pct), ("J", 68));
    }

    #[test]
    fn an_axis_with_no_answers_falls_back_to_its_first_pole_at_fifty() {
        // JP 一题不答，其余全答 A。
        let mut m = Map::new();
        for q in questions::QUESTIONS {
            if q.dimension != "JP" {
                m.insert(q.id.to_string(), Value::String("A".to_string()));
            }
        }
        let s = score(&m).unwrap();
        assert_eq!(s.code, "ESTJ");
        assert_eq!((s.dimensions.jp.pole, s.dimensions.jp.pct), ("J", 50));
        assert_eq!(s.dimensions.ei.pct, 85);
    }

    #[test]
    fn too_few_answers_is_an_error_naming_both_numbers() {
        let e = score(&answers(&[("1", "A"), ("2", "B")])).unwrap_err();
        assert_eq!(e.answered, 2);
        assert_eq!(e.required, MIN_ANSWERS);
        let msg = e.to_string();
        assert!(
            msg.contains("20") && msg.contains("2"),
            "消息里两个数都要有：{msg}"
        );
    }

    #[test]
    fn unknown_ids_and_junk_choices_are_skipped_not_fatal() {
        let mut m = half_and_half();
        m.insert("9999".to_string(), Value::String("A".to_string()));
        m.insert("abc".to_string(), Value::String("A".to_string()));
        let s = score(&m).unwrap();
        assert_eq!(s.code, "ESTJ", "多出的键不该影响结果");
        assert_eq!(s.dimensions.ei.pct, 85, "28 题全 A 仍是满格");
    }

    #[test]
    fn a_junk_choice_does_not_count_as_an_answer() {
        // 题号不在题库里、以及选项不是 A/B 的，都**不算作答**。
        // 这条用门槛来证明：门槛数的是真作答，不是收到的键数。
        let build = |junk: usize| -> Map<String, Value> {
            let mut m = half_and_half();
            for q in questions::QUESTIONS.iter().take(junk) {
                m.insert(q.id.to_string(), Value::String("C".to_string()));
            }
            m
        };
        // 20 题真作答 → 过
        assert!(score(&build(8)).is_ok(), "剩 20 题有效，应当通过门槛");
        // 19 题真作答 → 不过
        let e = score(&build(9)).unwrap_err();
        assert_eq!(e.answered, 19);
        assert_eq!(e.required, MIN_ANSWERS);
    }

    #[test]
    fn a_real_code_resolves_to_a_profile() {
        let s = score(&half_and_half()).unwrap();
        let p = s.profile.expect("ESTJ 档案必须在");
        assert_eq!(p.code, s.code);
    }
}
