//! MBTI 的 HTTP 接线。
//!
//! 端点形状对齐 Octop（`.octop-ref/octop/src/octop/api/routers/mbti.py`
//! 与 `dashboard/src/api/modules/mbti.ts`）的 `/mbti/current`、`/mbti/types`、
//! `/mbti/test/questions`、`/mbti/test/submit`、`/mbti/apply`，**只读对齐**路径；
//! 两处是我们自己的：
//!
//! - 挂在 `/api/mbti` 而不是上游的 `/api/...`（上游那个 router 挂在
//!   Octop 自己的前缀下）。全站统一 `/api` 开头。
//! - `/api/mbti/apply` **必须带 `expert_id`**（见 [`mbti::apply`] 的理由）：
//!   本项目的人格挂在专家上，没有「当前智能体」这个默认目标。
//! - 多一个 `/api/mbti/history`：Octop 只留「当前类型」一个字段，
//!   本项目把每次测评都存下来（见迁移 `0010_mbti.sql`）。

use axum::extract::{Query, State};
use axum::Json;
use serde_json::{json, Map, Value};

use crate::api_experts::{map_agent_error, only_keys, registry};
use crate::auth::AuthUser;
use crate::body::JsonBody;
use crate::error::ApiError;
use crate::mbti::{apply as persona, profiles, questions, score, store};
use crate::state::AppState;

/// 一次测评结果。`profile` 为 null 表示这个 code 没有档案（程序问题）。
fn result_json(row: &store::ResultRow, zh: bool) -> Value {
    store::to_public(
        row,
        profiles::get(&row.code).map(|p| score::profile_json(p, zh)),
    )
}

/// `lang=zh|en`。**缺省中文** —— 判据与错误信息全是中文，
/// 没传 lang 时默认给中文才是可预期的。
fn zh_of(q: &Value) -> bool {
    match q.get("lang").and_then(Value::as_str) {
        Some(s) => !s.eq_ignore_ascii_case("en"),
        None => true,
    }
}

/// `GET /api/mbti/types` —— 16 型档案。
pub async fn types(
    State(_state): State<AppState>,
    user: AuthUser,
    Query(q): Query<Value>,
) -> Result<Json<Value>, ApiError> {
    let _ = &user;
    let zh = zh_of(&q);
    Ok(Json(json!({
        "types": profiles::PROFILES
            .iter()
            .map(|p| score::profile_json(p, zh))
            .collect::<Vec<Value>>()
    })))
}

/// `GET /api/mbti/questions` —— 28 道题。
pub async fn test_questions(
    State(_state): State<AppState>,
    user: AuthUser,
    Query(q): Query<Value>,
) -> Result<Json<Value>, ApiError> {
    let _ = &user;
    let zh = zh_of(&q);
    Ok(Json(json!({
        "questions": questions::QUESTIONS
            .iter()
            .map(|q| {
                json!({
                    "id": q.id,
                    "dimension": q.dimension,
                    "question": if zh { q.question_zh } else { q.question_en },
                    "option_a": if zh { q.option_a_zh } else { q.option_a_en },
                    "option_b": if zh { q.option_b_zh } else { q.option_b_en },
                })
            })
            .collect::<Vec<Value>>(),
        // 门槛随题库一起发，省得前端再写死一个 20 —— 改题库时会漏掉。
        "min_answers": questions::MIN_ANSWERS,
    })))
}

/// `GET /api/mbti/history` —— 测评历史，最新在前。
pub async fn history(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<Value>,
) -> Result<Json<Value>, ApiError> {
    let zh = zh_of(&q);
    let rows = store::list(state.db()?, user.0.user_id, store::KEEP_RECORDS).await?;
    Ok(Json(json!({
        "history": rows.iter().map(|r| result_json(r, zh)).collect::<Vec<Value>>(),
        "current": rows.first().map(|r| result_json(r, zh)).unwrap_or(Value::Null),
        "keep": store::KEEP_RECORDS,
    })))
}

/// `POST /api/mbti/test` —— 提交作答，算分并落一条记录。
///
/// **每提交一次就多一行**，哪怕结果跟上次一模一样。用户反复点「重新测」
/// 是正常行为，历史就是给他看这个的。
pub async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    only_keys(&body, &["answers", "language"], "POST /api/mbti/test")?;

    let answers = match body.get("answers") {
        Some(Value::Object(m)) => m.clone(),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "字段 answers 必须是对象（题号 → \"A\"/\"B\"），实际收到 {}。",
                match other {
                    Value::Null => "null",
                    Value::Bool(_) => "布尔值",
                    Value::Number(_) => "数字",
                    Value::String(_) => "字符串",
                    Value::Array(_) => "数组",
                    Value::Object(_) => "对象",
                }
            )))
        }
        None => return Err(ApiError::bad_request("缺少必填字段 answers。")),
    };
    let zh = !matches!(body.get("language").and_then(Value::as_str), Some(s) if s.eq_ignore_ascii_case("en"));

    let scored = score::score(&answers).map_err(|e| {
        ApiError::bad_request(format!("{e}\n下一步：答题卡上标着未答的题，补完再提交。"))
    })?;
    let profile = scored.profile.ok_or_else(|| {
        // 走到这里说明档案表少了一型（16 型被改过）。这是代码坏了不是用户错了。
        ApiError::service_unavailable(format!(
            "档案表里没有 {} 这一型。请报给管理员：16 型档案与计分逻辑对不上。",
            scored.code
        ))
    })?;

    let dims = score::dimensions_json(&scored.dimensions);
    let row_id = store::insert(
        state.db()?,
        user.0.user_id,
        &scored.code,
        &dims,
        &Value::Object(answers),
        now_ms(),
    )
    .await?;

    let row = store::get_by_row_id(state.db()?, user.0.user_id, row_id)
        .await?
        .ok_or_else(|| ApiError::storage_unavailable("刚写进去的测评记录读不回来".to_string()))?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({
            "result": result_json(&row, zh),
            "profile": score::profile_json(profile, zh),
        })),
    ))
}

