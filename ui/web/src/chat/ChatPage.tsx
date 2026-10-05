import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { FormEvent, ReactNode } from 'react'
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useNavigate, useParams } from 'react-router-dom'

import { ApiError, apiJson, setToken, getToken } from '../api/client'
import { EXPERTS_KEY, listExperts } from '../experts/api'
import { Popconfirm } from '../experts/ExpertsUi'
import { loadSessions, createSession, loadHealth, loadMessageHistory, sendChatMessage, chatErrorMessage } from './chatApi'
import { upsertMessage, type ChatMessage } from './model'
import { ChatSidebar } from './ChatSidebar'
import { ChatWelcome } from './ChatWelcome'
import { useChatWelcome } from './welcomeContent'
import { Transcript } from './Transcript'
import './ChatPage.css'
import './chatShell.css'

export interface ChatPageProps {
  pollIntervalMs?: number
}

interface NoticeState {
  sessionId: string | null
  message: string
}

/**
 * 工具坞里那些还没接通的能力。这里只列名字 + 下一步该接哪个路由，
 * 不画点了没反应的按钮，也不放假状态。状态码来自 crates/quill-server/src/routes.rs。
 */
const PENDING_TOOLS = [
  { labelKey: 'chat.tools.skills', fallback: '技能包', route: 'GET /api/extensions/skills', state: '501' },
  { labelKey: 'chat.tools.plugins', fallback: '插件', route: 'GET /api/extensions/plugins', state: '501' },
  { labelKey: 'chat.tools.mcp', fallback: 'MCP', route: 'GET /api/extensions/mcp', state: '501' },
  { labelKey: 'chat.tools.cron', fallback: '定时任务', route: '/api/cron', state: '未注册' },
] as const

