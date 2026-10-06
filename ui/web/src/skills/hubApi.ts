import { apiJson } from '../api/client'

/**
 * 技能市场的接口层。
 *
 * **上游是一个外部服务**（默认 SkillHub，`api.skillhub.cn`），
 * 经 quill 服务端代理。所以这一层拿到的是「**我们真的取到了什么**」，
 * 不是「市场里应该有什么」—— 上游挂了就该显示挂，不能显示空列表。
 */

/** 市场里的一个技能集（一个包，里面含一篇编排说明与若干下游技能）。 */
export interface HubSkillSet {
  id: number
  slug: string
  display_name: string
  display_name_en: string
  summary: string
  summary_en: string
  scene: string
  sub_scene: string
  /** 上游给的正文预览。没有就是没有，界面不编。 */
  content: string
  skill_slugs: string[]
  skill_count: number
  icon_url: string
}

export interface HubList {
  /** 实际连的是哪个上游，界面上要显示 —— 用户得知道这不是「内置列表」。 */
  host: string
  items: HubSkillSet[]
  /**
   * 上游给的总数。**`null` 就是上游没给** ——
   * 界面上这时只能说「本页 20 个」，不能说「共 20 个」。
   */
  total: number | null
  page: number
  page_size: number
}

export interface HubInstallResult {
  installed: { slug: string; description: string }[]
  source_slug: string
  display_name: string | null
  /** **真的装上了几个。** 不是一个技能集 = 几个技能。 */
  installed_count: number
  /**
   * 包里点名引用、但**没有正文**的下游技能。
   *
   * 界面上要单独列出来并说明「没装」，因为用户点一次安装却只看到
   * 1 个技能时会怀疑是不是装漏了。
   */
  referenced_not_installed: string[]
  /** 因为不是 .md / 不是 manifest 而被丢掉的条目数。 */
  skipped_other: number
  compressed_bytes: number
  uncompressed_bytes: number
  /** 装完一定是停用的。 */
  enabled: boolean
}

export const HUB_ROUTE = '/api/extensions/skill-hub'

export function listHub(page = 1, pageSize = 20): Promise<HubList> {
  return apiJson<HubList>(`${HUB_ROUTE}?page=${page}&page_size=${pageSize}`)
}

export function installFromHub(slug: string): Promise<HubInstallResult> {
  return apiJson<HubInstallResult>(
    `${HUB_ROUTE}/${encodeURIComponent(slug)}/install`,
    { method: 'POST' },
  )
}

// ---------------------------------------------------------------------------
// 单技能这一层
//
// **与上面的技能集不是同一个东西**，界面也必须分开呈现：
// 技能集实测只有一篇编排说明（`identify.md`）+ 一份点名 6 个下游技能的
// manifest，而那 6 个包里没有正文；单技能才是带 `SKILL.md` 的真技能。
// 把两者混在一张列表里，用户就会以为「装一个包 = 拿到一堆技能」。
// ---------------------------------------------------------------------------

/** 市场里的**一个技能**（不是技能集）。 */
export interface HubSkill {
  slug: string
  /** 上游的名字。可能是中文显示名，也可能是个占位串。 */
  name: string
  description: string
  /** 上游有中文简介时用它。 */
  description_zh: string
  version: string
  category: string
  icon_url: string
  /** 真实安装次数。**`null` 是上游没给**，界面显示「—」而不是 0。 */
  installs: number | null
  downloads: number | null
}

export interface HubSkillSearch {
  host: string
  query: string
  items: HubSkill[]
  /** 上游的 search 就是一个数组，**没有总数**。所以恒为 `null`。 */
  total: null
}

export interface HubRankSection {
  kind: string
  section: string
  count: number
}

export interface HubRankings {
  host: string
  kind: string
  /** 上游给这一份榜单起的名字，如 `hot_downloads`。`null` 表示是聚合的「全部」。 */
  section: string | null
  items: HubSkill[]
  /** 「全部」这一页里每个榜单各有多少条。 */
  sections?: HubRankSection[]
  /** 没有拉到的榜单。**非空就说明这一页不完整**，界面上要说出来。 */
  errors: Record<string, string>
  kinds: string[]
}

export interface HubSkillInstallResult {
  installed: { slug: string; description: string }
  installed_count: number
  source_slug: string
  /** 拿包里的哪个文件当的正文。**说出来**，别让人以为一定是 SKILL.md。 */
  body_file: string
  skipped_other: number
  compressed_bytes: number
  uncompressed_bytes: number
  enabled: boolean
}

export function searchHubSkills(query: string, limit = 20): Promise<HubSkillSearch> {
  return apiJson<HubSkillSearch>(
    `${HUB_ROUTE}/skills?q=${encodeURIComponent(query)}&limit=${limit}`,
  )
}

export function hubRankings(kind: string): Promise<HubRankings> {
  return apiJson<HubRankings>(`${HUB_ROUTE}/rankings?kind=${encodeURIComponent(kind)}`)
}

export function installHubSkill(slug: string): Promise<HubSkillInstallResult> {
  return apiJson<HubSkillInstallResult>(
    `${HUB_ROUTE}/skills/${encodeURIComponent(slug)}/install`,
    { method: 'POST' },
  )
}

/** 界面上显示哪个名字。**中文优先** —— 这是中文界面，英文名是兜底。 */
export function hubLabel(item: HubSkillSet, t: (k: string) => string): string {
  return item.display_name || t('hub.unnamed')
}

/** 有没有正文摘要。**没有就说没有**，不拿 slug 顶替（那不是说明）。 */
export function hubSummary(item: HubSkillSet): string {
  return item.summary || item.summary_en || ''
}

/** 单技能的名字。**中文显示名优先**，都没有就用 slug（那至少是准的）。 */
export function skillLabel(item: HubSkill): string {
  return item.name?.trim() || item.slug
}

/** 单技能的简介。**中文优先**，都没有就是空字符串 —— 空就是空。 */
export function skillSummary(item: HubSkill): string {
  return item.description_zh?.trim() || item.description?.trim() || ''
}

/** 安装次数的原始值。**`null` 是上游没给**，界面显示「—」而不是 0。
 *
 * 不在这里做翻译：翻译要用 i18next 的插值签名，硬塞个 `(k) => string`
 * 进来只会逼调用方写 cast。数据与文案分开，判断「有没有这个数」也归组件。 */
export function installsCount(item: HubSkill): number | null {
  return typeof item.installs === 'number' ? item.installs : null
}
