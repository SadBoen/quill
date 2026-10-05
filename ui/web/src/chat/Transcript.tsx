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
    return running ? <p className="chat-muted">{t('chat.waitingForModel', { defaultValue: '等待模型返回…' })}</p> : null
  }
  return (
    <div className={running ? 'chat-transcript chat-transcript-running' : 'chat-transcript'}>
      {messages.map((message) => <MessageRow key={message.id} message={message} />)}
    </div>
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
