import { apiJson } from '../api/client'

/**
 * `GET /api/sessions/{id}/context` 的响应。
 *
 * 两处「单位不同」是刻意的，界面上必须照着显示：
 * - `used_tokens` / `max_tokens`：**token**，来自模型端实测 + 配置。
 * - `segments[].chars`：**字符**。quill 没有分词器，
 *   把字符数说成 token 数就是凭空造数字。
 */
export interface ContextSegment {
  key: string
  chars: number
}

export interface SessionContext {
  max_tokens: number
  /** null = 还没跟模型说过话，没有实测值。不是 0。 */
  used_tokens: number | null
  /** null = 没实测值（分母也可能为 0）。 */
  used_percent: number | null
  segments: ContextSegment[]
  segment_unit: string
}

export function loadSessionContext(sessionId: string): Promise<SessionContext> {
  return apiJson<SessionContext>(`/api/sessions/${encodeURIComponent(sessionId)}/context`)
}