import { useTranslation } from 'react-i18next'

import { LIBRARY_EXPERTS_BY_ID } from '../experts/library'

export interface ChatQuickPrompt {
  /** 稳定的 key，用于 React 列表 key。 */
  id: string
  title: string
  description: string
  /** 点卡片时填进输入框的正文（不自动发送）。 */
  prompt: string
  /** CSS 变量名或色值；空串表示用默认色调。 */
  color?: string
}

export interface ChatWelcomeContent {
  /** 角色欢迎语；空串表示没有模板，主区回落到通用空态文案。 */
  welcome: string
  quickPrompts: ChatQuickPrompt[]
  /** 欢迎语/快捷提问的来源标记，用于在界面上如实说明这是模板文案而非用户自己写的。 */
  fromLibrary: boolean
  /** 命中的专家库模板 id（没命中时为空串）。 */
  libraryId: string
  /** 专家是「基于模板生成」出来的（专家 id ≠ 模板 id），欢迎语借自它的来源模板。 */
  inherited: boolean
}

export const EMPTY_WELCOME: ChatWelcomeContent = {
  welcome: '',
  quickPrompts: [],
  fromLibrary: false,
  libraryId: '',
  inherited: false,
}

/**
 * 欢迎语 / 快捷提问来自专家库模板（Octop 预设专家的 manifest）。
 *
 * 两条命中路径：
 *  1. 专家 id 就是模板 id（早期一键添加留下的，或用户照模板 id 建）；
 *  2. 专家是「基于模板生成」的（id 形如 ai-coding-coach-3），此时退回它的
 *     source_template——否则派生出来的专家一进对话页就是没内容的光板欢迎屏。
 *
 * 两条都没有就老实返回空：纯手建专家没有模板，主区回落通用空态文案，
 * 不会拿别的专家的欢迎语凑数。
 */
export function useChatWelcome(expertId: string, sourceTemplate?: string | null): ChatWelcomeContent {
  const { i18n } = useTranslation()
  const direct = expertId ? LIBRARY_EXPERTS_BY_ID[expertId] : undefined
  const inherited = !direct && !!sourceTemplate ? LIBRARY_EXPERTS_BY_ID[sourceTemplate] : undefined
  const source = direct ?? inherited
  if (!source) return EMPTY_WELCOME

  const en = i18n.resolvedLanguage?.startsWith('en') ?? false
  const pick = (zh: string, fallback: string): string => (en ? fallback : zh) || zh || fallback

  return {
    welcome: pick(source.welcomeZh, source.welcomeEn),
    fromLibrary: true,
    inherited: !!inherited,
    libraryId: source.id,
    quickPrompts: source.quickPrompts.map((item, index) => ({
      id: `${source.id}-${index}`,
      title: pick(item.titleZh, item.titleEn),
      description: pick(item.descriptionZh, item.descriptionEn),
      prompt: pick(item.promptZh, item.promptEn),
      color: item.color || undefined,
    })),
  }
}
