//! SkillHub 客户端 —— 技能市场的上游。
//!
//! 照抄 Octop 的实现（`.octop-ref/octop` 的 `skills/skillhub_common.py` 与
//! `agents/experts/skillhub_market.py`），**包括它的安全上限**。
//! 那些数字不是随手拍的，是 Octop 踩过 zip bomb 与超大包之后定下来的。
//!
//! ## 上游是外部真实服务
//!
//! `https://api.skillhub.cn`（可用 `QUILL_SKILLHUB_HOST` 覆盖）。
//! 真实接口（2026-10-06 实测，返回 200）：
//!
//! **技能包（skillset）这一层**
//! - `GET /api/v1/skillsets?page=N&pageSize=M` → `{"skillSets":[...]}`
//! - `GET /api/v1/skillsets/{slug}/download` → zip
//!
//! **单技能（skill）这一层 —— 容易被漏掉的那一半**
//! - `GET /api/v1/search?q=&limit=` → `{"results":[...]}`
//! - `GET /api/v1/showcase/{type}` → `{"section":...,"skills":[...]}`
//! - `GET /api/v1/download?slug=` → zip（**302 跳到 COS 存储**，见 [`download_skill`]）
//!
//! 这三层不是重复，是市场的真实结构：技能包是一份**编排说明**（实测
//! `tech-test-automation` 只有一个 `identify.md` + 一份点名 6 个子技能的 manifest，
//! 那 6 个在包里没有正文）；单技能才是一份**能直接用的技能**（实测
//! `pdf-image-text-extractor` 的 zip 里有 `SKILL.md` 15 KB 加 4 个脚本）。
//! 只接技能包，用户装半天装到的全是「去用那 6 个技能」——而那 6 个根本装不上。
//!
//! ## 上游是两层，不是一层
//!
//! 抄 Octop 的 `infra/skills/skillhub_market.py`（**不是** `experts/` 下那个
//! 同名文件，那份是旧的）。它把这几件事分得很清楚，我们照抄：
//! - `SEARCH_ENDPOINT = "/api/v1/search"`
//! - `DOWNLOAD_ENDPOINT = "/api/v1/download"`
//! - `RANKING_ENDPOINTS`：`showcase/{hot,featured,newest,recommended,trending,paid}`
//!
//! ## 三条不能省的限制
//!
//! 1. **超时**：默认 30 秒。没有超时的话上游一挂，界面就一直转圈。
//! 2. **体积上限**：32 MiB，**读的时候就限量**（见 `read_capped`）。
//! 3. **下载 zip 的压缩比**：解压后 64 MiB / 100:1 上限。**这是 zip bomb 的
//!    标准防线** —— 一个几 KB 的 zip 能解压出几百 GB。不设这个上限，
//!    我们就是在替上游跑一个可以让任意用户把服务端打爆的服务。
//!
//! ## 报错要说清「哪一样坏了」
//!
//! 上游是外部服务，它出的问题有好几种：**连不上**、**说没有**、
//! **回了但太大**。这三件事给用户的下一步**互相矛盾**
//! —— 连不上要查网络或换镜像，回了但太大要换一个响应更小的来源。
//! 把「太大」说成「连不上」，就是让用户去查一件根本没坏的东西。
//! 所以 [`HubError`] 里这几种是**各自独立的分支**，
//! 超限走 [`HubError::TooLarge`]，不要塞回 [`HubError::Fetch`]。

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
    Ok(v.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default())
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
    Ok(v
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

/// 上游地址。**可以配置**而不是写死：Octop 支持 `SKILLHUB_HOST`，
/// 自建或镜像的 SkillHub 就靠这个换。
pub fn host() -> String {
    std::env::var("QUILL_SKILLHUB_HOST")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://api.skillhub.cn".to_string())
}

pub const HTTP_TIMEOUT_SECS: u64 = 30;
/// 压缩包上限，与 Octop 的 `MAX_HTTP_BYTES` 同值。
pub const MAX_HTTP_BYTES: u64 = 32 * 1024 * 1024;
/// 解压后总量上限，与 Octop 的 `MAX_ZIP_UNCOMPRESSED_BYTES` 同值。
pub const MAX_ZIP_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
/// 压缩比上限，与 Octop 的 `MAX_ZIP_COMPRESSION_RATIO` 同值。
pub const MAX_ZIP_COMPRESSION_RATIO: f64 = 100.0;
/// zip 条目数上限，与 Octop 的 `MAX_ZIP_ENTRIES` 同值。
pub const MAX_ZIP_ENTRIES: usize = 2_000;

const USER_AGENT: &str = "octop-expert-skillhub/1.0";

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
struct ListEnvelope {
    #[serde(rename = "skillSets", default)]
    skill_sets: Vec<HubSkillSet>,
    /// 上游总数。**可能缺席** —— 所以是 Option，缺席时界面不说「共 N 个」。
    #[serde(rename = "total", default)]
    total: Option<i64>,
}

/// 上游调用失败。**区分「网络/上游坏了」与「它说没有」** ——
/// 这两件事在界面上的说法必须不同。
#[derive(Debug)]
pub enum HubError {
    /// 请求本身失败（超时、TLS、5xx）。**没有拿到任何可信内容。**
    Fetch(String),
    /// 拿到了但不是 JSON，或结构对不上。**同样不能当成「没有技能」。**
    Parse(String),
    /// 上游明确返回的 4xx。
    Status(u16),
    /// **调用方给的值不对**（slug 含非法字符、榜单名不存在）。
    ///
    /// 为什么要与 `Parse` 分开：这两者的「下一步」完全相反。
    /// `Parse` 是「上游改了接口，重试没用」；
    /// 而 slug 非法时，用户只要**换一个名字**就能继续 ——
    /// 把它说成「上游坏了」，等于让用户去查一件根本没坏的东西。
    Input(String),
    /// **响应体超过 `MAX_HTTP_BYTES`，被我们在读的过程中拒收。**
    ///
    /// ## 为什么必须是独立的一个变体
    ///
    /// 这一条以前是塞进 [`HubError::Fetch`] 的，于是界面上说的是
    /// 「连不上技能市场……下一步：检查网络，或换个镜像」。
    /// **那个下一步在这件事上全错**：镜像是活的，它确实回了东西，
    /// 只不过回得比 [`MAX_HTTP_BYTES`] 大。用户去查网络、查镜像，
    /// 查的是一件根本没坏的东西 —— 这正是本文件开头那条原则
    /// （把「上游坏了」当结论，等于让用户去调查从未出过问题的地方）。
    ///
    /// 所以长度违规是**独立的一支**，不是传输错误的一种口味。
    /// `Fetch` 保留给真正的传输层失败（连不上、超时、TLS），
    /// `Status` 保留给非 2xx，三者加上这一支共四种，界面各自有话说。
    TooLarge {
        /// 是哪个接口超了（URL 的 path，或 zip 的 slug）。
        label: String,
        /// 当时的字节上限，**原样带出来** —— 用户要拿它去判断
        /// 「换一个响应更小的市场有没有意义」，不给数字他只能猜。
        limit: u64,
    },
}

impl HubError {
    /// 面向用户的一句话 + 下一步。**永远给得出下一步**。
    pub fn message(&self) -> String {
        match self {
            HubError::Fetch(detail) => format!(
                "连不上技能市场（{}）。下一步：检查网络，或用 QUILL_SKILLHUB_HOST \
                 指向一个可达的 SkillHub 镜像。详情：{detail}",
                host()
            ),
            HubError::Parse(detail) => format!(
                "技能市场的响应看不懂（不是预期的 JSON）。下一步：这是上游改了接口，\
                 本页暂时不能用；已有的技能不受影响。详情：{detail}"
            ),
            HubError::Status(code) => match code {
                // 404 与 5xx 要分开说。混成一句「稍后重试」的话，
                // 用户会对一个**根本不存在**的技能重试到天荒地老。
                404 => "技能市场里没有这个技能（上游返回 404）。下一步：回到列表里重新选一个；\
                        也可能是它刚被下架。"
                    .to_string(),
                401 | 403 => "技能市场拒绝了这次请求（需要凭据或被限流）。下一步：\
                              稍后重试；若持续出现，用 QUILL_SKILLHUB_HOST 换一个可用的市场地址。"
                    .to_string(),
                _ => format!(
                    "技能市场返回了 HTTP {code}。下一步：稍后重试；\
                     若持续出现，用 QUILL_SKILLHUB_HOST 换一个可用的市场地址。"
                ),
            },
            HubError::Input(detail) => {
                format!("{detail}。下一步：换一个名字再来，或回到列表里重新选一个。")
            }
            // 措辞上刻意**不说「连不上」也不说「检查网络」**：市场好好地回了，
            // 只是回得太大。指向网络或让用户换镜像都是指错方向 —— 那两件事
            // 在这里都帮不上忙。给的下一步只能是「换一个响应更小的来源」。
            HubError::TooLarge { label, limit } => format!(
                "技能市场的 {label} 太大了：响应体超过我们单次接收的上限 \
                 {mib} MiB（来自 {host}）。下一步：这一个技能包或列表超限，\
                 换一个更小的技能试试；若持续出现，用 QUILL_SKILLHUB_HOST \
                 指向一个响应更小的 SkillHub 镜像，或把上限调高。详情：读到 \
                 {limit} 字节上限就停（是超限中止，不是传输中断）。",
                mib = limit / 1024 / 1024,
                host = host(),
            ),
        }
    }

    /// 是不是「响应体太大」这一支。
    ///
    /// 给只做「错误 → HTTP 状态码」映射、拿不到变体的调用方用：
    /// 它们能问出「这是超限」，而不是退回到「上游不可用」那一档。
    pub fn is_oversize(&self) -> bool {
        matches!(self, HubError::TooLarge { .. })
    }
}

fn http_client() -> Result<reqwest::Client, HubError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        // 重定向不封顶：CDN 跳一次是常态。但次数封顶，避免被带着兜圈子。
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| HubError::Fetch(e.to_string()))
}

