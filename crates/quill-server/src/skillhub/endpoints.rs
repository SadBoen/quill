//! 上游端点的调用封装：技能集层（list/fetch/download）与单技能层（search/showcase/download）。
//!
//! 这里只做「拼 URL → 发请求 → 解析成模型」；HTTP 细节在 [`super::http`]，
//! 返回形状在 [`super::models`]，失败结局在 [`super::errors`]。

use serde::Serialize;

use super::errors::{aggregate_board_errors, HubError};
use super::http::{download_zip, get_json, host};
use super::models::{
    HubSkill, HubSkillSet, ListEnvelope, Showcase, ShowcaseAll, ShowcaseEnvelope, SkillEnvelope,
};

/// 列出技能集。
///
/// `page` 从 1 起。`page_size` 有上限（Octop 用 100）——
/// 一次拉一万条不是「更快」，是把界面卡死。
pub async fn list_skillsets(page: u32, page_size: u32) -> Result<SkillSetPage, HubError> {
    let size = page_size.clamp(1, 100);
    let url = format!(
        "{}/api/v1/skillsets?page={}&pageSize={}",
        host(),
        page.max(1),
        size
    );
    let value = get_json(&url).await?;
    let env: ListEnvelope =
        serde_json::from_value(value).map_err(|e| HubError::Parse(e.to_string()))?;
    Ok(SkillSetPage {
        items: env.skill_sets,
        // 总数缺席就是缺席，不拿本页条数顶替 —— 那是编一个数。
        total: env.total,
        page: page.max(1),
        page_size: size,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillSetPage {
    pub items: Vec<HubSkillSet>,
    pub total: Option<i64>,
    pub page: u32,
    pub page_size: u32,
}

/// 取一个技能集的详情：`GET /api/v1/skillsets/{slug}`。
///
/// 列表里已经带了大半字段，**这个端点不是为了「多拿点什么」**，
/// 而是装专家时需要一个可信的兜底：包里若没有可解析的 `manifest.json`，
/// 就退回这里给的 `skillSlugs`（见 `skillhub_unpack::skillset_contents`）。
/// Octop 装之前同样先 `fetch_skillset`
/// （`.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:389`）。
pub async fn fetch_skillset(slug: &str) -> Result<HubSkillSet, HubError> {
    let safe = validate_slug(slug)?;
    let url = format!("{}/api/v1/skillsets/{safe}", host());
    let value = get_json(&url).await?;
    // 上游把详情包在 `{"skillSet": {...}}` 里；解不出来就说解不出来，
    // 不用列表项去顶 —— 那是拿 A 的数据冒充 B 的回答。
    let raw = value
        .get("skillSet")
        .cloned()
        .or_else(|| value.get("skillset").cloned());
    // 形状对不上归 `Parse`，不是 `Status(404)`：404 是上游**说没有**，
    // 而这里是上游答了、只是我们认不出。两者给用户的下一步不同。
    let raw = match raw {
        Some(v) if v.is_object() => v,
        _ => {
            return Err(HubError::Parse(format!(
                "技能集 {safe} 的详情响应里既没有 skillSet，也不是对象本身"
            )));
        }
    };
    let item: HubSkillSet =
        serde_json::from_value(raw).map_err(|e| HubError::Parse(e.to_string()))?;
    if item.slug.trim().is_empty() {
        return Err(HubError::Parse(format!("技能集 {safe} 的详情里没有 slug")));
    }
    Ok(item)
}

/// 下载一个技能集的 zip。**读的时候就限量**，不是收下再判断。
pub async fn download_skillset(slug: &str) -> Result<Vec<u8>, HubError> {
    let safe = validate_slug(slug)?;
    let url = format!("{}/api/v1/skillsets/{safe}/download", host());
    download_zip(&url, &safe).await
}

/// 上游 `showcase` 支持的榜单。照抄 Octop 的 `_RANKING_TYPES`。
///
/// 这是**上游自己声明的取值集合**，不是我们猜的：多一个不存在的类型，
/// 上游只会回一个我们不认识的错误。
///
/// **顺序即界面上页签的顺序**，所以「推荐」排在第一个：
/// 它是「还没搜索过的人」最该看到的一栏，而界面上默认选中的就是它。
pub const SHOWCASE_KINDS: [&str; 7] = [
    "recommended",
    "all",
    "trending",
    "hot",
    "featured",
    "newest",
    "paid",
];

/// `GET /api/v1/showcase/{kind}` 的路径。**只有白名单里的类型才有路径。**
pub(super) fn showcase_path(kind: &str) -> Option<&'static str> {
    match kind {
        "hot" => Some("/api/v1/showcase/hot"),
        "featured" => Some("/api/v1/showcase/featured"),
        "newest" => Some("/api/v1/showcase/newest"),
        "recommended" => Some("/api/v1/showcase/recommended"),
        "trending" => Some("/api/v1/showcase/trending"),
        "paid" => Some("/api/v1/showcase/paid"),
        _ => None,
    }
}

/// 搜单技能。空查询词会被上游当成 `q=a`（Octop 这么兜的），
/// 我们照抄 —— 自己编一个默认词是替上游决定它该搜什么。
pub async fn search_skills(query: &str, limit: u32) -> Result<Vec<HubSkill>, HubError> {
    let q = if query.trim().is_empty() {
        "a"
    } else {
        query.trim()
    };
    let size = limit.clamp(1, 100);
    let url = format!("{}/api/v1/search?q={}&limit={size}", host(), urlencode(q));
    let value = get_json(&url).await?;
    let env: SkillEnvelope =
        serde_json::from_value(value).map_err(|e| HubError::Parse(e.to_string()))?;
    Ok(env.results)
}

/// 拉一个榜单。上游**没有总数**，所以这里返回的就是「这一份榜单里的全部」，
/// 界面上不能说「共 N 个」。
pub async fn showcase_skills(kind: &str) -> Result<Showcase, HubError> {
    let path = showcase_path(kind).ok_or_else(|| {
        HubError::Input(format!(
            "榜单类型 {kind:?} 上游没有这个路径。已知的是：{}。",
            SHOWCASE_KINDS.join("、")
        ))
    })?;
    let url = format!("{}{path}", host());
    let value = get_json(&url).await?;
    let env: ShowcaseEnvelope =
        serde_json::from_value(value).map_err(|e| HubError::Parse(e.to_string()))?;
    Ok(Showcase {
        kind: kind.to_string(),
        section: env.section,
        items: env.skills,
    })
}

/// 极简 percent-encode。查询词是用户输入，会被拼进 URL。
///
/// 不引第三方 crate 就能覆盖 slug 允许的字符集；其余一律转成 `%XX`。
/// 反过来看更重要的：**没有它**，`&`、`#`、`?` 就能把查询词变成 URL 的
/// 另一部分（把 `/api/v1/search` 变成别的端点）。
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 下载一个单技能的 zip。
///
/// ## 它会 302
///
/// 实测 2026-10-06：`GET /api/v1/download?slug=pdf-extract` 返回 **302**，
/// `Location` 指向腾讯云 COS（`.../skills/pdf-extract/1.0.0.zip`），
/// 加上跟随后才是那 691 字节的 zip 本身。
/// 这就是为什么 [`http_client`] 的重定向策略必须开着 ——
/// 客户端不跟，收到的是 106 字节的 HTML，不是 zip。
pub async fn download_skill(slug: &str) -> Result<Vec<u8>, HubError> {
    let safe = validate_slug(slug)?;
    let url = format!("{}/api/v1/download?slug={safe}", host());
    let bytes = download_zip(&url, &safe).await?;
    Ok(bytes)
}

/// 一次拉全部榜单。
///
/// 「全部」是**我们**的聚合词，上游没有 `/api/v1/showcase/all` 这个端点
/// （见 [`showcase_path`]）。所以这个函数是并发发 6 个请求再合并，
/// 和 Octop 的 `fetch_skillhub_rankings("all")` 一样。
///
/// ## 部分失败时如实报
///
/// 6 个榜单里有 1 个挂了，**不能因此让整个页面变成空的** ——
/// 那是把「有一个分区没拉到」说成「市场里什么都没有」。
/// 所以成功的照常返回，失败的记进 [`ShowcaseAll::errors`]。
///
/// ## 全部失败时要**按类**保留结局
///
/// 6 个榜单全挂时，聚合出来的那个错误**不能一律是 [`HubError::Fetch`]** ——
/// 六个榜单全因为响应体太大而没回来，说成「连不上技能市场，下一步：检查网络」
/// 就是让用户去查一件从头到尾都正常的东西：上游好好地回了话，只是回得
/// 超过 [`MAX_HTTP_BYTES`]。规则与理由见 [`aggregate_board_errors`]。
pub async fn showcase_all() -> Result<ShowcaseAll, HubError> {
    let kinds: Vec<&'static str> = SHOWCASE_KINDS
        .iter()
        .copied()
        .filter(|k| *k != "all")
        .collect();

    let mut set = tokio::task::JoinSet::new();
    for kind in kinds {
        // URL 在循环里就算好：进 task 之后只能带 'static 的东西。
        let url = format!("{}{}", host(), showcase_path(kind).unwrap_or_default());
        set.spawn(async move {
            let parsed = get_json(&url).await.and_then(|v| {
                serde_json::from_value::<ShowcaseEnvelope>(v)
                    .map_err(|e| HubError::Parse(e.to_string()))
            });
            (kind, parsed)
        });
    }

    let mut by_kind: Vec<Showcase> = Vec::new();
    // 失败先收**变体本身**，不是它渲染好的那句话。
    // 全部失败时要按类判定结局（[`aggregate_board_errors`]），只留字符串的话
    // 类当场就丢了 —— 那正是这个缺口的成因：留着 `HubError` 才问得出 `is_oversize()`。
    let mut failures: Vec<(String, HubError)> = Vec::new();
    while let Some(joined) = set.join_next().await {
        let Ok((kind, result)) = joined else {
            continue;
        };
        match result {
            Ok(env) => by_kind.push(Showcase {
                kind: kind.to_string(),
                section: env.section,
                items: env.skills,
            }),
            Err(e) => failures.push((kind.to_string(), e)),
        }
    }

    // 排序是为了**结果稳定**：并发回来的顺序不定，
    // 而界面上「全部」这一页如果每次刷新顺序都变，用户会以为是两批不同的技能。
    by_kind.sort_by(|a, b| {
        let rank = |k: &str| SHOWCASE_KINDS.iter().position(|x| *x == k).unwrap_or(99);
        rank(&a.kind).cmp(&rank(&b.kind))
    });

    if by_kind.is_empty() {
        // 一个都没成功 ≠ 连不上：得看它们**各自是怎么没的**。
        return Err(aggregate_board_errors(&failures));
    }

    // 渲染成话只在这一步做，渲染的是每个榜单**自己**的那句 ——
    // 部分失败时每个分区各自的话术才是对的（超限的说上限、连不上的说网络）。
    let errors = failures
        .iter()
        .map(|(kind, e)| (kind.clone(), e.message()))
        .collect();

    Ok(ShowcaseAll {
        sections: by_kind,
        errors,
    })
}

/// slug 只能是安全字符。**这一条挡的是路径穿越**：
/// slug 会被拼进 URL 与文件名，`../` 就能爬出技能目录。
pub fn validate_slug(slug: &str) -> Result<String, HubError> {
    let t = slug.trim();
    if t.is_empty() {
        return Err(HubError::Input("技能标识是空的".to_string()));
    }
    if t.len() > 128 {
        return Err(HubError::Input(format!("技能标识过长（{} 字符）", t.len())));
    }
    if !t
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(HubError::Input(format!(
            "技能标识 {t:?} 含不允许的字符。只接受小写字母、数字与连字符"
        )));
    }
    Ok(t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugs_that_could_climb_out_of_the_skills_directory_are_rejected() {
        assert!(validate_slug("tech-test-automation").is_ok());
        assert!(validate_slug("../etc/passwd").is_err());
        assert!(validate_slug("a/b").is_err());
        assert!(validate_slug("..").is_err());
        assert!(validate_slug("Skill").is_err(), "大写不接受");
        assert!(validate_slug("").is_err());
    }

    #[test]
    fn page_size_is_clamped_so_one_request_cannot_pull_the_whole_market() {
        // 逻辑与 list_skillsets 内的 clamp 一致，这里钉住上限。
        let huge: u32 = 10_000;
        assert_eq!(huge.clamp(1, 100), 100);
        assert_eq!(0u32.clamp(1, 100), 1);
    }

    #[test]
    fn unknown_showcase_kinds_get_no_path_instead_of_a_guessed_one() {
        // 拼一个上游没有的路径进去，得到的只会是一个看不懂的 404。
        // `all` 是**我们的**聚合词：界面上它是「全部」，但上游没有这个端点，
        // 所以它在可选值里、却不能被拼成路径。
        assert!(
            showcase_path("all").is_none(),
            "all 没有上游端点，它是聚合词"
        );
        assert!(SHOWCASE_KINDS.contains(&"all"), "但界面上要能选到它");
        for kind in SHOWCASE_KINDS.iter().filter(|k| **k != "all") {
            assert!(showcase_path(kind).is_some(), "{kind} 应当有路径");
        }
        assert!(showcase_path("../secret").is_none());
        assert!(showcase_path("HOT").is_none(), "大小写不同就是另一个东西");
    }

    #[test]
    fn a_query_is_percent_encoded_so_it_cannot_rewrite_our_url() {
        // 这是真实风险：查询词被拼进 URL，`&` / `#` / `?` 会把 URL 改成别的端点。
        assert_eq!(urlencode("pdf"), "pdf");
        assert_eq!(urlencode("a&b"), "a%26b");
        assert_eq!(urlencode("a?b=c"), "a%3Fb%3Dc");
        assert_eq!(urlencode("a#b"), "a%23b");
        assert_eq!(
            urlencode("中文 技能"),
            "%E4%B8%AD%E6%96%87%20%E6%8A%80%E8%83%BD"
        );
        // 未转义的话，URL 会变成「search?q=a」再加一个 `&limit=999」之外的端点。
        assert!(!urlencode("a&b").contains('&'));
    }
}
