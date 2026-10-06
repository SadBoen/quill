//! 专家市场：从 SkillHub 把一个技能集装成「我的专家」。
//!
//! ## 这个市场和技能包市场是**同一个上游**
//!
//! Octop 的专家市场走的是 SkillHub 的 `/api/v1/skillsets`
//! （`.octop-ref/octop/dashboard/src/api/modules/expertMarket.ts:99-112`
//! → `.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:314`）。
//! 我们早就把这两条路径接上了，只是**装成了技能包而不是专家**
//! （见 `skillhub::download_skillset` 与 `api_extensions::skill_hub_install`）。
//! 所以这里没有第二个上游客户端，直接复用 `skillhub` / `skillhub_unpack`。
//!
//! ## 第一版只有列表与安装
//!
//! 详情页不做：列表项已经带 slug / 名字 / 说明 / 场景 / 技能数，
//! 够界面显示与判断装不装了。

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::auth::AuthUser;
use crate::error::ApiError;
use crate::state::AppState;

/// 市场专家 id 的前缀。`hub-` 而不是 `skillhub-`：
/// 专家 id 上限 64 字符（`ExpertId`），而 slug 最长 128，
/// 前缀越短越不容易把一个合法 slug 顶出界。
pub(crate) const MARKET_ID_PREFIX: &str = "hub-";

/// 市场专家的 `source_template`。**不带冒号**：
/// 迁移 `0005_expert_source_template.sql` 的 CHECK 只接受 `[a-z0-9-]`。
pub(crate) const MARKET_SOURCE_PREFIX: &str = "skillhub-";

/// 一个技能集装成的专家 id。
///
/// ## 为什么不在这里截断
///
/// 截断会造出一个**看起来装成功、其实 id 撞了别人**的专家。
/// 超长就直接报错并说清上限，让用户换一个短 slug 的包。
pub(crate) fn market_expert_id(slug: &str) -> Result<quill_adapters::ExpertId, ApiError> {
    let raw = format!("{MARKET_ID_PREFIX}{slug}");
    quill_adapters::ExpertId::parse(&raw).map_err(|e| {
        ApiError::bad_request(format!(
            "专家市场里这个包的标识太长或含非法字符，派生出不了合法的专家 id（{e}）。\
             你填的是 {raw:?}（{len} 个字符，上限 64）。\
             下一步：换一个 slug 更短的包，或直接在「专家库」里按这个人格手动建一个。",
            len = raw.chars().count(),
        ))
    })
}

/// `GET /api/experts/market?page=&page_size=` —— 专家市场列表。
///
/// 认证与 `api_extensions::skill_hub_list` 同理：要打到外部服务上，
/// 逐 handler 的 `AuthUser` 是唯一的门。
pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<crate::api_extensions::HubListQuery>,
) -> Result<Response, ApiError> {
    let page = crate::skillhub::list_skillsets(q.page, q.page_size)
        .await
        .map_err(|e| crate::api_extensions::hub_error("读专家市场列表失败", e))?;

    // 「装过没有」要一次问完，不能每个条目查一次库。
    let registry = crate::api_experts::registry(&state)?;
    let visible = registry
        .list_visible(&user.0.user_id)
        .map_err(|e| crate::api_experts::agent_error_to_api("列出专家", e))?;
    let mine: std::collections::HashSet<String> =
        visible.iter().map(|e| e.id().as_str().to_string()).collect();

    let items: Vec<Value> = page
        .items
        .iter()
        .map(|s| {
            let id = format!("{MARKET_ID_PREFIX}{}", s.slug);
            json!({
                "slug": s.slug,
                "display_name": s.display_name,
                "summary": s.summary,
                "scene": s.scene,
                "skill_slugs": s.skill_slugs,
                "skill_count": s.skill_count,
                "installed": mine.contains(&id),
            })
        })
        .collect();

    Ok(Json(json!({
        "host": crate::skillhub::host(),
        "items": items,
        // **上游没给就是 null**，不用本页条数顶替 —— 那是在编一个总数。
        "total": page.total,
        "page": page.page,
        "page_size": page.page_size,
    }))
    .into_response())
}

