/**
 * `GET /api/auth/me` 的响应（`api_auth.rs::me`）。
 *
 * 注意形状：服务端现在回**真实用户名**（`username` / `display_name` / `role`），
 * 不再回顶层 `user_id`。token 走 `QUILL_TOKENS` 时这些字段取自名册，
 * 走登录签发的会话令牌时取自库里 profile；取不到 profile 时退化成 32 位 hex
 * 并置 `profile_unavailable: true`。
 */
export interface User {
  id: string
  username: string
  display_name: string
  role: string
  /** 只有走库里 profile 的分支才有；环境变量令牌那条快路径不带。 */
  status?: string
  is_admin: boolean
  approval_mode: string
  profile_unavailable?: boolean
  profile_error?: string
}

export interface Session {
  id: string
  kind: string
  room_id: string
  title: string
  expert_id: string
  provider_id: string
  model: string
  state: string
  message_count: number
  created_at: number
  last_active_at: number
}

// 专家的唯一权威定义在 experts/api.ts（对齐后端 /api/experts 契约），这里转出去避免两处漂移。
export type { Expert } from '../experts/api'

export interface SendMessageResponse {
  session_id: string
  user_message: { id: string; seq: number; content: string; created_at: number }
  reply: string
  reasoning: string
  message: { id: string; seq: number; content: string; created_at: number }
  finish_reason: string | null
  usage: { input: number; output: number }
  turn_ms: number
}

export interface CreateSessionResponse {
  id: string
  room_id: string
  title: string
  model: string
  state: string
}

export interface HealthReport {
  status: string
  version: string
  addr: string
  ui_assets_available: boolean
  storage: { ready: boolean; detail?: string; db_path?: string; missing_tables?: string[] }
  llm: {
    configured: boolean
    base_url: string
    model: string
    max_tokens: number
    max_context_tokens?: number
    compaction_threshold_tokens?: number
    default_provider_id?: string | null
    provider_count?: number
  }
  warnings: { source: string; message: string }[]
}
