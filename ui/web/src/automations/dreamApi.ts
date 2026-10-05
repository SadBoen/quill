import { apiJson } from '../api/client'

/**
 * 「梦境」/记忆整理能力在 quill 里没有对应后端：
 * `/api/dream*` 未登记，页面只保留骨架并说明下一步。
 */
export type DreamStatus = 'pending' | 'running' | 'updated' | 'skipped' | 'failed'

export interface DreamItem {
  id: string
  status: DreamStatus
  started_at: string
  finished_at: string | null
  message_count: number
  restored_at: string | null
  error: { code: string | null } | null
}

export interface DreamPage {
  items: DreamItem[]
  next_offset?: number | null
}

export interface DreamDetail {
  id: string
  status: DreamStatus
  before: string | null
  after: string | null
  restored_at: string | null
}

export const DREAM_ROUTE = '/api/dream'

export function listDream(offset: number): Promise<DreamPage> {
  return apiJson<DreamPage>(`${DREAM_ROUTE}?limit=50&offset=${offset}`)
}

export function getDream(id: string): Promise<DreamDetail> {
  return apiJson<DreamDetail>(`${DREAM_ROUTE}/${encodeURIComponent(id)}`)
}

export function restoreDream(id: string): Promise<DreamDetail> {
  return apiJson<DreamDetail>(`${DREAM_ROUTE}/${encodeURIComponent(id)}/restore`, { method: 'POST' })
}
