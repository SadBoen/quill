/**
 * 角色头像：只靠 id 生成一个首字母 + 稳定的配色，不引入任何图片资源
 * （Octop 的吉祥物/图标是他们的美术资产，这里只借版式不借素材）。
 */

export const EXPERT_TONES = ['accent', 'success', 'warning', 'danger', 'ink', 'muted'] as const

export type ExpertTone = (typeof EXPERT_TONES)[number]

export const DEFAULT_EXPERT_TONE: ExpertTone = 'muted'

/** 专家 id 是 kebab-case，取首字母大写；空 id 返回「?」。 */
export function expertInitial(expertId: string): string {
  const first = expertId.trim().charAt(0)
  return first ? first.toUpperCase() : '?'
}

/** 同一个 id 永远落到同一个色调，避免每次渲染跳色。 */
export function expertTone(expertId: string): ExpertTone {
  let hash = 0
  for (let index = 0; index < expertId.length; index += 1) {
    hash = (hash * 31 + expertId.charCodeAt(index)) >>> 0
  }
  return EXPERT_TONES[hash % EXPERT_TONES.length] ?? DEFAULT_EXPERT_TONE
}
