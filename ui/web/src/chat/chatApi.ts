import { ApiError, apiJson } from '../api/client'
import type { CreateSessionResponse, HealthReport, SendMessageResponse, Session } from '../api/types'
import type { MessageHistory } from './model'
import type { SessionMetrics } from './sessionMetrics'

export async function loadSessions(): Promise<Session[]> {
  const body = await apiJson<{ sessions: Session[] }>('/api/sessions')
  return body.sessions
}

export function loadHealth(): Promise<HealthReport> {
  return apiJson<HealthReport>('/healthz')
}

export function createSession(expertId?: string): Promise<CreateSessionResponse> {
  return apiJson<CreateSessionResponse>('/api/sessions', {
    method: 'POST',
    body: JSON.stringify({ ...(expertId ? { expert_id: expertId } : {}) }),
  })
}

export function loadMessageHistory(sessionId: string): Promise<MessageHistory> {
  return apiJson<MessageHistory>(`/api/sessions/${encodeURIComponent(sessionId)}/messages`)
}

export function sendChatMessage(sessionId: string, text: string): Promise<SendMessageResponse> {
  return apiJson<SendMessageResponse>(`/api/sessions/${encodeURIComponent(sessionId)}/messages`, {
    method: 'POST',
    body: JSON.stringify({ content: text }),
  })
}

/**
 * 会话级 Token 统计。统计条的数据源。
 *
 * 返回值里大量字段是 `null` —— 那是「quill 没记这一项」，不是 0。
 * 处理办法在 `sessionMetrics.visibleMetrics()`：直接过滤掉，别补 0。
 */
export function loadSessionMetrics(sessionId: string): Promise<SessionMetrics> {
  return apiJson<SessionMetrics>(`/api/sessions/${encodeURIComponent(sessionId)}/metrics`)
}

export interface UsageSession {
  id: string
  title: string
  expert_id: string | null
  last_active_at: number
  metrics: SessionMetrics
}

export interface UsageReport {
  /** 跨全部计入会话的合计。与逐行相加必然一致（同一个纯函数算的）。 */
  totals: SessionMetrics
  sessions: UsageSession[]
  session_count: number
  limit: number
  /** 真实会话数超过 limit 时为 true —— 界面必须说出来，不能把「最近 N 个」当「全部」。 */
  truncated: boolean
}

export function loadUsage(): Promise<UsageReport> {
  return apiJson<UsageReport>('/api/usage')
}

/**
 * 把 ApiError 变成一句能直接显示给用户的话。
 *
 * 必须带上 `nextStep`：服务端在错误信封里认真写了「下一步：…」
 * （调大 QUILL_LLM_MAX_CONTEXT_TOKENS、减少挂着的技能、别去重启模型服务…），
 * **丢掉它等于让用户对着一句「HTTP 400」干瞪眼**。项目硬规矩是面向用户的
 * 错误一律带「下一步」，前端这里就是最后一环。
 */
export function chatErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof ApiError) {
    const codeSuffix = `(${error.code})`
    const withCode = error.message.includes(codeSuffix)
      ? error.message
      : `${error.message} ${codeSuffix}`
    const step = error.nextStep?.trim()
    if (!step || withCode.includes(step)) return withCode
    return `${withCode} ${step}`
  }
  return error instanceof Error ? error.message : fallback
}
