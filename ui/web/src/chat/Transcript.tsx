import type { ReactNode } from 'react'
import ReactMarkdown from 'react-markdown'
import { useTranslation } from 'react-i18next'
import remarkGfm from 'remark-gfm'

import { messageBlocks, type ChatMessage, type ContentBlock } from './model'

function MessageRow({ message }: { message: ChatMessage }): ReactNode {
  const { i18n, t } = useTranslation()
  const isHuman = message.role === 'user'
  return (
    <article className={`chat-message chat-message-${isHuman ? 'user' : 'assistant'}`}>
      <header>
        <strong>{isHuman ? t('chat.you', { defaultValue: '我' }) : 'Quill'}</strong>
        <span>{formatTime(message.created_at, i18n.resolvedLanguage)}</span>
      </header>
      <ContentBlocks blocks={messageBlocks(message)} />
      {!isHuman && message.turn_ms > 0 ? (
        <p className="chat-muted">
          {t('chat.usage', {
            ms: message.turn_ms,
            input: message.input_tokens,
            output: message.output_tokens,
            defaultValue: '{{ms}} ms · 入 {{input}} / 出 {{output}} tokens',
          })}
        </p>
      ) : null}
      {message.error_code ? <p className="chat-banner chat-banner-error">{message.error_code}</p> : null}
    </article>
  )
}

export function Transcript({
  messages,
  running,
}: {
  messages: ChatMessage[]
  running: boolean
}): ReactNode {
  const { t } = useTranslation()
  if (!messages.length) {
    // `data-testid` 挂在**这一段**上而不是外层容器：失败重发时输入框里也有
    // 同样那句文本，按文本找会一次命中两个地方。
    return running ? (
      <p className="chat-muted" data-testid="chat-transcript">
        {t('chat.waitingForModel', { defaultValue: '等待模型返回…' })}
      </p>
    ) : null
  }
  return (
    <div
      data-testid="chat-transcript"
      className={running ? 'chat-transcript chat-transcript-running' : 'chat-transcript'}
    >
      {messages.map((message) => <MessageRow key={message.id} message={message} />)}
    </div>
  )
}

export interface LiveToolStep {
  name: string
  /** `null` = 还在跑；`false` = 失败。失败的必须说出来，不能一律显示成「已调用」。 */
  ok: boolean | null
}

/**
 * 正在生成的那条回复。
 *
 * 单独一个组件而不是把占位消息塞进 `history`：那一行还没有 id 与 seq，
 * 塞进去就得编一个假的，而假 id 早晚会和真 id 撞上（`upsertMessage` 按 id 去重）。
 */
export function LiveReply({
  text,
  reasoning,
  tools,
}: {
  text: string
  reasoning: string
  tools: LiveToolStep[]
}): ReactNode {
  const { t } = useTranslation()
  const blocks: ContentBlock[] = []
  if (reasoning.trim()) blocks.push({ type: 'thinking', thinking: reasoning })
  if (text.trim()) blocks.push({ type: 'text', text })
  return (
    <article className="chat-message chat-message-assistant chat-message-live">
      <header>
        <strong>Quill</strong>
        <span>{t('chat.generating', { defaultValue: '生成中…' })}</span>
      </header>
      <ContentBlocks blocks={blocks} />
      {tools.length ? (
        <ul className="chat-tool-steps" data-testid="chat-live-tools">
          {tools.map((step, index) => (
            <li key={`${step.name}-${index}`} data-ok={step.ok === null ? 'running' : step.ok ? 'ok' : 'failed'}>
              {step.ok === null
                ? t('chat.toolRunning', { name: step.name, defaultValue: '正在调用工具 {{name}}…' })
                : step.ok
                  ? t('chat.toolOk', { name: step.name, defaultValue: '工具 {{name}} 已返回' })
                  : t('chat.toolFailed', { name: step.name, defaultValue: '工具 {{name}} 失败' })}
            </li>
          ))}
        </ul>
      ) : null}
    </article>
  )
}

export function ContentBlocks({ blocks }: { blocks: ContentBlock[] }): ReactNode {
  const { t } = useTranslation()
  return blocks.map((block, index) => {
    if (block.type === 'text' && typeof block.text === 'string') {
      return <ReactMarkdown key={index} remarkPlugins={[remarkGfm]}>{block.text}</ReactMarkdown>
    }
    if (block.type === 'thinking' && typeof block.thinking === 'string') {
      return (
        <details key={index} className="chat-thinking">
          <summary>{t('chat.thinking', { defaultValue: '思考过程' })}</summary>
          <p>{block.thinking}</p>
        </details>
      )
    }
    return null
  })
}

function formatTime(value: number, language: string | undefined): string {
  if (!value) return ''
  return new Intl.DateTimeFormat(language === 'zh-CN' ? 'zh-CN' : 'en', {
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value))
}
