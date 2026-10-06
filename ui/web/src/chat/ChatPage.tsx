import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { FormEvent, ReactNode } from 'react'
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useNavigate, useParams } from 'react-router-dom'

import { ApiError, apiJson, setToken, getToken } from '../api/client'
import { EXPERTS_KEY, listExperts } from '../experts/api'
import { Popconfirm } from '../experts/ExpertsUi'
import { loadSessions, createSession, loadHealth, loadMessageHistory, loadSessionMetrics, chatErrorMessage } from './chatApi'
import { upsertMessage, type ChatMessage } from './model'
import SessionMetricsBar from './SessionMetricsBar'
import { ContextWindowChart } from '../usage/ContextWindowChart'
import { ChatSidebar } from './ChatSidebar'
import { ChatWelcome } from './ChatWelcome'
import { useChatWelcome } from './welcomeContent'
import { Transcript, LiveReply, type LiveToolStep } from './Transcript'
import { streamChatMessage } from './chatStream'
import { CAPABILITY_GAPS, capabilityStatusLabel } from '../capabilityGaps'
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
 * 工具坞里那些还没做到能用的能力。这里只列名字 + 路由 + 真实的接通程度，
 * 不画点了没反应的按钮，也不放假状态。数据来自 `../capabilityGaps`，
 * 与专家页的「能力缺口」表共用一份，两处口径不会各说各话。
 */

