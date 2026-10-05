import { ApiError, apiJson } from '../api/client'
import type { CreateSessionResponse, HealthReport, SendMessageResponse, Session } from '../api/types'
import type { MessageHistory } from './model'

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

export function chatErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof ApiError) {
    const codeSuffix = `(${error.code})`
    return error.message.includes(codeSuffix) ? error.message : `${error.message} ${codeSuffix}`
  }
  return error instanceof Error ? error.message : fallback
}
