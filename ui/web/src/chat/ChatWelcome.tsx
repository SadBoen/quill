import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router-dom'

import type { ChatQuickPrompt } from './welcomeContent'

/** 吉祥物占位：纯 SVG，不引用 Octop 的图片资源。 */
function MascotMark(): ReactNode {
  return (
    <svg className="chat-mascot" viewBox="0 0 96 96" role="img" aria-hidden="true" focusable="false">
      <circle cx="48" cy="48" r="46" fill="var(--accent-soft)" />
      <path
        d="M66 24c-16 2-30 12-38 26-4 7-5 14-4 20 6 1 13 0 20-4 14-8 24-22 26-38z"
        fill="var(--accent)"
        fillOpacity=".18"
        stroke="var(--accent-strong)"
        strokeWidth="2.5"
        strokeLinejoin="round"
      />
      <path d="M28 72c8-12 18-22 32-32" fill="none" stroke="var(--accent-strong)" strokeWidth="2.5" strokeLinecap="round" />
      <circle cx="46" cy="44" r="2.6" fill="var(--accent-strong)" />
    </svg>
  )
}

export interface ChatWelcomeProps {
  /** 当前角色名（没有选角色时传空串，主区会显示通用文案）。 */
  expertName: string
  /** 角色欢迎语；空串时用通用空态标题 + 说明。 */
  welcome: string
  quickPrompts: ChatQuickPrompt[]
  /** 点快捷提问卡片：只把正文填进输入框，不自动发送。 */
  onPickPrompt: (prompt: string) => void
  /** 欢迎语/快捷提问来自专家库模板时为 true，界面上要如实标出来，别让人以为是这个专家自己写的。 */
  fromLibrary?: boolean
  /** 专家是「基于模板生成」的（id 与模板不同），欢迎语借自它的来源模板。 */
  inherited?: boolean
}

/**
 * 空态 / 欢迎屏：吉祥物位 + 粗标题 + 说明 + 快捷提问网格 + 主 CTA。
 * 有欢迎语就显示欢迎语，没有就显示通用文案——不编造角色人格。
 */
export function ChatWelcome({ expertName, welcome, quickPrompts, onPickPrompt, fromLibrary, inherited }: ChatWelcomeProps): ReactNode {
  const { t } = useTranslation()
  const heading = welcome || t('chat.welcome.heading', { defaultValue: '开始一段对话' })
  const description = welcome
    ? expertName
      ? t('chat.welcome.withExpert', {
          expert: expertName,
          defaultValue: '正在和「{{expert}}」对话。下面的卡片会填进输入框，你自己决定什么时候发。',
        })
      : t('chat.welcome.promptHint', { defaultValue: '下面的卡片会填进输入框，你自己决定什么时候发。' })
    : t('chat.welcome.description', { defaultValue: '先挑一个角色，再描述你要做的事；模型会在这条会话里接着上下文往下答。' })

  return (
    <section className="chat-welcome">
      <div className="chat-welcome-inner">
        <MascotMark />
        <h2 className="chat-welcome-title">{heading}</h2>
        <p className="chat-welcome-sub">{description}</p>
        {quickPrompts.length ? (
          <div className="chat-quick">
            <p className="chat-quick-title">
              {t('chat.welcome.quickTitle', { defaultValue: '试试这些' })}
              {fromLibrary ? (
                <span className="chat-quick-source">
                  {inherited
                    ? t('chat.welcome.inheritedFromLibrary', { defaultValue: '（借自它所基于的专家库模板）' })
                    : t('chat.welcome.fromLibrary', { defaultValue: '（来自专家库模板）' })}
                </span>
              ) : null}
            </p>
            <div className="chat-quick-grid">
              {quickPrompts.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className="chat-quick-card"
                  style={item.color ? { borderColor: item.color } : undefined}
                  onClick={() => onPickPrompt(item.prompt)}
                >
                  <span className="chat-quick-card-title">{item.title}</span>
                  {item.description ? <span className="chat-quick-card-desc">{item.description}</span> : null}
                </button>
              ))}
            </div>
          </div>
        ) : (
          <Link className="primary-button chat-welcome-cta" to="/experts">
            {t('chat.welcome.cta', { defaultValue: '去专家页挑一个角色' })}
          </Link>
        )}
      </div>
    </section>
  )
}
