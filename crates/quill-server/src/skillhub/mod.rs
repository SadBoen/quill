//! SkillHub 客户端 —— 技能市场的上游。
//!
//! 照抄 Octop 的安全上限（`.octop-ref/octop` 的 `skills/skillhub_common.py`）。
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
//! ## 端点路径是实测来的，不是抄来的
//!
//! 端点常量（`SEARCH_ENDPOINT` / `DOWNLOAD_ENDPOINT` / `RANKING_ENDPOINTS`）在上游
//! **另一份** `infra/skills/skillhub_market.py` 里，而那份**不在我们的 sparse 检出
//! 集合内**（可查的那份只有 `agents/experts/skillhub_market.py`，里面没有这几个常量）。
//! 所以上面那三条端点是 2026-10-06 **真机实测**（对 https://api.skillhub.cn 发请求、
//! 看它真实返回什么）定下来的，不是逐字抄来的 —— 谁去核对都核不出「抄自哪一行」，
//! 因为真的没有那一行。这也是我们不写「照抄 infra/skills/ 那份、experts 那份是旧的」
//! 的原因：那份不在手边，「哪份是旧的」判断不了。
//!
//! 我们**确实**从可核的那份（`agents/experts/skillhub_market.py`）学到的，是它的
//! 安全上限（`skills/skillhub_common.py`）与两处**没有照抄**的缺陷：
//! - 包里多个 `.md` 时它取 `zf.namelist()` 的第一个、不排序（:610-616）；
//! - manifest 的扁平 `skillSlugs` 直接透传、不去重（:652-655）。
//! 我们改成先计划再写入、按名去重并如实报重复，见 [`plan_install`]。
//!
//! ## 三条不能省的限制
//!
//! 1. **超时**：默认 30 秒。没有超时的话上游一挂，界面就一直转圈。
//! 2. **体积上限**：32 MiB，**读的时候就限量**（见 [`http::read_capped`]）。
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
//!
//! ## 子模块布局（Q008 纯机械拆分，行为零变化）
//!
//! - [`http`]：上游 HTTP 客户端、`host()`、超时与体积上限、**边读边限量**。
//! - [`models`]：上游返回形状的数据模型（技能集 / 单技能 / 榜单 / manifest）。
//! - [`endpoints`]：各端点的调用封装与 slug 校验。
//! - [`errors`]：`HubError` 的四种结局与六榜聚合规则。
//! - [`unpack`]：zip 解包与四道安全上限（旧的 `skillhub_unpack` 路径见 `lib.rs`）。

mod endpoints;
mod errors;
mod http;
mod models;
pub mod unpack;

pub use self::endpoints::{
    download_skill, download_skillset, fetch_skillset, list_skillsets, search_skills, showcase_all,
    showcase_skills, validate_slug, SkillSetPage, SHOWCASE_KINDS,
};
pub use self::errors::HubError;
pub use self::http::{host, HTTP_TIMEOUT_SECS, MAX_HTTP_BYTES};
pub use self::models::{
    parse_manifest, HubLabels, HubManifest, HubManifestSkill, HubSkill, HubSkillSet, Showcase,
    ShowcaseAll,
};
pub use self::unpack::{MAX_ZIP_COMPRESSION_RATIO, MAX_ZIP_ENTRIES, MAX_ZIP_UNCOMPRESSED_BYTES};
