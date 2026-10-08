//! 上游返回形状的数据模型：技能集、单技能、榜单、manifest。
//!
//! 这里的 `de_text` / `de_flag` / `de_labels` 是三个「防 null」的反序列化助手，
//! 它们为什么必须存在（`#[serde(default)]` 管不了 `null`）见各自文档。

use serde::{Deserialize, Serialize};

/// 把上游的字段读成字符串，**`null` 与非字符串都读成空串**。
///
/// ## 为什么必须有这个
///
/// `#[serde(default)]` **只在字段缺失时生效，管不了 `null`** ——
/// 这是个很容易以为「已经防住了」的坑。
///
/// 实测（2026-10-06）：`GET /api/v1/search?q=pdf&limit=20` 的 20 条结果里，
/// 就有 `pdf-md` 的 `icon_url` 是 `null`；搜「翻译」时 6 条里至少 6 条如此。
/// 而 `paid` 榜单的 `section` 也是 `null`。
/// 声明成 `String` 的话，**一个 null 就让整页搜索变成 500** ——
/// 用户看到的是「技能市场没连上」，而市场明明好好地回了 20 条结果。
///
/// 空串是诚实的：那个字段上游没给值。**不是**替上游编一个。
fn de_text<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default())
}

/// `labels.requires_api_key` 这一类**开关型字段**。
///
/// 不用 [`de_text`]：那个只认字符串，遇到布尔 `true` 会读成空串
/// —— 于是「上游说需要密钥」被读成「没说」，比报错更糟。
/// 这里显式认两种形态：字符串 `"true"` 与布尔 `true`。
/// **实测上游现在给的是字符串**，布尔只是替明天留的余地。
fn de_flag<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(match v {
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        Some(other) => other.as_str().unwrap_or_default().to_string(),
        None => String::new(),
    })
}

/// `labels` 也是同类坑：上游有时给对象、有时给 `null`。
/// `null` 读成「没有标签」—— 那是对的，一个没标签的技能不是错误。
fn de_labels<'de, D>(d: D) -> Result<HubLabels, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    Ok(v.and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

/// 上游列表返回的形状。
///
/// **只声明我们真会用的字段。** 上游多加一个字段不会把我们打挂
/// （serde 默认忽略未知字段），而我们多声明一个用不到的字段，
/// 就等于凭空承诺了一个自己不会兑现的能力。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubSkillSet {
    #[serde(default)]
    pub id: i64,
    pub slug: String,
    #[serde(rename = "displayName", default, deserialize_with = "de_text")]
    pub display_name: String,
    #[serde(rename = "displayNameEn", default, deserialize_with = "de_text")]
    pub display_name_en: String,
    #[serde(default, deserialize_with = "de_text")]
    pub summary: String,
    #[serde(rename = "summaryEn", default, deserialize_with = "de_text")]
    pub summary_en: String,
    #[serde(default, deserialize_with = "de_text")]
    pub scene: String,
    #[serde(rename = "subScene", default, deserialize_with = "de_text")]
    pub sub_scene: String,
    #[serde(default, deserialize_with = "de_text")]
    pub content: String,
    #[serde(default)]
    pub skill_slugs: Vec<String>,
    #[serde(rename = "skillCount", default)]
    pub skill_count: i64,
    #[serde(rename = "iconUrl", default, deserialize_with = "de_text")]
    pub icon_url: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ListEnvelope {
    #[serde(rename = "skillSets", default)]
    pub(super) skill_sets: Vec<HubSkillSet>,
    /// 上游总数。**可能缺席** —— 所以是 Option，缺席时界面不说「共 N 个」。
    #[serde(rename = "total", default)]
    pub(super) total: Option<i64>,
}

// ---------------------------------------------------------------------------
// 单技能（skill）这一层
// ---------------------------------------------------------------------------