function formatTokens(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '0'
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`
  if (value >= 1000) return `${(value / 1000).toFixed(1)}K`
  return String(Math.round(value))
}

export function ChatPage(_props: ChatPageProps): ReactNode {
  const { t } = useTranslation()
  const { sessionId } = useParams<{ sessionId: string }>()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const sessions = useQuery({ queryKey: ['sessions'], queryFn: loadSessions, staleTime: 15_000 })
  // 与专家页、专家库 tab 共用 EXPERTS_KEY，所以 queryFn 的返回形状必须完全一致：
  // 这里曾经用 loadExperts() 包成 { experts: [...] }，而专家页存的是裸数组，
  // 从专家页直接跳过来时命中的是数组，data.experts 为 undefined，下拉就空了。
  const experts = useQuery({ queryKey: EXPERTS_KEY, queryFn: listExperts, staleTime: 60_000 })
  const health = useQuery({ queryKey: ['health'], queryFn: loadHealth, staleTime: 30_000, retry: false })
  const allExperts = experts.data ?? []
  const pickableExperts = allExperts.filter((expert) => expert.default_enabled)
  const hiddenExpertCount = allExperts.length - pickableExperts.length
  const session = sessions.data?.find((candidate) => candidate.id === sessionId)
  const [history, setHistory] = useState<ChatMessage[] | null>(null)
  const [historyError, setHistoryError] = useState<string | null>(null)
  const [expertId, setExpertId] = useState('')
  const [text, setText] = useState('')
  const [sending, setSending] = useState(false)
  const [notice, setNotice] = useState<NoticeState | null>(null)
  const [renaming, setRenaming] = useState(false)
  const [titleDraft, setTitleDraft] = useState('')
  const [deletingId, setDeletingId] = useState<string | null>(null)
  const deleting = deletingId !== null
  const [lastUsage, setLastUsage] = useState<{ input: number; output: number } | null>(null)
  const [toolsOpen, setToolsOpen] = useState(false)
  const [sidebarOpen, setSidebarOpen] = useState(false)
  const chatScroll = useRef<HTMLDivElement>(null)
  const composerInput = useRef<HTMLTextAreaElement>(null)
  const generation = useRef(0)

  const activeExpertId = sessionId ? (session?.expert_id ?? '') : expertId
  const activeExpert = pickableExperts.find((expert) => expert.id === activeExpertId)
  const expertLabel = activeExpert?.display_name || activeExpert?.id || t('chat.title.defaultExpert', { defaultValue: '默认' })
  const welcome = useChatWelcome(activeExpertId, activeExpert?.source_template ?? null)
  const title = session?.title?.trim() ? session.title : t('nav.newChat', { defaultValue: '新对话' })
  const visibleNotice = notice?.sessionId === (sessionId ?? null) ? notice.message : null
  const hasMessages = (history?.length ?? 0) > 0
  const showWelcome = !historyError && !hasMessages && !sending

  const contextLimit = health.data?.llm.max_context_tokens ?? 0
  const compactionThreshold = health.data?.llm.compaction_threshold_tokens ?? 0
  const usedTokens = lastUsage ? lastUsage.input + lastUsage.output : 0

  useEffect(() => {
    if (!sessionId) {
      setHistory(null)
      setHistoryError(null)
      return
    }
    let disposed = false
    generation.current += 1
    const current = generation.current
    setHistory(null)
    setHistoryError(null)
    void loadMessageHistory(sessionId)
      .then((body) => {
        if (!disposed && generation.current === current) setHistory(body.messages)
      })
      .catch((error) => {
        if (!disposed && generation.current === current) {
          setHistoryError(chatErrorMessage(error, t('chat.historyLoadFailed', { defaultValue: '历史加载失败。' })))
        }
      })
    return () => {
      disposed = true
    }
  }, [sessionId, t])

  useLayoutEffect(() => {
    const root = chatScroll.current
    if (root) root.scrollTop = root.scrollHeight
  }, [history, sending])

  const refreshSessions = useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ['sessions'] })
  }, [queryClient])

  function selectExpert(nextExpertId: string): void {
    setExpertId(nextExpertId)
    setSidebarOpen(false)
    if (!nextExpertId) {
      if (sessionId) navigate('/chat')
      return
    }
    const latest = (sessions.data ?? []).find((item) => item.expert_id === nextExpertId)
    if (latest) {
      if (latest.id !== sessionId) navigate(`/chat/${latest.id}`)
      return
    }
    if (sessionId) navigate('/chat')
  }

  function startNewChat(): void {
    setSidebarOpen(false)
    navigate('/chat')
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault()
    const sentText = text.trim()
    if (sending || !sentText) return

    setSending(true)
    setNotice(null)
    setText('')
    try {
      const targetId = sessionId ?? (await createSession(expertId || undefined)).id
      if (!sessionId) navigate(`/chat/${targetId}`, { replace: true })
      setHistory((current) => current ?? [])
      const result = await sendChatMessage(targetId, sentText)
      setLastUsage({ input: result.usage.input, output: result.usage.output })
      setHistory((current) => (current ?? []).reduce(upsertMessage, [
        {
          id: result.user_message.id,
          seq: result.user_message.seq,
          role: 'user',
          status: 'complete',
          content: result.user_message.content,
          reasoning: '',
          input_tokens: 0,
          output_tokens: 0,
          turn_ms: 0,
          error_code: '',
          created_at: result.user_message.created_at,
        },
        {
          id: result.message.id,
          seq: result.message.seq,
          role: 'assistant',
          status: 'complete',
          content: result.message.content,
          reasoning: result.reasoning,
          input_tokens: result.usage.input,
          output_tokens: result.usage.output,
          turn_ms: result.turn_ms,
          error_code: '',
          created_at: result.message.created_at,
        },
      ]))
    } catch (error) {
      setText(sentText)
      setNotice({
        sessionId: sessionId ?? null,
        message: chatErrorMessage(error, t('chat.sendFailed', { defaultValue: '消息发送失败，请重试。' })),
      })
    } finally {
      setSending(false)
      await refreshSessions()
    }
  }

  async function handleRename(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault()
    const nextTitle = titleDraft.trim()
    if (!sessionId || !nextTitle) return
    setNotice({
      sessionId,
      message: t('chat.renameUnsupported', {
        defaultValue: '当前后端尚未开放会话改名（PATCH /api/sessions/{id}）。',
      }),
    })
    setRenaming(false)
  }

  async function handleDelete(targetId?: string): Promise<void> {
    const deleteId = targetId ?? sessionId
    if (!deleteId || deleting) return
    setDeletingId(deleteId)
    try {
      await apiJson(`/api/sessions/${encodeURIComponent(deleteId)}`, { method: 'DELETE' })
      if (deleteId === sessionId) navigate('/chat', { replace: true })
      else setSidebarOpen(false)
    } catch (error) {
      if (error instanceof ApiError && error.status === 404) {
        // 删除是幂等的：再删一次也返回 200 + deleted:false。走到 404 说明这一行
        // 已经不在库里（别处删过，或压根不属于当前用户），不是「接口没接通」。
        setNotice({
          sessionId,
          message: t('chat.deleteGone', {
            defaultValue: '这个会话已经不在了（可能已在别处删除）。下一步：刷新左侧列表，或回列表重新选一个会话。',
          }),
        })
      } else {
        setNotice({
          sessionId,
          message: chatErrorMessage(error, t('chat.deleteFailed', { defaultValue: '会话删除失败。' })),
        })
      }
    } finally {
      setDeletingId(null)
      await refreshSessions()
    }
  }

  const usageLabel = !lastUsage
    ? t('chat.tokenMeter.idle', { defaultValue: '本次尚未产生用量' })
    : contextLimit > 0
      ? t('chat.tokenMeter.label', {
          used: formatTokens(usedTokens),
          limit: formatTokens(contextLimit),
          defaultValue: '本次 {{used}} / 上限 {{limit}}',
        })
      : t('chat.tokenMeter.unknownLimit', {
          used: formatTokens(usedTokens),
          defaultValue: '本次 {{used}}（上限未知）',
        })

  return (
    <div className="chat-shell">
      <ChatSidebar
        experts={pickableExperts}
        expertsPending={experts.isPending}
        sessions={sessions.data ?? []}
        sessionsPending={sessions.isPending}
        activeExpertId={activeExpertId}
        activeSessionId={sessionId ?? null}
        open={sidebarOpen}
        onSelectExpert={selectExpert}
        onOpenSession={(nextId) => {
          setSidebarOpen(false)
          navigate(`/chat/${nextId}`)
        }}
        onDeleteSession={(nextId) => void handleDelete(nextId)}
        deletingId={deletingId}
        onNewChat={startNewChat}
        onClose={() => setSidebarOpen(false)}
      />

      <section className="chat-main">
        <header className="chat-titlebar">
          <button
            type="button"
            className="chat-titlebar-toggle"
            aria-expanded={sidebarOpen}
            aria-label={t('chat.sidebar.open', { defaultValue: '打开角色与会话列表' })}
            title={t('chat.sidebar.open', { defaultValue: '打开角色与会话列表' })}
            onClick={() => setSidebarOpen((value) => !value)}
          >
            ☰
          </button>
          <div className="chat-titlebar-copy">
            <span className="chat-titlebar-expert">{expertLabel}</span>
            <span className="chat-titlebar-sep" aria-hidden="true">·</span>
            <strong className="chat-titlebar-title">{title}</strong>
          </div>
          <div className="chat-session-controls">
            {session ? (
              <span className="chat-session-meta">
                {t('chat.sessionMeta', {
                  count: session.message_count,
                  model: session.model,
                  defaultValue: '{{count}} 条消息 · {{model}}',
                })}
              </span>
            ) : null}
            {sessionId ? (
              <>
                <button
                  type="button"
                  className="chat-secondary-button"
                  onClick={() => {
                    setTitleDraft(session?.title ?? '')
                    setRenaming(true)
                  }}
                >
                  {t('common.edit', { defaultValue: '改名' })}
                </button>
                <Popconfirm
                  className="chat-danger-button"
                  label={t('common.delete', { defaultValue: '删除' })}
                  icon={<span aria-hidden="true">🗑</span>}
                  title={t('chat.deleteConfirmTitle', { defaultValue: '删除这个会话？' })}
                  description={t('chat.deleteConfirmHint', {
                    defaultValue: '这是软删除：会话会从列表里消失，消息记录仍保留在本机数据库中。',
                  })}
                  okText={t('common.delete', { defaultValue: '删除' })}
                  cancelText={t('common.cancel', { defaultValue: '取消' })}
                  dangerOk
                  pending={deleting}
                  onConfirm={() => void handleDelete()}
                />
              </>
            ) : null}
          </div>
        </header>

        <div ref={chatScroll} className="chat-scroll" aria-live="polite">
          <div className="chat-content">
            {renaming ? (
              <form className="chat-rename" onSubmit={(event) => void handleRename(event)}>
                <label htmlFor="chat-title">{t('chat.sessionTitle', { defaultValue: '会话标题' })}</label>
                <input id="chat-title" value={titleDraft} onChange={(event) => setTitleDraft(event.target.value)} maxLength={64} autoFocus />
                <button type="submit" className="primary-button">{t('common.save', { defaultValue: '保存' })}</button>
                <button type="button" className="chat-secondary-button" onClick={() => setRenaming(false)}>{t('common.cancel', { defaultValue: '取消' })}</button>
              </form>
            ) : null}
            {historyError ? <p className="chat-banner chat-banner-error" role="alert">{historyError}</p> : null}
            {visibleNotice ? <p className="chat-banner chat-banner-error" role="alert">{visibleNotice}</p> : null}
            {showWelcome ? (
              <ChatWelcome
                expertName={activeExpert ? expertLabel : ''}
                welcome={welcome.welcome}
                quickPrompts={welcome.quickPrompts}
                fromLibrary={welcome.fromLibrary}
                inherited={welcome.inherited}
                onPickPrompt={(prompt) => {
                  setText(prompt)
                  composerInput.current?.focus()
                }}
              />
            ) : null}
            {history ? <Transcript messages={history} running={sending} /> : null}
            {sending && history ? (
              <article className="chat-message chat-message-assistant chat-message-live">
                <header><strong>Quill</strong><span>{t('chat.generating', { defaultValue: '思考中…' })}</span></header>
                <p className="chat-muted">{t('chat.waitingForModel', { defaultValue: '等待模型返回…' })}</p>
              </article>
            ) : null}
          </div>
        </div>

        <div className="chat-input-dock">
          <form className="chat-input-card" onSubmit={(event) => void handleSubmit(event)}>
            <textarea
              ref={composerInput}
              aria-label={t('draftChat.message', { defaultValue: '消息' })}
              placeholder={t('draftChat.placeholder', { defaultValue: '输入消息，Enter 发送，Shift+Enter 换行' })}
              rows={2}
              value={text}
              onChange={(event) => setText(event.target.value)}
              onKeyDown={(event) => {
                if (
                  event.key !== 'Enter'
                  || event.shiftKey
                  || event.nativeEvent.isComposing
                  || event.nativeEvent.keyCode === 229
                ) return
                event.preventDefault()
                event.currentTarget.form?.requestSubmit()
              }}
              disabled={sending}
            />
            <div className="chat-input-dock-row">
              <div className="chat-input-dock-left">
                {!sessionId ? (
                  <label className="composer-expert">
                    <span>{t('chat.expert', { defaultValue: '专家' })}</span>
                    {/* default_enabled=false 的专家不进下拉，否则专家页那个开关就是个摆设。 */}
                    <select value={expertId} onChange={(event) => setExpertId(event.target.value)}>
                      <option value="">{t('chat.expertNone', { defaultValue: '（默认）' })}</option>
                      {pickableExperts.map((expert) => (
                        <option key={expert.id} value={expert.id}>
                          {expert.display_name || expert.id}
                          {/* 人格正文为空时选它等于没效果，这里明说，避免用户以为是自己选错了。 */}
                          {expert.instructions?.trim() ? '' : ` ${t('chat.expertNoPersona', { defaultValue: '（未设置人格）' })}`}
                        </option>
                      ))}
                    </select>
                  </label>
                ) : null}
                {hiddenExpertCount > 0 ? (
                  <small className="composer-expert-hint">
                    {t('chat.expertHidden', {
                      count: hiddenExpertCount,
                      defaultValue: '另有 {{count}} 个专家已取消「默认启用」，不在这个下拉里。',
                    })}
                  </small>
                ) : null}
                <button
                  type="button"
                  className="text-button"
                  onClick={() => { setToken(getToken()) }}
                >
                  {t('chat.tokenHint', { defaultValue: '令牌已保存在本机浏览器' })}
                </button>
                <span className="chat-input-tools-anchor">
                  <button
                    type="button"
                    className="chat-dock-btn"
                    aria-expanded={toolsOpen}
                    onClick={() => setToolsOpen((value) => !value)}
                  >
                    {t('chat.tools.button', { defaultValue: '工具' })}
                  </button>
                  {toolsOpen ? (
                    <div className="chat-tools-panel" role="note">
                      <p className="chat-tools-title">
                        {t('chat.tools.panelTitle', { defaultValue: '工具（后端尚未接通）' })}
                      </p>
                      <p className="chat-tools-hint">
                        {t('chat.tools.hint', {
                          defaultValue: '技能包、插件、MCP、定时任务这四项后端还没实现，这里只标出下一步该接哪个路由，不放点了没反应的按钮。',
                        })}
                      </p>
                      <ul className="chat-tools-list">
                        {PENDING_TOOLS.map((tool) => (
                          <li key={tool.route}>
                            <span className="chat-tools-name">{t(tool.labelKey, { defaultValue: tool.fallback })}</span>
                            <code className="chat-tools-route">{tool.route}</code>
                            <span className="chat-tools-state">
                              {t('chat.tools.notReady', { state: tool.state, defaultValue: '未接通 · {{state}}' })}
                            </span>
                          </li>
                        ))}
                      </ul>
                    </div>
                  ) : null}
                </span>
              </div>
              <div className="chat-input-dock-right">
                <span
                  className="chat-usage-label"
                  title={
                    contextLimit > 0
                      ? t('chat.tokenMeter.title', {
                          limit: contextLimit,
                          threshold: compactionThreshold || '—',
                          defaultValue:
                            '上下文上限 {{limit}} tokens（来自 /healthz）。已配置压缩阈值 {{threshold}} tokens，但压缩还没实现：超过上限不会自动摘要，需要自己新建会话。',
                        })
                      : t('chat.tokenMeter.noLimit', { defaultValue: '/healthz 里还没有 max_context_tokens，上限未知。' })
                  }
                >
                  {usageLabel}
                </span>
                <button className="send-button" aria-label={t('draftChat.send', { defaultValue: '发送' })} disabled={sending || !text.trim()}>
                  {sending ? '…' : '↑'}
                </button>
              </div>
            </div>
          </form>
          <p className="chat-ai-disclaimer">
            {t('chat.disclaimer', { defaultValue: 'AI 生成内容，请注意甄别。' })}
          </p>
        </div>
      </section>
    </div>
  )
}