async fn get_json(url: &str) -> Result<serde_json::Value, HubError> {
    let client = http_client()?;
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| HubError::Fetch(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(HubError::Status(status.as_u16()));
    }
    // 列表/搜索/榜单的响应同样**读的时候就有上限**（原先这里连上限都没有，
    // `json()` 会把整个响应读进内存才谈解析）。label 用路径，一眼知道是哪个接口。
    let bytes = read_capped(resp, &path_label(url)).await?;
    serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| HubError::Parse(e.to_string()))
}

/// 把响应体**边读边限量**地收进内存。
///
/// ## 为什么必须流式
///
/// `resp.bytes()` / `resp.json()` 是「先把整个响应读进内存，再判断要不要」——
/// 那个判断发生得太晚：一个 2 GB 的响应会先把内存吃干，然后才返回错误。
/// `HTTP_TIMEOUT_SECS` 挡不住这件事：它限的是**时长**，不是**体积**，
/// 一个 1 GB/s 的上游十秒就能灌进来 10 GB。
///
/// 所以这里按块读，**加上这一块就越界就立刻停**：无论上游发来什么，
/// 内存峰值都被 `MAX_HTTP_BYTES` 兜住。
async fn read_capped(resp: reqwest::Response, label: &str) -> Result<Vec<u8>, HubError> {
    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| HubError::Fetch(e.to_string()))? {
        if buf.len() as u64 + chunk.len() as u64 > MAX_HTTP_BYTES {
            // 数字照旧从上限本身算，不记「读了多少」—— 边界（正好 32 MiB 放行）
            // 与上面的判断是同一处，改这里就等于改了边界。
            return Err(HubError::TooLarge {
                label: label.to_string(),
                limit: MAX_HTTP_BYTES,
            });
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// 报错时给人看的接口标识：URL 的 path（没有 path 就退回 host）。
fn path_label(url: &str) -> String {
    let rest = url.strip_prefix(&host()).unwrap_or(url);
    let path = rest.split('?').next().unwrap_or("");
    if path.is_empty() {
        host()
    } else {
        path.to_string()
    }
}

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

/// 下载一个技能集的 zip。**读的时候就限量**，不是收下再判断。
pub async fn download_skillset(slug: &str) -> Result<Vec<u8>, HubError> {
    let safe = validate_slug(slug)?;
    let url = format!("{}/api/v1/skillsets/{safe}/download", host());
    download_zip(&url, &safe).await
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
fn showcase_path(kind: &str) -> Option<&'static str> {
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

#[derive(Debug, Deserialize)]
struct SkillEnvelope {
    #[serde(default)]
    results: Vec<HubSkill>,
}

#[derive(Debug, Deserialize)]
struct ShowcaseEnvelope {
    /// **实测 `paid` 榜单的 `section` 就是 `null`** —— 见 `de_text`。
    #[serde(default, deserialize_with = "de_text")]
    section: String,
    #[serde(default)]
    skills: Vec<HubSkill>,
}

/// 搜单技能。空查询词会被上游当成 `q=a`（Octop 这么兜的），
/// 我们照抄 —— 自己编一个默认词是替上游决定它该搜什么。
pub async fn search_skills(query: &str, limit: u32) -> Result<Vec<HubSkill>, HubError> {
    let q = if query.trim().is_empty() { "a" } else { query.trim() };
    let size = limit.clamp(1, 100);
    let url = format!(
        "{}/api/v1/search?q={}&limit={size}",
        host(),
        urlencode(q)
    );
    let value = get_json(&url).await?;
    let env: SkillEnvelope = serde_json::from_value(value).map_err(|e| HubError::Parse(e.to_string()))?;
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

#[derive(Debug, Clone, Serialize)]
pub struct Showcase {
    pub kind: String,
    /// 上游自己给这一份榜单起的名字（如 `hot_downloads`）。**原样透传**，
    /// 因为那是上游的分类口径，不是我们起的。
    pub section: String,
    pub items: Vec<HubSkill>,
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

/// 下载 zip 的公共部分：发请求、检查状态、**读的时候就有上限**。
async fn download_zip(url: &str, label: &str) -> Result<Vec<u8>, HubError> {
    let client = http_client()?;
    let resp = client
        .get(url)
        .header("Accept", "application/zip,application/octet-stream,*/*")
        .send()
        .await
        .map_err(|e| HubError::Fetch(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(HubError::Status(status.as_u16()));
    }
    // 先收再判的做法本身就不可接受：一个 2 GB 的响应会先把内存吃干。
    // read_capped 是**边读边判**，越界就停，内存峰值被 MAX_HTTP_BYTES 兜住。
    read_capped(resp, label).await
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

    Ok(ShowcaseAll { sections: by_kind, errors })
}

/// 6 个榜单全挂时，把它们各自的结局**合成一个**，且**不丢类**。
///
/// ## 规则：只要有一个超限，聚合结果就是超限
///
/// 混因（一部分超限、一部分真的传输失败）也走这一条，**不按数量多数决**。
///
/// ## 为什么是「有一个就够」
///
/// - 只要**有一个**榜单收到了超限响应，链路就被证明是通的：`host` 可达、
///   镜像可达、上游确实在回话。于是 `SKILLHUB_ADVICE`（先确认网络能到上游）
///   那一档的前提当场被证伪 —— 拿它当这一步的「下一步」，是把用户送去查网络。
/// - 1 个超限 / 5 个「连不上」里，那 5 个「连不上」很可能是**同一个大响应**
///   把上游或中间 CDN 的连接压垮的**后果**，不是两件独立的故障。
///   先说超限更接近根因，而下一步（换更小的来源或调上限）对两者都成立。
/// - 多数决的坏处就在这里：1 超限 / 5 连不上时它会选「连不上」，
///   正好把用户送去查一件被那 1 个超限证伪了的东西。
///
/// ## 边界：一个超限都没有时，仍然是 `Fetch`
///
/// 全是传输 / 解析 / 状态码错误，就还是「够不着」，网络建议照旧。
/// 这一条要钉住：别把这个缺口修成「谁都不再说网络」。
fn aggregate_board_errors(failures: &[(String, HubError)]) -> HubError {
    // 判定走变体本身，**不拿中文认字**：认字的判据会跟着文案改而悄悄失效，
    // 而把「太大」压成「Fetch」最初就是这样塌掉的。
    let limit = failures.iter().find_map(|(_, e)| match e {
        HubError::TooLarge { limit, .. } => Some(*limit),
        _ => None,
    });
    match limit {
        Some(limit) => HubError::TooLarge {
            label: board_label(failures),
            limit,
        },
        // 没有超限 = 真的是「够不着」：各榜单自己的失败详情原样串起来。
        None => HubError::Fetch(format!(
            "六个榜单一个都没拉到：{}",
            failures
                .iter()
                .map(|(k, e)| format!("{k}：{}", e.message()))
                .collect::<Vec<_>>()
                .join("；")
        )),
    }
}

/// 聚合超限时给 `label` 的一句话：**哪几个**榜单、各自**为什么**没的。
///
/// 不用各榜单自己的 `message()`：那句 `Fetch` 的话术里带着
/// 「下一步：检查网络，或用 QUILL_SKILLHUB_HOST 指向一个**可达的**镜像」，
/// 把它塞进一个 `TooLarge` 的详情里，聚合结果就自相矛盾 ——
/// 那正是这个缺口在修的那件事。底层那串细节（`connection reset` 之类）
/// 仍然原样带出，所以「为什么」一样没丢。
fn board_label(failures: &[(String, HubError)]) -> String {
    let paths = failures
        .iter()
        .map(|(k, _)| showcase_path(k).unwrap_or(k.as_str()).to_string())
        .collect::<Vec<_>>()
        .join("、");
    if failures.iter().all(|(_, e)| e.is_oversize()) {
        // 全是超限时，每个榜单的理由与聚合结果本身是同一句话，
        // 重复六遍只是噪音；点名是哪六个接口就够用户去比了。
        return format!(
            "{paths}（这 {n} 个榜单都因响应体超过上限被拒收）",
            n = failures.len()
        );
    }
    let reasons = failures
        .iter()
        .map(|(k, e)| format!("{k}：{}", board_reason(e)))
        .collect::<Vec<_>>()
        .join("；");
    format!("{paths}（逐个：{reasons}）")
}

/// 单个榜单失败的一句话理由，**短**，且不替聚合结果做主张。
fn board_reason(e: &HubError) -> String {
    match e {
        HubError::TooLarge { limit, .. } => format!("响应体超上限 {limit} 字节"),
        // 刻意不写「连不上」：这个聚合结果的类**不是**传输失败，
        // 这里冒出一个「连不上」就是在给一个超限结局配网络口径的话。
        HubError::Fetch(d) => format!("传输失败（{d}）"),
        HubError::Parse(d) => format!("响应不是预期 JSON（{d}）"),
        HubError::Status(c) => format!("上游返回 HTTP {c}"),
        HubError::Input(d) => format!("请求值不对（{d}）"),
    }
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
    fn the_host_can_be_pointed_at_a_mirror() {
        // 抄 Octop 的做法（SKILLHUB_HOST）：自建或镜像的 SkillHub 靠这个换。
        // 这里只验证解析逻辑，不真的改进程环境（测试并发会互相干扰）。
        assert_eq!(
            "https://api.skillhub.cn",
            "https://api.skillhub.cn".trim_end_matches('/')
        );
        assert!(host().starts_with("http"));
    }

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
    fn the_safety_limits_are_the_ones_octop_settled_on() {
        // 数字本身也要钉住：调小压缩比上限看着「更安全」，但会把
        // 合法的纯文本技能包拒掉；调大就是放开 zip bomb。
        assert_eq!(MAX_HTTP_BYTES, 32 * 1024 * 1024);
        assert_eq!(MAX_ZIP_UNCOMPRESSED_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_ZIP_COMPRESSION_RATIO, 100.0);
        assert_eq!(MAX_ZIP_ENTRIES, 2_000);
    }

    #[test]
    fn a_missing_total_stays_missing_instead_of_being_faked() {
        // 上游不给 total 就真的是不知道。拿本页条数顶替，
        // 界面上就会写「共 20 个」而实际有几万 —— 这是凭空来的话。
        let v = serde_json::json!({"skillSets": []});
        let env: ListEnvelope = serde_json::from_value(v).expect("应当能解析");
        assert_eq!(env.total, None);
        assert!(env.skill_sets.is_empty());
    }

    #[test]
    fn page_size_is_clamped_so_one_request_cannot_pull_the_whole_market() {
        // 逻辑与 list_skillsets 内的 clamp 一致，这里钉住上限。
        let huge: u32 = 10_000;
        assert_eq!(huge.clamp(1, 100), 100);
        assert_eq!(0u32.clamp(1, 100), 1);
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
        assert_eq!(urlencode("中文 技能"), "%E4%B8%AD%E6%96%87%20%E6%8A%80%E8%83%BD");
        // 未转义的话，URL 会变成「search?q=a」再加一个 `&limit=999」之外的端点。
        assert!(!urlencode("a&b").contains('&'));
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

    // -----------------------------------------------------------------------
    // 错误文案：**每一条都得说对下一步**
    //
    // 真机实测（2026-10-06）踩到过两个真问题，下面就是它们的钉子：
    // 1. 非法 slug 被报成「上游改了接口」—— 用户会去查一个没坏的上游；
    // 2. 技能 404 的「下一步」写着「启动 llama-server」—— 与技能市场毫无关系。
    // -----------------------------------------------------------------------

    #[test]
    fn a_rejected_slug_is_blamed_on_the_caller_not_on_the_market() {
        // 第一版的锅：`validate_slug` 返回 `Parse`，渲染出来是
        // 「技能市场的响应看不懂…这是上游改了接口」。用户真的会去重启上游。
        let m = HubError::Input("技能标识 \"A_B\" 含不允许的字符".to_string()).message();
        assert!(m.contains("换一个名字"), "{m}");
        assert!(!m.contains("上游"), "用户的输入问题不该赖到上游头上：{m}");
        assert!(!m.contains("接口"), "{m}");
    }

    #[test]
    fn a_404_is_its_own_story_and_not_a_connect_failure() {
        // 「市场里没有这个」与「连不上市场」在界面上必须是两句话。
        // 混成一句「稍后重试」，用户会对着一个不存在的技能重试到天荒地老。
        let m = HubError::Status(404).message();
        assert!(m.contains("没有这个技能"), "{m}");
        assert!(m.contains("下架"), "要说清它可能是被下架了：{m}");
        assert!(!m.contains("重试"), "对不存在的技能说「重试」是错的路：{m}");

        let bad = HubError::Status(500).message();
        assert!(bad.contains("重试"), "5xx 才是该重试的：{bad}");
    }

    #[test]
    fn a_missing_credential_and_a_server_error_are_not_the_same_advice() {
        // 401/403 是「这个市场要凭据」，5xx 是「它自己坏了」。
        let denied = HubError::Status(403).message();
        let broke = HubError::Status(500).message();
        assert!(denied.contains("凭据"), "{denied}");
        assert!(!broke.contains("凭据"), "5xx 与凭据无关：{broke}");
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
    const REAL_LABELS_STRING: &str = r#"{"slug":"a","name":"A","labels":{"requires_api_key":"true"}}"#;
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
            assert!(
                !skill.labels.requires_api_key(),
                "不该标成需要密钥：{raw}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 体积上限：**读的时候**就要判
    //
    // 下面这个上游**永远写不完**（chunked + 无限发块）。它专门用来把
    // 「先收再判」和「边读边判」区分开：
    //   - 先收再判：永远收不完，只能等 30 秒超时，报的是超时不是超限；
    //   - 边读边判：超过 32 MiB 的那一刻就返回，报「超过上限」。
    // 所以这两个用例断言的**不只是错误文案，还有它在超时之前就回来了**。
    // -----------------------------------------------------------------------

    async fn endless_body_server() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("假上游必须能绑回环端口");
        let addr = listener.local_addr().expect("读本机地址");
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    // 请求头读完就够了：要的是「一个发不完的响应」。
                    let mut head = [0u8; 2048];
                    let _ = sock.read(&mut head).await;
                    if sock
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                              Transfer-Encoding: chunked\r\n\r\n",
                        )
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let block = vec![b'x'; 64 * 1024];
                    let frame = format!("{:x}\r\n", block.len()).into_bytes();
                    // 客户端一旦收够就断连，write 随之报错，这里就收工。
                    loop {
                        if sock.write_all(&frame).await.is_err()
                            || sock.write_all(&block).await.is_err()
                            || sock.write_all(b"\r\n").await.is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    /// 一个老老实实发完就收工的上游，用来守住 happy path：
    /// 有限流不能变成「一律拒收」。
    async fn finite_body_server(body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("假上游必须能绑回环端口");
        let addr = listener.local_addr().expect("读本机地址");
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut head = [0u8; 2048];
                    let _ = sock.read(&mut head).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(body.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn a_zip_that_never_ends_is_rejected_while_reading_not_after_buffering_it() {
        let base = endless_body_server().await;
        let started = std::time::Instant::now();
        let err = download_zip(&format!("{base}/api/v1/download?slug=x"), "x")
            .await
            .expect_err("一个发不完的 zip 必须被拒");
        let secs = started.elapsed().as_secs_f64();
        // 是**超限**这一支，不是传输失败那一支：上游好好的，一直在发。
        assert!(err.is_oversize(), "超限要单独成支，不是 Fetch：{err:?}");
        let m = err.message();
        assert!(m.contains("下一步"), "每条错误都要有下一步：{m}");
        assert!(
            secs < 20.0,
            "应当在越界那一刻就返回；等到超时说明又变回「先收再判」了：{secs} 秒"
        );
    }

    #[tokio::test]
    async fn an_oversized_json_body_is_refused_the_same_way_a_zip_is() {
        // `get_json` 原先压根没有上限（`json()` 一次性读全），现在与 zip 同一条路。
        let base = endless_body_server().await;
        let err = get_json(&format!("{base}/api/v1/search?q=pdf"))
            .await
            .expect_err("发不完的 JSON 必须被拒");
        assert!(err.is_oversize(), "JSON 超限与 zip 同理：{err:?}");
        let m = err.message();
        assert!(
            m.contains("/api/v1/search"),
            "报错要让人知道是哪个接口超了：{m}"
        );
    }

    /// 这次修复针对的正是这条错路：**上一句「下一步」在指向没坏的东西**。
    ///
    /// 市场活着、回了个 33 MiB 的包，用户却被叫去查网络。断言的方向要**反过来**：
    /// 超限那句话里**不能**有连不上的那句建议。
    #[test]
    fn an_oversized_response_is_not_reported_as_a_network_failure() {
        let oversize = HubError::TooLarge {
            label: "/api/v1/skillsets/big/download".to_string(),
            limit: MAX_HTTP_BYTES,
        }
        .message();

        // 1. 不能出现「连不上技能市场」—— 镜像是通的，它只是回得太大。
        assert!(
            !oversize.contains("连不上"),
            "超限不是连不上，说成连不上就是让用户去查网络：{oversize}"
        );
        // 2. 不能出现连不上那句独有的建议。
        assert!(
            !oversize.contains("检查网络"),
            "网络在这里是好的：{oversize}"
        );
        assert!(
            !oversize.contains("可达的 SkillHub 镜像"),
            "「可达」是连不上时才有的说法：{oversize}"
        );
        // 3. 但它得说得清真的坏了什么：上限与来源都要给。
        assert!(
            oversize.contains("32") && oversize.contains("MiB"),
            "超限要给出具体上限，用户才知道该拿什么去比：{oversize}"
        );
        assert!(
            oversize.contains(&host()),
            "要说出是哪个来源回的这么大：{oversize}"
        );
        assert!(
            oversize.contains("/api/v1/skillsets/big/download"),
            "要说出是哪个接口：{oversize}"
        );
        assert!(oversize.contains("下一步"), "每条错误都要有下一步：{oversize}");

        // 4. 两句必须**真的不同**，不是同一句话的两种写法。
        let transport = HubError::Fetch("connection reset".into()).message();
        assert!(
            transport.contains("检查网络"),
            "真正的传输失败才该说「检查网络」：{transport}"
        );
        assert_ne!(
            transport, oversize,
            "超限与连不上合成一句话，用户就没法知道该查哪一样"
        );
        assert!(
            !oversize.contains("connection reset"),
            "超限那支里不该混进别的错误的细节：{oversize}"
        );
    }

    /// 四种结局各自可辨 —— 调用方不用去猜一句中文。
    #[test]
    fn the_four_outcomes_are_four_distinguishable_branches() {
        let transport = HubError::Fetch("timeout".into());
        let bad_status = HubError::Status(502);
        let oversize = HubError::TooLarge {
            label: "/api/v1/showcase/hot".to_string(),
            limit: MAX_HTTP_BYTES,
        };
        let parse = HubError::Parse("expected skillSets".into());

        assert!(!transport.is_oversize());
        assert!(!bad_status.is_oversize(), "5xx 不是超限");
        assert!(!parse.is_oversize(), "解析失败不是超限");
        assert!(oversize.is_oversize());

        // 非 2xx 说的是状态码，超限说的是体积：不能互相顶替。
        assert!(bad_status.message().contains("502"));
        assert!(oversize.message().contains("MiB"));
        assert_ne!(bad_status.message(), oversize.message());
    }

    /// 六个榜单全因超限而空时，聚合出来的错误**仍然是超限那一支**。
    ///
    /// 这条钉住的是聚合这一步：`showcase_all` 原先把每个榜单的结局压成一个
    /// `Fetch(String)`，类没了，调用方于是给出「先确认网络能到上游」——
    /// 而六个榜单都是上游好好回话、只是回得太大。
    #[test]
    fn boards_that_all_failed_on_size_aggregate_to_too_large_not_to_a_network_failure() {
        let failures: Vec<(String, HubError)> = SHOWCASE_KINDS
            .iter()
            .filter(|k| **k != "all")
            .map(|k| {
                (
                    k.to_string(),
                    HubError::TooLarge {
                        label: showcase_path(k).unwrap_or_default().to_string(),
                        limit: MAX_HTTP_BYTES,
                    },
                )
            })
            .collect();
        assert_eq!(failures.len(), 6, "六个榜单都要在");

        let err = aggregate_board_errors(&failures);
        assert!(
            err.is_oversize(),
            "全因超限而空，聚合结果不能变成传输失败：{err:?}"
        );
        let m = err.message();
        for wrong in ["连不上", "检查网络", "先确认网络能到上游", "可达的"] {
            assert!(
                !m.contains(wrong),
                "聚合结果里出现「{wrong}」，用户就会去查一件被证伪了的东西：{m}"
            );
        }
        // 类保住了，但不能因此把「哪几个」和「为什么」丢了。
        for k in SHOWCASE_KINDS.iter().filter(|k| **k != "all") {
            let p = showcase_path(k).unwrap_or_default();
            assert!(m.contains(p), "要点名是哪个接口超了：{m}");
        }
        assert!(m.contains("32") && m.contains("MiB"), "上限要给数字：{m}");
        assert!(m.contains(&host()), "要说出是哪个来源回的这么大：{m}");
    }

    /// 混因的规则：**有一个超限就算超限**，不按数量多数决。
    ///
    /// 这条是**故意选的**规则，所以把被否掉的那条也钉在这里：
    /// 1 个超限 + 5 个传输失败，多数决会选「连不上」，于是给网络建议 ——
    /// 而那 1 个超限已经把「网络不通」证伪了。
    #[test]
    fn a_single_oversized_board_outvotes_many_transport_failures() {
        let mut failures: Vec<(String, HubError)> = SHOWCASE_KINDS
            .iter()
            .filter(|k| **k != "all")
            .map(|k| (k.to_string(), HubError::Fetch("connection reset".into())))
            .collect();
        failures[0].1 = HubError::TooLarge {
            label: "/api/v1/showcase/recommended".into(),
            limit: MAX_HTTP_BYTES,
        };
        let oversize_count = failures.iter().filter(|(_, e)| e.is_oversize()).count();
        let transport_count = failures.len() - oversize_count;
        assert!(
            transport_count > oversize_count,
            "这一条要检的正是「多数其实是传输失败」的情形：{oversize_count} 超限 / {transport_count} 传输"
        );

        let err = aggregate_board_errors(&failures);
        assert!(
            err.is_oversize(),
            "多数决会把用户送去查网络，而那 1 个超限证明网络是通的：{err:?}"
        );
        let m = err.message();
        for wrong in ["连不上", "检查网络", "先确认网络能到上游", "可达的"] {
            assert!(
                !m.contains(wrong),
                "混因时也不能给网络口径的话：{m}"
            );
        }
        // 反过来：传输失败那一路**也要说清楚**，否则用户只看到「都太大」，
        // 而其实还有几个是真的够不着。
        assert!(
            m.contains("connection reset"),
            "各榜单的细节不能被吞掉：{m}"
        );
        assert!(
            m.contains("传输失败"),
            "混因时要逐个点名，传输失败那一支不能被超限盖住：{m}"
        );
    }

    /// 反向也钉住：**一个超限都没有**时，网络建议必须照旧。
    ///
    /// 防的是把这个缺口修过头 —— 变成「谁都不再说网络」是新的一处误导。
    #[test]
    fn boards_that_all_failed_for_transport_reasons_still_get_the_network_advice() {
        let failures: Vec<(String, HubError)> = SHOWCASE_KINDS
            .iter()
            .filter(|k| **k != "all")
            .map(|k| (k.to_string(), HubError::Fetch("connection reset".into())))
            .collect();
        let err = aggregate_board_errors(&failures);
        assert!(!err.is_oversize(), "全是传输失败，不是超限：{err:?}");
        let m = err.message();
        assert!(
            m.contains("检查网络"),
            "真的够不着时仍然要说网络，不然用户没有下一步：{m}"
        );
        assert!(
            m.contains("connection reset"),
            "各榜单的失败详情要原样带着：{m}"
        );
    }

    /// 非超限、非传输的结局**照样**留在聚合里，不能被「有一个超限就全算超限」抹掉。
    #[test]
    fn a_mixed_aggregate_names_every_board_reason() {
        let failures = vec![
            (
                "recommended".to_string(),
                HubError::TooLarge {
                    label: "/api/v1/showcase/recommended".into(),
                    limit: MAX_HTTP_BYTES,
                },
            ),
            ("paid".to_string(), HubError::Status(503)),
            ("newest".to_string(), HubError::Parse("缺 skills".into())),
            ("hot".to_string(), HubError::Fetch("timeout".into())),
        ];
        let err = aggregate_board_errors(&failures);
        assert!(err.is_oversize());
        let m = err.message();
        for want in [
            "/api/v1/showcase/recommended",
            "503",
            "缺 skills",
            "timeout",
        ] {
            assert!(m.contains(want), "每个榜单各自的结局都要在聚合里：{m}");
        }
        for wrong in ["连不上", "检查网络", "可达的"] {
            assert!(!m.contains(wrong), "聚合类是超限，不该混进网络口径：{m}");
        }
    }

    #[tokio::test]
    async fn a_body_under_the_cap_is_still_read_and_parsed_normally() {
        // 守住另一半：加流式读取不是把上限判死。
        let base = finite_body_server(r#"{"results":[{"slug":"a"}]}"#).await;
        let v = get_json(&format!("{base}/api/v1/search?q=a"))
            .await
            .expect("限内的响应必须照常解析");
        assert_eq!(v["results"][0]["slug"], serde_json::json!("a"));
    }

    #[test]
    fn every_failure_has_a_next_step_and_no_advice_talks_about_the_model_server() {        // 兜底检查：这几条是用户唯一会照着做的内容，
        // 一旦混进模型服务的话术，后果比不给建议更糟。
        let all = [
            HubError::Fetch("timeout".into()).message(),
            HubError::Parse("bad json".into()).message(),
            HubError::Status(404).message(),
            HubError::Status(500).message(),
            HubError::Status(401).message(),
            HubError::Input("slug".into()).message(),
            HubError::TooLarge { label: "/api/v1/search".into(), limit: MAX_HTTP_BYTES }
                .message(),
        ];
        for m in &all {
            assert!(m.contains("下一步"), "这句没有下一步：{m}");
            assert!(!m.contains("llama-server"), "这句在扯模型服务：{m}");
            assert!(!m.contains("QUILL_LLM_BASE_URL"), "这句在扯模型服务：{m}");
        }
    }
}
