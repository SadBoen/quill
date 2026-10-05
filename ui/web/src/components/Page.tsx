import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { ApiError } from '../api/client'

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
}: {
  eyebrow?: string
  title: string
  description?: string
  actions?: ReactNode
}): ReactNode {
  return (
    <header className="page-header">
      <div>
        {eyebrow ? <span className="eyebrow">{eyebrow}</span> : null}
        <h1>{title}</h1>
        {description ? <p>{description}</p> : null}
      </div>
      {actions ? <div className="page-actions">{actions}</div> : null}
    </header>
  )
}

export function Card({
  title,
  description,
  actions,
  children,
  tone,
  testId,
  rowState,
}: {
  title?: ReactNode
  description?: string
  actions?: ReactNode
  children: ReactNode
  tone?: 'danger'
  /** 测试定位用。MCP 卡片靠它断言「界面渲染了几条 == 服务端返回了几条」。 */
  testId?: string
  /** 这一行在界面上**存没存**（`saved` / `changed` / `new`）。 */
  rowState?: string
}): ReactNode {
  return (
    <section className={`card${tone ? ` card-${tone}` : ''}`} data-testid={testId} data-row-state={rowState}>
      {title || description || actions ? (
        <header className="card-header">
          <div>
            {title ? <h2>{title}</h2> : null}
            {description ? <p>{description}</p> : null}
          </div>
          {actions}
        </header>
      ) : null}
      <div className="card-body">{children}</div>
    </section>
  )
}

export function ErrorNotice({ error }: { error: unknown }): ReactNode {
  const { t } = useTranslation()
  if (!error) return null
  const message = error instanceof Error ? error.message : t('common.requestFailed')
  const code = error instanceof ApiError && error.code && !message.includes(error.code) ? error.code : null
  // 服务端给的「下一步」单独占一行：它常常比错误本身更长（要带可复制命令），
  // 挤在同一行里会被当成半句话读漏。
  const nextStep = error instanceof ApiError ? error.nextStep : null
  return (
    <div className="form-error" role="alert">
      <p>{message}{code ? <> <code>{code}</code></> : null}</p>
      {nextStep ? <p className="form-error-next">{nextStep}</p> : null}
    </div>
  )
}

export function StatusBadge({ children, tone = 'neutral' }: { children: ReactNode; tone?: 'neutral' | 'success' | 'warning' | 'danger' }): ReactNode {
  return <span className={`status-badge status-${tone}`}>{children}</span>
}