/// `POST /api/mbti/apply` —— 把某次测评的人格写进一个专家的 instructions。
pub async fn apply_to_expert(
    State(state): State<AppState>,
    user: AuthUser,
    JsonBody(body): JsonBody,
) -> Result<Json<Value>, ApiError> {
    only_keys(
        &body,
        &["row_id", "expert_id", "language"],
        "POST /api/mbti/apply",
    )?;

    let row_id = match body.get("row_id") {
        Some(Value::Number(n)) => n.as_i64().ok_or_else(|| {
            ApiError::bad_request("字段 row_id 必须是整数行号。".to_string())
        })?,
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "字段 row_id 必须是数字，实际收到 {}。\n\
                 下一步：row_id 来自 `POST /api/mbti/test` 的返回，别自己编。",
                other
            )))
        }
        None => return Err(ApiError::bad_request("缺少必填字段 row_id。")),
    };
    let expert_slug = match body.get("expert_id") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(_) => {
            return Err(ApiError::bad_request(
                "字段 expert_id 必须是非空字符串（专家标识）。".to_string()
            ))
        }
        None => {
            return Err(ApiError::bad_request(
                "缺少必填字段 expert_id。\n\
                 下一步：人格要落到某个具体专家身上 —— 本项目的人格挂在专家上，\
                 没有「默认智能体」这个位置，所以必须点名。",
            ))
        }
    };
    let zh = !matches!(body.get("language").and_then(Value::as_str), Some(s) if s.eq_ignore_ascii_case("en"));

    let row = store::get_by_row_id(state.db()?, user.0.user_id, row_id)
        .await?
        .ok_or_else(|| ApiError::not_found("找不到这次测评记录（row_id 不对，或不是你的记录）。"))?;
    let profile = profiles::get(&row.code).ok_or_else(|| {
        ApiError::service_unavailable(format!("档案表里没有 {} 这一型。", row.code))
    })?;

    let id = quill_adapters::ExpertId::parse(&expert_slug).map_err(|e| {
        ApiError::bad_request(format!("专家标识 {expert_slug:?} 非法（{e}）。"))
    })?;
    let reg = registry(&state)?;
    let expert = map_agent_error("读取专家", reg.get_visible(&user.0.user_id, &id))?;

    let placement = persona::compose(expert.instructions(), profile, zh);
    if placement.text.len() > persona::MAX_INSTRUCTIONS {
        return Err(ApiError::bad_request(format!(
            "写入后的人格正文会有 {} 字符，超过上限 {}。\n\
             下一步：先把该专家自己写的正文压短，再应用人格。",
            placement.text.len(),
            persona::MAX_INSTRUCTIONS
        )));
    }

    map_agent_error(
        "写入人格正文",
        reg.set_instructions(&user.0.user_id, &id, placement.text.clone()),
    )?;
    store::mark_applied(state.db()?, user.0.user_id, row_id, &expert_slug).await?;

    let mut out = Map::new();
    out.insert("row_id".into(), json!(row_id));
    out.insert("code".into(), json!(row.code));
    out.insert("expert_id".into(), json!(expert_slug));
    out.insert(
        "instructions".into(),
        Value::String(placement.text),
    );
    out.insert(
        "replaced".into(),
        json!(placement.previous.is_some()),
    );
    out.insert("previous_code".into(), json!(placement.previous));
    Ok(Json(Value::Object(out)))
}

/// unix epoch 毫秒。与 `api_channels::now_ms` 同一口径。
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 判据与内部测试用：题库是固定的常量，出问题就是编译期的事。
#[cfg(test)]
mod tests {
    use super::*;

    fn q(pairs: &[(&str, &str)]) -> Value {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), json!(*v)))
            .collect::<Map<String, Value>>()
            .into()
    }

    #[test]
    fn lang_defaults_to_chinese_and_honours_en() {
        assert!(zh_of(&q(&[])));
        assert!(zh_of(&q(&[("lang", "zh")])));
        assert!(!zh_of(&q(&[("lang", "en")])));
        assert!(!zh_of(&q(&[("lang", "EN")])));
        // 认不出的语言给中文，而不是给英文 —— 中文是默认项。
        assert!(zh_of(&q(&[("lang", "klingon")])));
    }
}