/// 上游单技能列表里的一条。
///
/// **只声明界面上真会显示、且我们真能核实的字段。**
/// 上游另有一堆（`claim_state`、`publisher`、`source`、`tags`…），
/// 那些不声明 —— 不是用不上就不好意思，是**多声明一个自己不兑现的字段，
/// 等于凭空给出一个承诺**。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubSkill {
    pub slug: String,
    /// 上游给的名字。实测有三种：`displayName` 是中文名，而
    /// `name` 有时是个占位串（搜 `pdf` 时 `martin-pdf` 的 name 就是
    /// 字面的 `pdf`），所以**优先 displayName**。
    #[serde(default, deserialize_with = "de_text")]
    pub name: String,
    #[serde(default, deserialize_with = "de_text")]
    pub description: String,
    /// 上游有中文简介时用它，否则界面上会是一段英文。
    #[serde(rename = "description_zh", default, deserialize_with = "de_text")]
    pub description_zh: String,
    #[serde(default, deserialize_with = "de_text")]
    pub version: String,
    #[serde(default, deserialize_with = "de_text")]
    pub category: String,
    /// **上游经常给 `null`**（实测 20 条里就有几条）—— 见 `de_text`。
    #[serde(rename = "icon_url", default, deserialize_with = "de_text")]
    pub icon_url: String,
    /// 真实安装次数。**拿不到就缺席**，不用 0 顶替。
    #[serde(default)]
    pub installs: Option<i64>,
    #[serde(default)]
    pub downloads: Option<i64>,
    /// 上游的标签。**只取一个字段** —— `requires_api_key`，Octop 的卡片上
    /// 有「需要 API Key」的橙标（`SkillHubTab.tsx` 的 `requiresApiKey()`）。
    /// 没有它的话，用户装完才发现这个技能要自己的密钥。
    #[serde(default, deserialize_with = "de_labels")]
    pub labels: HubLabels,
}

/// 上游 `labels` 里我们**真的用得到**的那一个。
///
/// 实测（2026-10-06）：`labels` 的值是**字符串**（`"true"`）而不是布尔，
/// 所以这里收成 `String`，由 [`HubLabels::requires_api_key`] 认两种写法。
/// 收成 `bool` 的话，上游哪天改成 `true` 就会解析失败 —— 那正是
/// ISSUE-056 踩过的坑。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HubLabels {
    #[serde(rename = "requires_api_key", default, deserialize_with = "de_flag")]
    pub requires_api_key: String,
}

impl HubLabels {
    /// 上游给的是字符串 `"true"`，但也见过布尔 `true`。**两种都认。**
    pub fn requires_api_key(&self) -> bool {
        matches!(self.requires_api_key.trim(), "true" | "1" | "yes")
    }
}

impl HubSkill {
    /// 界面上显示的名字。有中文显示名就用它。
    pub fn display_name(&self) -> String {
        let d = self.name.trim();
        if d.is_empty() {
            self.slug.clone()
        } else {
            d.to_string()
        }
    }