/// `POST /api/experts/market/{slug}/install` —— 把技能集装成我的专家。
///
/// 三件事，顺序固定：
/// 1. 建专家（人格来自包里那一篇 `skillsets/*.md`）；
/// 2. 把它点名的技能逐个装上，**一律不启用**；
/// 3. 如实回报哪一个装上了、哪一个本来就有、哪一个没装成。
///
/// 技能失败**不废整单**：人格已经在库里了，把整单退回去等于
/// 「因为附带的两个技能没下下来，主人格也一并消失」。
pub async fn install(
    State(state): State<AppState>,
    user: AuthUser,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    let safe = crate::skillhub::validate_slug(&slug)
        .map_err(|e| crate::api_extensions::hub_error("安装市场专家失败", e))?;
    let id = market_expert_id(&safe)?;
    let registry = crate::api_experts::registry(&state)?;

    // 已经装过：**不重复下载**，也不覆盖用户可能已经改过的人格。
    // 幂等重试返回的是库里那份现状，不是包里那份。
    if let Ok(existing) = registry.get_visible(&user.0.user_id, &id) {
        return Ok(Json(json!({
            "already_installed": true,
            "expert": crate::api_experts::expert_json(&existing),
            // 人格正文取自库里那份，所以「来自包里哪个文件」如实说不知道。
            "persona": { "source_file": null, "chars": existing.instructions().chars().count() },
            "skills": [],
        }))
        .into_response());
    }

    let bytes = crate::skillhub::download_skillset(&safe)
        .await
        .map_err(|e| crate::api_extensions::hub_error(&format!("下载专家包 {safe} 失败"), e))?;

    let contents = crate::skillhub_unpack::skillset_contents(&bytes, &safe)
        .map_err(|e| ApiError::bad_request(e.message()))?;

    // 技能名单的来源，按可信度排：包里的 manifest → 上游详情接口。
    // 两条都拿不到就一个都不装，**不猜**。
    let mut skill_slugs: Vec<String> = contents
        .manifest
        .as_ref()
        .map(|m| m.referenced_slugs())
        .unwrap_or_default();
    if skill_slugs.is_empty() {
        if let Ok(detail) = crate::skillhub::fetch_skillset(&safe).await {
            skill_slugs = detail.skill_slugs;
        }
    }

    let new = quill_agent::NewExpert {
        id,
        display_name: display_name_of(&contents.manifest, &safe),
        description: contents.persona.body.trim().to_string(),
        instructions: contents.persona.body.clone(),
        model: None,
        source_template: Some(format!("{MARKET_SOURCE_PREFIX}{safe}")),
    };
    let expert = registry
        .create_user_expert(user.0.user_id, new)
        .map_err(|e| crate::api_experts::agent_error_to_api("从市场安装专家", e))?;

    let mut skills: Vec<Value> = Vec::new();
    for s in &skill_slugs {
        let entry = match crate::api_extensions::install_one_skill(&state, user.0.user_id, s).await {
            Ok(r) => json!({
                "slug": r.slug,
                "status": if r.already_present { "already_present" } else { "installed" },
                "body_file": r.body_file,
            }),
            Err(e) => json!({
                "slug": s,
                "status": "failed",
                // 带上真实原因：用户要能自己判断是网络问题还是包坏了。
                "reason": e.detail(),
            }),
        };
        skills.push(entry);
    }

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({
            "already_installed": false,
            "expert": crate::api_experts::expert_json(&expert),
            "persona": {
                "source_file": contents.persona.source_file,
                "chars": contents.persona.body.chars().count(),
            },
            "skills": skills,
        })),
    )
        .into_response())
}

/// 显示名：manifest 优先，其次上游列表里的名字，最后退回 slug。
///
/// 退回 slug 不是编名字 —— 它就是上游唯一的标识，如实摆出来。
fn display_name_of(manifest: &Option<crate::skillhub::HubManifest>, slug: &str) -> String {
    if let Some(m) = manifest {
        if let Some(d) = m.display_name() {
            return d.to_string();
        }
    }
    slug.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_ids_carry_a_prefix_that_keeps_the_slug_legal() {
        let id = market_expert_id("pdf-toolkit").expect("合法 slug 应能派生出 id");
        assert_eq!(id.as_str(), "hub-pdf-toolkit");
    }

    #[test]
    fn a_slug_too_long_for_an_expert_id_is_refused_rather_than_truncated() {
        // 截断会造出撞名的专家：两个长 slug 截完前缀后可能撞成同一个 id，
        // 那就是「装成功了、装的是别人那个」。必须报错。
        let long = "a".repeat(70);
        let err = market_expert_id(&long).expect_err("超长 slug 必须报错");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
        let body = format!("{}{}", err.detail(), err.next_step());
        assert!(body.contains("64"), "报错必须说清长度上限：{body}");
        // 回显的是**完整**的派生 id：截一个短的塞进报错里，
        // 用户会拿它去建专家，然后撞上别人的。
        assert!(
            body.contains(&format!("{MARKET_ID_PREFIX}{long}")),
            "报错应回显完整的派生 id：{body}"
        );
    }

    #[test]
    fn source_template_prefix_survives_the_column_check_constraint() {
        // 迁移 0005 只允许 [a-z0-9-]：写成 `skillhub:xxx` 会被数据库拒掉。
        let v = format!("{MARKET_SOURCE_PREFIX}{}", "pdf-toolkit");
        assert!(
            v.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "source_template 含 CHECK 不允许的字符：{v}"
        );
        assert!(v.len() <= 64, "source_template 也要过 64 上限：{v}");
    }
}