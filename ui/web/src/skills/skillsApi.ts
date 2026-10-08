import { apiJson } from '../api/client'

/**
 * 技能包（SKILL）的接口层。
 *
 * ## 这个页面为什么以前不存在
 *
 * 后端 `GET/POST/DELETE /api/extensions/skills` 一直都在，**但前端一次都没有
 * 调用过它** —— 技能包只在聊天页的「工具」面板里以一行「能力状态」出现过。
 * 结果是真机上一个技能都开不了：库里所有行都是 `enabled = false`，
 * 而唯一能改这个值的 `PATCH /api/extensions/skills/{name}` 此前压根没登记。
 *
 * **后端做完了、开关没有出口，功能就等于没做。**
 */

/** 一条技能，字段与 `skills_repo::to_json` + `list_skills` 的加工一一对应。 */
export interface Skill {
  /** 技能名，也是模型看到的工具名（`tool_name` 与它同值）。 */
  slug: string
  tool_name: string
  description: string
  version: string
  /** `builtin` 或 `local`。服务端已折成 `kind` 了，这里保留原值便于排查。 */
  source: string
  kind: 'builtin' | 'workspace'
  /**
   * 库里的启用开关。
   *
   * **它与 `model_can_see` 是两件事**，界面上必须分开显示：
   * 开关开了但磁盘正文被删了，模型这一轮照样调不到。
   */
  enabled: boolean
  /** 正文文件路径。 */
  path: string
  // `tool_allowlist` 原本在这里 —— 2026-10-09 随迁移 0014 删掉了那一列（Q102 /
  // ISSUE-008）：它从来没有消费者，界面上也从来没显示过它。
  /** 正文字数。正文文件找不到时为 0。 */
  content_chars?: number
  /** 库里有行但磁盘上没正文。 */
  content_missing?: boolean
  /**
   * **模型这一轮真正看到的那段字。**
   *
   * 与 `description` 在 description 为空时会分叉：库里那一列是空的，
   * 而 `skills_repo::skill_summary` 会退回正文开头一段。
   * 界面算常驻开销**只能用这一个** —— 拿空列去算会得到
   * 「116 个技能 · 常驻开销 0 字符」，一个既好看又危险的说法。
   */
  model_sees_summary?: string
  /**
   * 这一次对话里模型到底看不看得见它 —— 服务端走
   * `tools::skill_visibility` 算的，与 `ToolRegistry::with_skills` 同一个函数。
   */
  model_can_see: boolean
  /** 看得见但被否决时的白话原因，原样显示。 */
  not_mounted_reason?: string
}

export interface SkillList {
  skills?: Skill[]
}

export const SKILLS_ROUTE = '/api/extensions/skills'

export function listSkills(): Promise<SkillList> {
  return apiJson<SkillList>(SKILLS_ROUTE)
}

/** 开关一个技能。**只改这一个字段**，不会碰正文与其它列。 */
export function setSkillEnabled(slug: string, enabled: boolean): Promise<{ skill: Skill }> {
  return apiJson<{ skill: Skill }>(`${SKILLS_ROUTE}/${encodeURIComponent(slug)}`, {
    method: 'PATCH',
    body: JSON.stringify({ enabled }),
  })
}

/** 软删并移除磁盘正文。 */
export function deleteSkill(slug: string): Promise<{ deleted: boolean; name: string }> {
  return apiJson<{ deleted: boolean; name: string }>(
    `${SKILLS_ROUTE}/${encodeURIComponent(slug)}`,
    { method: 'DELETE' },
  )
}

export function skillsOf(body: SkillList | undefined): Skill[] {
  return body?.skills ?? []
}

/**
 * 启用中技能的**常驻字符数**。
 *
 * ## 为什么这个数要摆在页面上
 *
 * 每个启用技能的**摘要**都跟着每一轮请求走（正文只在模型真的调用时才回灌，
 * 见 `skills_repo::as_tool_spec`），所以这是实打实的每轮开销。
 * ISSUE-036 的实测：13 个技能 32430 字符 ≈ 10810 tokens > 8192 窗口，
 * 于是「1+1」都必然 503，而界面上一个字都不提示。
 *
 * **数的是 `model_sees_summary`，不是 `description`。** 两者在 description 为空
 * 时会分叉：库里那一列是空的，模型看到的却是正文开头一段。拿空列去数，
 * 一百多个技能会算出「0 字符」—— 让用户以为可以随便开，而模型每轮都在吃。
 * 换算成 token 只能给估算，而且必须**明写是估算**：quill 没有分词器，
 * 假装算得准就是在编一个数。
 */
export function enabledSummaryChars(skills: Skill[]): number {
  return skills
    .filter((s) => s.enabled)
    .reduce((sum, s) => sum + (s.model_sees_summary ?? '').length, 0)
}