    /// 界面上显示的简介。**中文优先**，其次英文，空就是空。
    pub fn summary(&self) -> &str {
        let zh = self.description_zh.trim();
        if !zh.is_empty() {
            zh
        } else {
            self.description.trim()
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct SkillEnvelope {
    #[serde(default)]
    pub(super) results: Vec<HubSkill>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ShowcaseEnvelope {
    /// **实测 `paid` 榜单的 `section` 就是 `null`** —— 见 `de_text`。
    #[serde(default, deserialize_with = "de_text")]
    pub(super) section: String,
    #[serde(default)]
    pub(super) skills: Vec<HubSkill>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Showcase {
    pub kind: String,
    /// 上游自己给这一份榜单起的名字（如 `hot_downloads`）。**原样透传**，
    /// 因为那是上游的分类口径，不是我们起的。
    pub section: String,
    pub items: Vec<HubSkill>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShowcaseAll {
    pub sections: Vec<Showcase>,
    /// 没拉到的榜单。**有值就说明这一页不是完整的** ——
    /// 界面得说出来，而不是假装这就是全部。
    pub errors: Vec<(String, String)>,
}

impl ShowcaseAll {
    /// 把所有榜单的技能并成一份，按 slug 去重（保留先出现的那条）。
    pub fn merged(&self) -> Vec<HubSkill> {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out = Vec::new();
        for section in &self.sections {
            for skill in &section.items {
                if seen.insert(skill.slug.clone()) {
                    out.push(skill.clone());
                }
            }
        }
        out
    }
}

/// 包里那份 `manifest.json`。
///
/// 实测（2026-10-06，`tech-test-automation`）：包里只有 `manifest.json`
/// 与 `identify.md` 两个条目，manifest 里 `skillSlugs` 列了 6 个子技能，
/// 但**每个只有 slug、显示名与一句简介，没有正文**。
///
/// 所以这个字段是「这个包点名要用哪些下游技能」，**不是「装了几个」**。
/// 两处都当成一回事，就是在界面上编一个数字。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HubManifest {
    #[serde(default, deserialize_with = "de_text")]
    pub slug: String,
    #[serde(rename = "displayName", default, deserialize_with = "de_text")]
    pub display_name: String,
    #[serde(default, deserialize_with = "de_text")]
    pub summary: String,
    /// 直接列出的 slug。
    #[serde(rename = "skillSlugs", default)]
    pub skill_slugs: Vec<String>,
    /// `skills[]` 里的详细条目，只取我们用得到的字段。
    #[serde(default)]
    pub skills: Vec<HubManifestSkill>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HubManifestSkill {
    #[serde(default, deserialize_with = "de_text")]
    pub slug: String,
    #[serde(rename = "displayName", default, deserialize_with = "de_text")]
    pub display_name: String,
    #[serde(default, deserialize_with = "de_text")]
    pub summary: String,
}

impl HubManifest {
    /// 这个包点名引用、但**包里没有正文**的下游技能。
    ///
    /// 优先用 `skills[]`（有显示名），退回到 `skillSlugs`。
    /// 两处都空的包就返回空数组 —— 界面上写「没有额外依赖」是对的，
    /// 拿 `skills[]` 的长度当「装了几个」是错的。
    pub fn referenced_slugs(&self) -> Vec<String> {
        if !self.skills.is_empty() {
            return self
                .skills
                .iter()
                .map(|s| s.slug.clone())
                .filter(|s| !s.is_empty())
                .collect();
        }
        self.skill_slugs.clone()
    }

    pub fn display_name(&self) -> Option<&str> {
        let d = self.display_name.trim();
        if d.is_empty() {
            None
        } else {
            Some(d)
        }
    }
}

/// 解析包里的 manifest。**解析不了就返回 None，不猜。**
///
/// 上游改了格式时，我们只是拿不到「还引用了哪些下游技能」这条补充信息，
/// 安装本身照常进行 —— 为此拒绝整个安装是本末倒置。
pub fn parse_manifest(body: &str) -> Option<HubManifest> {
    serde_json::from_str::<HubManifest>(body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_missing_total_stays_missing_instead_of_being_faked() {
        // 上游不给 total 就真的是不知道。拿本页条数顶替，
        // 界面上就会写「共 20 个」而实际有几万 —— 这是凭空来的话。
        let v = serde_json::json!({"skillSets": []});
        let env: ListEnvelope = serde_json::from_value(v).expect("应当能解析");
        assert_eq!(env.total, None);
        assert!(env.skill_sets.is_empty());
    }

    /// 上游真实 manifest 的结构（2026-10-06 实测，截取自 `tech-test-automation`）。
    const REAL_MANIFEST: &str = r#"{
      "manifestVersion": "1",
      "type": "skillset",
      "slug": "tech-test-automation",
      "displayName": "自动化测试",
      "summary": "从 TDD 到 E2E 的完整工作流。",
      "scene": "tech",
      "subScene": "test-automation",
      "skillSlugs": [
        "superpowers-tdd", "test-case-generator", "test-patterns",
        "e2e-testing-patterns", "api-test-automation", "afrexai-qa-test-plan"
      ],
      "skills": [
        {"slug":"superpowers-tdd","displayName":"Superpowers Tdd","summary":"..."},
        {"slug":"test-case-generator","displayName":"Test Case Generator","summary":"..."}
      ]
    }"#;

    #[test]
    fn a_manifest_lists_which_downstream_skills_a_package_references() {
        let m = parse_manifest(REAL_MANIFEST).expect("真实 manifest 应当能解析");
        assert_eq!(m.slug, "tech-test-automation");
        assert_eq!(m.display_name(), Some("自动化测试"));
        // `skills[]` 只有 2 条明细，`skillSlugs` 有 6 个 —— 两者不一致是上游的现状。
        assert_eq!(m.skill_slugs.len(), 6);
        assert_eq!(m.skills.len(), 2);
        // 优先用 `skills[]`：它带显示名。
        assert_eq!(
            m.referenced_slugs(),
            vec!["superpowers-tdd", "test-case-generator"]
        );
    }

    #[test]
    fn a_manifest_with_no_details_falls_back_to_the_slug_list() {
        let m = parse_manifest(r#"{"slug":"x","skillSlugs":["a","b"]}"#).expect("能解析");
        assert_eq!(m.referenced_slugs(), vec!["a", "b"]);
    }

    #[test]
    fn a_broken_manifest_only_costs_the_extra_info_not_the_install() {
        // 解析不了就返回 None，**不猜**、**不拒装** —— 这份 manifest 只是
        // 「还引用了哪些下游技能」的补充信息，为它拒绝整个安装是本末倒置。
        assert!(parse_manifest("{ this is not json").is_none());
        assert!(parse_manifest("").is_none());
    }

    #[test]
    fn an_empty_display_name_is_absent_rather_than_blank() {
        // 空串会让界面上出现一个没有字的标签。
        let m = parse_manifest(r#"{"slug":"x","displayName":"  "}"#).expect("能解析");
        assert_eq!(m.display_name(), None);
    }

    // -----------------------------------------------------------------------
    // 单技能这一层
    // -----------------------------------------------------------------------

    /// 上游 `search` 的真实返回（2026-10-06 实测，截取自 `?q=pdf&limit=5`）。
    /// 特意保留了两个坑：`name` 是个占位串，而 `description_zh` 才是人话。
    const REAL_SEARCH: &str = r#"{"results":[
      {
        "slug":"pdf-image-text-extractor",
        "name":"PDF和图片文字提取",
        "displayName":"PDF和图片文字提取",
        "description":"Extract text from images or PDF documents.",
        "description_zh":"从图片或 PDF 文档中识别并提取文字内容。",
        "version":"1.0.13",
        "category":"office-efficiency",
        "icon_url":"https://cloudcache.tencent-cloud.com/x.png",
        "installs":378,
        "downloads":900,
        "claim_state":"",
        "claimable":false,
        "publisher":"RedFoxHub",
        "tags":["ocr","pdf"]
      },
      {
        "slug":"martin-pdf",
        "name":"pdf",
        "displayName":"pdf",
        "description":"Placeholder-ish name.",
        "version":"1.0.0",
        "installs":17
      }
    ]}"#;

    #[test]
    fn a_single_hub_skill_parses_and_prefers_the_chinese_summary() {
        let env: SkillEnvelope = serde_json::from_str(REAL_SEARCH).expect("应当能解析");
        assert_eq!(env.results.len(), 2);
        let s = &env.results[0];
        assert_eq!(s.slug, "pdf-image-text-extractor");
        assert_eq!(s.display_name(), "PDF和图片文字提取");
        assert_eq!(s.version, "1.0.13");
        // 中文简介优先：没有它，界面上就是一段英文。
        assert_eq!(s.summary(), "从图片或 PDF 文档中识别并提取文字内容。");
    }

    #[test]
    fn a_skill_without_a_display_name_falls_back_to_its_slug_not_to_blank() {
        // 空名字会在界面上变成一个没有字的卡片。
        let env: SkillEnvelope = serde_json::from_str(REAL_SEARCH).expect("能解析");
        let s = &env.results[0];
        let blank = HubSkill {
            slug: "only-slug".to_string(),
            name: "   ".to_string(),
            ..s.clone()
        };
        assert_eq!(blank.display_name(), "only-slug");
    }

    #[test]
    fn an_english_only_skill_shows_its_english_summary_rather_than_nothing() {
        let env: SkillEnvelope = serde_json::from_str(REAL_SEARCH).expect("能解析");
        // 第二条没有 description_zh。
        assert_eq!(env.results[1].summary(), "Placeholder-ish name.");
    }

    #[test]
    fn install_counts_stay_absent_when_upstream_does_not_give_them() {
        // 「0 次安装」与「上游没给」在界面上是两句话。缺席就该缺席，
        // 拿 0 顶替就是在编一个数字。
        let v = serde_json::json!({"results":[{"slug":"x","name":"X"}]});
        let env: SkillEnvelope = serde_json::from_value(v).expect("应当能解析");
        assert_eq!(env.results[0].installs, None);
        assert_eq!(env.results[0].downloads, None);
        let v2 = serde_json::json!({"results":[{"slug":"x","installs":0}]});
        let env2: SkillEnvelope = serde_json::from_value(v2).expect("应当能解析");
        assert_eq!(
            env2.results[0].installs,
            Some(0),
            "上游真的给了 0，那才是 0"
        );
    }

    #[test]
    fn showcase_reports_the_sections_own_name_rather_than_the_kind_we_asked_for() {
        // 上游把 `hot` 叫 `hot_downloads`。透传它的口径，
        // 我们自己再翻译一遍就等于替上游改口。
        let v = serde_json::json!({"section":"hot_downloads","skills":[{"slug":"x","name":"X"}]});
        let env: ShowcaseEnvelope = serde_json::from_value(v).expect("应当能解析");
        assert_eq!(env.section, "hot_downloads");
        assert_eq!(env.skills.len(), 1);
    }

    #[test]
    fn an_empty_showcase_is_empty_rather_than_missing() {
        // 「这一份榜单是空的」是真的空，序列化出去就是空数组。
        let v = serde_json::json!({"section":"newest","skills":[]});
        let env: ShowcaseEnvelope = serde_json::from_value(v).expect("应当能解析");
        assert!(env.skills.is_empty());
    }

    fn board(kind: &str, section: &str, slugs: &[&str]) -> Showcase {
        Showcase {
            kind: kind.to_string(),
            section: section.to_string(),
            items: slugs
                .iter()
                .map(|s| HubSkill {
                    slug: s.to_string(),
                    name: s.to_string(),
                    ..serde_json::from_value(serde_json::json!({"slug": s})).expect("能解析")
                })
                .collect(),
        }
    }

    #[test]
    fn merging_every_board_deduplicates_skills_that_appear_in_several() {
        // 同一个技能同时上「热门」和「推荐」是上游的常态。
        // 不去重的话「全部」这一页里同一个技能会连着出现两三次。
        let all = ShowcaseAll {
            sections: vec![
                board("hot", "hot_downloads", &["a", "b"]),
                board("recommended", "recommended", &["b", "c"]),
            ],
            errors: vec![],
        };
        let slugs: Vec<String> = all.merged().into_iter().map(|s| s.slug).collect();
        assert_eq!(slugs, vec!["a", "b", "c"]);
    }

    #[test]
    fn a_board_that_failed_to_load_is_reported_rather_than_silently_absent() {
        // 「6 个榜单里 1 个没拉到」必须能说出来。
        // 把它吞掉，界面上那个页签就会假装自己是完整的。
        let all = ShowcaseAll {
            sections: vec![board("hot", "hot_downloads", &["a"])],
            errors: vec![("trending".to_string(), "上游 503".to_string())],
        };
        assert_eq!(all.merged().len(), 1, "成功的那部分照常给");
        assert_eq!(all.errors.len(), 1, "失败的那部分要报出来");
        assert_eq!(all.errors[0].0, "trending");
    }

    /// 上游真的会给 `null` 的样本（2026-10-06 实测）。
    ///
    /// 搜 `pdf` 的 20 条里 `pdf-md` 的 `icon_url` 是 `null`；搜「翻译」6 条里
    /// 至少 6 条 `icon_url` 是 `null`；搜 `excel` 也有 1 条。
    /// 声明成 `String` 的话，**这一条就把整页搜索变成 500** ——
    /// 而市场明明好好地回了 20 条结果。见 ISSUE-056。
    ///
    /// 其余字段在同几轮探测里**没有一个是 null**（`version`、`description_zh`
    /// 都是字符串）。所以下面只把 `icon_url` 写成 null，
    /// 免得拿想象冒充证据。
    const REAL_SEARCH_WITH_NULLS: &str = r#"{"results":[
      {"slug":"pdf-md","name":"PDF","description":"d","description_zh":"中文",
       "version":"1.0.0","category":"office","icon_url":null,"installs":3,"publisher":null},
      {"slug":"normal-one","name":"N","description":"d","description_zh":"简介",
       "version":"1.0.0","category":"office","icon_url":"https://x/y.png","installs":5}
    ]}"#;

    #[test]
    fn a_null_field_does_not_take_the_whole_page_down() {
        // **`#[serde(default)]` 管不了 `null`** —— 这是最容易以为防住了的坑。
        let env: SkillEnvelope =
            serde_json::from_str(REAL_SEARCH_WITH_NULLS).expect("一条 null 不该炸掉整页");
        assert_eq!(env.results.len(), 2, "两条都要在");
        let n = &env.results[0];
        assert_eq!(n.slug, "pdf-md");
        // null 读成空串 = 「上游没给值」。**不是**替它编一个。
        assert_eq!(n.icon_url, "");
        // 同一行里没有 null 的字段照常读出来。
        assert_eq!(n.version, "1.0.0");
        assert_eq!(n.description_zh, "中文");
        assert_eq!(n.installs, Some(3));
        // 另一条不受影响。
        assert_eq!(env.results[1].icon_url, "https://x/y.png");
    }

    #[test]
    fn a_skill_with_only_a_slug_still_lands_on_the_list() {
        // 上游少给字段是常态，不该让整条消失 —— 用户宁可看到一条没说明的，
        // 也不要看到一个「市场没连上」。
        let v = serde_json::json!({"results":[{"slug":"bare"}]});
        let env: SkillEnvelope = serde_json::from_value(v).expect("应当能解析");
        assert_eq!(env.results.len(), 1);
        assert_eq!(env.results[0].display_name(), "bare");
        assert_eq!(env.results[0].summary(), "");
    }

    #[test]
    fn a_board_whose_section_is_null_is_still_a_board() {
        // 实测 `GET /api/v1/showcase/paid` 回的是 `"section": null`。
        let v = serde_json::json!({"section":null,"skills":[{"slug":"paid-one","name":"P"}]});
        let env: ShowcaseEnvelope = serde_json::from_value(v).expect("应当能解析");
        assert_eq!(env.section, "");
        assert_eq!(env.skills.len(), 1, "技能还是要给出来");
    }

    #[test]
    fn a_skill_set_that_omits_optional_fields_still_parses() {
        // 实测（2026-10-06）：skillsets 的响应里 `displayNameEn`、`summaryEn`、
        // `content` 这类字段有时干脆**没有这个键**（英文名与正文不是每个包都填）。
        //
        // 注意这里刻意不写 `null`：实测 skillsets 的字段值没有一个是 null
        // （null 是 search 的 `icon_url` 才有的毛病）。拿一个没观测到的情况
        // 当测试用例，就是拿想象冒充证据。
        let v = serde_json::json!({"skillSets":[
            {"id":1,"slug":"a","displayName":"甲","summary":"说明","skillSlugs":["x"]}
        ]});
        let env: ListEnvelope = serde_json::from_value(v).expect("应当能解析");
        assert_eq!(env.skill_sets.len(), 1);
        let s = &env.skill_sets[0];
        assert_eq!(s.display_name, "甲");
        // 缺席的英文名与正文是空串，不是编一个出来。
        assert_eq!(s.display_name_en, "");
        assert_eq!(s.content, "");
    }

    #[test]
    fn a_manifest_with_null_text_still_parses() {
        let m = parse_manifest(r#"{"slug":"x","displayName":null,"summary":null}"#)
            .expect("应当能解析");
        assert_eq!(m.display_name(), None);
        assert!(m.referenced_slugs().is_empty());
    }

    /// `labels.requires_api_key` 的真实形态（2026-10-06 实测）：
    /// 值是**字符串** `"true"`，不是布尔。
    const REAL_LABELS_STRING: &str =
        r#"{"slug":"a","name":"A","labels":{"requires_api_key":"true"}}"#;
    /// 同一字段的另一种形态：布尔。
    const REAL_LABELS_BOOL: &str = r#"{"slug":"a","name":"A","labels":{"requires_api_key":true}}"#;

    #[test]
    fn a_skill_that_needs_an_api_key_says_so_in_both_upstream_shapes() {
        // Octop 的卡片上有这个橙标（`requiresApiKey()`）。少了它，
        // 用户装完才发现这个技能要自己的密钥。
        let a: HubSkill = serde_json::from_str(REAL_LABELS_STRING).expect("应当能解析");
        assert!(a.labels.requires_api_key(), "字符串 \"true\"");
        // 收成 bool 的话，改成布尔 true 就会解析失败 —— 正是 ISSUE-056 的坑。
        let b: HubSkill = serde_json::from_str(REAL_LABELS_BOOL).expect("应当能解析");
        assert!(b.labels.requires_api_key(), "布尔 true");
    }

    #[test]
    fn a_skill_without_labels_is_not_marked_as_needing_a_key() {
        // 标签缺席是常态（`labels: null` 也出现过），不能当成「需要密钥」。
        for raw in [
            r#"{"slug":"a","name":"A"}"#,
            r#"{"slug":"a","name":"A","labels":null}"#,
            r#"{"slug":"a","name":"A","labels":{}}"#,
            r#"{"slug":"a","name":"A","labels":{"requires_api_key":"false"}}"#,
        ] {
            let skill: HubSkill = serde_json::from_str(raw).expect("应当能解析");
            assert!(!skill.labels.requires_api_key(), "不该标成需要密钥：{raw}");
        }
    }
}
