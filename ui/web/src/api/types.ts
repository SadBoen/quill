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
  /**
   * 缓存两项是 nullable：上游没报就是 `null`。**别把它 default 成 0** ——
   * 「模型端没报缓存用量」和「报的是 0」在界面上是两件完全不同的事，
   * 前者该整格不显示，后者该显示 0.0%。
   */
  usage: {
    input: number
    output: number
    cache_read?: number | null
    cache_write?: number | null
  }
  turn_ms: number
  /**
   * 这一段正文是**工具用尽之后、把工具摘掉逼出来的**，没有任何工具核实过。
   * 实测 4B 在这种时候会开始编数字，所以界面必须把它标出来，
   * 否则用户会把一段没核实过的话当成和平时一样可信的回答。见 ISSUE-043。
   * 老服务端没有这个字段，按 false 处理。
   */
  final_answer_forced?: boolean
  /**
   * 这一轮开始前**历史**的压缩状态（queue Q018）。数字是估算值
   * （quill 不带分词器，字段名里带 `estimated` 就是不许界面把它当实测值显示）。
   *
   * `compacted:false` 且 `note` 为空 = 没超阈值，本来就不该压；
   * `note` 非空 = 该压却没压成，界面必须把它显示出来（不许静默降级）。
   * 老服务端没有这个字段，按「未知」处理（不要假装成「未压缩」）。
   */
  context?: {
    estimated_history_tokens: number
    threshold_tokens: number
    compacted: boolean
    after_tokens: number
    summary_tokens: number
    note?: string | null
  }
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