function formatTokens(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '0'
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`
  if (value >= 1000) return `${(value / 1000).toFixed(1)}K`
  return String(Math.round(value))
}

/**
 * 从尾巴上抹掉服务端点名要丢弃的那一段。
 *
 * 服务端给的是**原文**而不是长度：只给个数字，前端就得靠「删 N 个字符」猜，
 * 而中文一个字一个码位、代理对又是两个 —— 猜错了就是界面留下半句话。
 * 尾巴对不上时宁可全清：宁可少显示，也不能留着该丢的那段。
 */
export function dropTail(buffer: string, dropped: string): string {
  if (!dropped) return buffer
  return buffer.endsWith(dropped) ? buffer.slice(0, buffer.length - dropped.length) : ''
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
  // 会话级统计。发完一条消息后 invalidate 一下，让新数据进来。
  const metrics = useQuery({
    queryKey: ['session-metrics', sessionId],
    queryFn: () => loadSessionMetrics(sessionId as string),
    enabled: Boolean(sessionId),
    staleTime: 5_000,
    retry: false,
  })
  const allExperts = experts.data ?? []
  const pickableExperts = allExperts.filter((expert) => expert.default_enabled)
  const hiddenExpertCount = allExperts.length - pickableExperts.length
  // 「不允许未选角色就聊天」的前端一半：没显式选时就落在通用专家上。
  // 服务端建会话时也会兜一层，但界面不能先摆出一个「未选角色」的中间态给用户看。
  // is_general 由服务端给，不在前端硬编码 'general' —— 后端换 id 时界面会跟着说谎。
  const generalExpert = allExperts.find((expert) => expert.is_general)
  const session = sessions.data?.find((candidate) => candidate.id === sessionId)
  const [history, setHistory] = useState<ChatMessage[] | null>(null)
  const [historyError, setHistoryError] = useState<string | null>(null)
  const [expertId, setExpertId] = useState('')
  const [text, setText] = useState('')
  const [sending, setSending] = useState(false)
  const [notice, setNotice] = useState<NoticeState | null>(null)
  // 服务端在「工具往返用尽、把工具摘掉逼出来」的正文上会带这个标记。
  // 那段话没有任何工具核实过，界面上必须说明，不能让它长得跟平常的回答一样。
  const [forcedAnswer, setForcedAnswer] = useState(false)
  const [renaming, setRenaming] = useState(false)
  const [titleDraft, setTitleDraft] = useState('')
  const [deletingId, setDeletingId] = useState<string | null>(null)
  const deleting = deletingId !== null
  const [lastUsage, setLastUsage] = useState<{ input: number; output: number } | null>(null)
  /**
   * 正在流式生成的那条回复。
   *
   * 它**不进 history**：那一行还没有 id 与 seq，塞进去就得编一个假的。
   * `started` 用来区分「一个字都还没有」与「有内容了」——前者显示原来那句
   * 「等待模型返回…」，后者把已经收到的字渲染出来。
   */
  const [live, setLive] = useState<{ started: boolean; text: string; reasoning: string; tools: LiveToolStep[] } | null>(null)
  const [toolsOpen, setToolsOpen] = useState(false)
  const [sidebarOpen, setSidebarOpen] = useState(false)
  const chatScroll = useRef<HTMLDivElement>(null)
  const composerInput = useRef<HTMLTextAreaElement>(null)
  const generation = useRef(0)

  const activeExpertId = sessionId ? (session?.expert_id ?? '') : expertId
  const activeExpert = pickableExperts.find((expert) => expert.id === (activeExpertId || generalExpert?.id || ''))
  const expertLabel = activeExpert?.display_name || activeExpert?.id || (experts.isPending ? t('chat.title.loadingExpert', { defaultValue: '角色加载中…' }) : t('chat.title.noExpert', { defaultValue: '没有可用角色' }))
  const welcome = useChatWelcome(activeExpertId || generalExpert?.id || '', activeExpert?.source_template ?? null)
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
    // 正在发送时**不要**重新拉历史。
    //
    // 从 /chat 直接发第一条消息时，navigate 换掉 sessionId 会触发这个 effect，
    // 而它的 GET 与 POST 是并发发出的 —— GET 先落地（那时用户消息还没写库，
    // 返回空列表），POST 随后把 `user_message` 帧插进 history，于是这条刚画
    // 出来的气泡被迟到的 GET 冲掉，用户看着自己的话凭空消失。
    // 发送中界面上的内容由这一轮自己负责维护，不交给历史拉取。
    if (sending) return
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
  }, [sessionId, sending, t])

  useLayoutEffect(() => {
    const root = chatScroll.current
    if (root) root.scrollTop = root.scrollHeight
  }, [history, sending])

  const refreshSessions = useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: ['sessions'] })
  }, [queryClient])

  /**
   * 发送失败后重新拉一次历史。
   *
   * 失败时那条用户消息**已经落库了**（实测：服务端 `/messages` 里有 1 条，
   * `status=complete`），但界面可能停在欢迎屏上、用户那条消息**看不见**——
   * 用户会以为自己根本没发出去，于是重发一遍，或者干脆以为 quill 坏了。
   * 所以失败路径上必须以**服务端为准**重新对齐一次，不能只把文本塞回输入框。
   */
  const reloadHistory = useCallback(async (id: string | null) => {
    if (!id) return
    try {
      const body = await loadMessageHistory(id)
      generation.current += 1
      setHistory(body.messages)
      setHistoryError(null)
    } catch (error) {
      setHistoryError(
        chatErrorMessage(error, t('chat.historyLoadFailed', { defaultValue: '历史加载失败。' })),
      )
    }
  }, [t])

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
    // 这条消息**实际发到哪个会话**，得在这里就记住。
    //
    // 原来 catch 里用的是 `sessionId ?? null`，那是从渲染闭包里读的旧值：
    // 从 /chat 直接发第一条消息时它还是 undefined，于是 catch 把 notice 记到
    // `sessionId: null` 上；而第 83 行是按**当前路由的** sessionId 过滤的，
    // 那时 navigate 已经把地址换成了新会话 id —— `null === "F565D514…"` 不成立，
    // `visibleNotice` 永远是 null。**发送失败的提示因此一条都显示不出来。**
    // 后端其实把可执行的「下一步：…」都写好了，被这里吞得干干净净。
    let targetIdForNotice: string | null = sessionId ?? null
    try {
      const targetId = sessionId ?? (await createSession(expertId || generalExpert?.id)).id
      targetIdForNotice = targetId
      if (!sessionId) navigate(`/chat/${targetId}`, { replace: true })
      setHistory((current) => current ?? [])
      setLive({ started: false, text: '', reasoning: '', tools: [] })
      const result = await streamChatMessage(targetId, sentText, {
        onEvent: (event) => {
          if (event.kind === 'user_message') {
            // 用户那条已经落库了，先把它画出来 —— 不然「等模型返回…」会一直
            // 顶在用户自己那句话上面，看着像没发出去。
            setHistory((current) => upsertMessage(current ?? [], {
              id: event.data.id,
              seq: event.data.seq,
              role: 'user',
              status: 'complete',
              content: event.data.content,
              reasoning: '',
              input_tokens: 0,
              output_tokens: 0,
              turn_ms: 0,
              error_code: '',
              created_at: event.data.created_at,
            }))
            return
          }
          if (event.kind === 'delta') {
            setLive((current) => {
              if (!current) return current
              return event.data.deltaKind === 'reasoning'
                ? { ...current, started: true, reasoning: current.reasoning + event.data.text }
                : { ...current, started: true, text: current.text + event.data.text }
            })
            return
          }
          if (event.kind === 'tool_call') {
            setLive((current) => current && {
              ...current,
              started: true,
              tools: [...current.tools, { name: event.data.name, ok: null }],
            })
            return
          }
          if (event.kind === 'tool_result') {
            setLive((current) => {
              if (!current) return current
              const steps = [...current.tools]
              for (let i = steps.length - 1; i >= 0; i -= 1) {
                if (steps[i].ok === null && steps[i].name === event.data.name) {
                  steps[i] = { ...steps[i], ok: event.data.ok }
                  break
                }
              }
              return { ...current, tools: steps }
            })
            return
          }
          if (event.kind === 'discard') {
            // 这一轮的正文会被工具往返覆盖掉。**必须真的抹掉**：留着的话用户
            // 会看着一段中途变调的答案，而存档里只有最终那段。
            setLive((current) => {
              if (!current) return current
              return {
                ...current,
                text: dropTail(current.text, event.data.text),
                reasoning: dropTail(current.reasoning, event.data.reasoning),
              }
            })
          }
        },
      })
      setLive(null)
      setLastUsage({ input: result.usage.input, output: result.usage.output })
      setForcedAnswer(Boolean(result.final_answer_forced))
      // 刚这一轮的 token 已落库，会话级统计要跟着更新。
      void queryClient.invalidateQueries({ queryKey: ['session-metrics', targetId] })
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
        sessionId: targetIdForNotice,
        message: chatErrorMessage(error, t('chat.sendFailed', { defaultValue: '消息发送失败，请重试。' })),
      })
      // 消息其实已经落库了，把界面拉回服务端的样子（ISSUE-039）
      await reloadHistory(targetIdForNotice)
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
          defaultValue: '本次 {{used}} / 配置上限 {{limit}}',
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
            {forcedAnswer ? (
              <p className="chat-banner chat-banner-error" role="alert">
                {t('chat.forcedFinalAnswer', {
                  defaultValue:
                    '下面的回答是在工具用尽、并且已经不给模型任何工具的情况下生成的，没有经过任何工具核实。',
                })}
              </p>
            ) : null}
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
              live?.started ? (
                <LiveReply text={live.text} reasoning={live.reasoning} tools={live.tools} />
              ) : (
                <article className="chat-message chat-message-assistant chat-message-live">
                  <header><strong>Quill</strong><span>{t('chat.generating', { defaultValue: '思考中…' })}</span></header>
                  <p className="chat-muted">{t('chat.waitingForModel', { defaultValue: '等待模型返回…' })}</p>
                </article>
              )
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
                    {/* 这里**不再有**「（默认）」空选项：空选择就是「未选角色」，
                        而未选角色时其实没有任何人格在跑 —— 界面上给这个选项就是在骗人。
                        兜底改由 generalExpert 承担，服务端建会话时也会再兜一层。 */}
                    <select value={expertId || generalExpert?.id || ''} onChange={(event) => setExpertId(event.target.value)}>
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
                        {t('chat.tools.panelTitle', { defaultValue: '工具（还没做到能用）' })}
                      </p>
                      <p className="chat-tools-hint">
                        {t('chat.tools.hint', {
                          defaultValue: '技能包已挂进对话的工具表，模型能在对话里调用；MCP 还没接协议层，对话里调不到；插件与定时任务的路由压根不存在。这里只标出真实状态与路由，不放点了没反应的按钮。',
                        })}
                      </p>
                      <ul className="chat-tools-list">
                        {CAPABILITY_GAPS.map((tool) => (
                          <li key={tool.route}>
                            <span className="chat-tools-name">{t(tool.labelKey, { defaultValue: tool.fallback })}</span>
                            <code className="chat-tools-route">{tool.route}</code>
                            <span className="chat-tools-state">
                              {capabilityStatusLabel(tool, t)}
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
                            '已配置上下文上限 {{limit}} tokens、压缩阈值 {{threshold}} tokens —— 这两个都是 /healthz 里的配置值，不是模型实测值。模型端点的真实窗口可能更小，quill 不去猜它：超了会被模型服务直接拒绝。压缩尚未实现，超过上限不会自动摘要，需要自己新建会话。',
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
          {/* 会话级统计。放在输入框下面：它是会话整体的数据，不是「本次」的。
              没数据时组件自己返回 null，不占位置。 */}
          <SessionMetricsBar metrics={metrics.data ?? null} />
          {/* 上下文窗口图（抄 octop 的 ContextWindowRing）。
              只在有实测值时出现 —— 没量过就整块不显示，不能画一个 0% 的空环。 */}
          {sessionId && (metrics.data?.turns ?? 0) > 0 ? (
            <ContextWindowChart sessionId={sessionId} context={null} />
          ) : null}
          <p className="chat-ai-disclaimer">
            {t('chat.disclaimer', { defaultValue: 'AI 生成内容，请注意甄别。' })}
          </p>
        </div>
      </section>
    </div>
  )
}
