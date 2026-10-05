export interface ErrorEnvelope {
  code: string
  message: string
}

/**
 * 服务端实际下发的错误信封。
 *
 * 后端（`crates/quill-server/src/error.rs`）发的是
 * `{"error":{"code","detail","next_step"}}` —— 注意是**嵌套一层**的
 * `error`，且字段叫 `detail` / `next_step`，不是顶层的 `code` / `message`。
 *
 * 这个类型曾经按「顶层 code + message」写，于是**全站**每一个接口报错都
 * 退化成 `Request failed (500)`：后端精心写的中文说明和「下一步：…」
 * 一条都到不了用户眼前。登录页的 429 提示、封禁提示、防枚举提示全都靠它。
 */
interface ServerErrorBody {
  error: {
    code: string
    detail: string
    next_step: string
  }
}

export class ApiError extends Error {
  readonly status: number
  readonly code: string

  /** 服务端给的修复动作；没有就为 null。 */
  readonly nextStep: string | null

  /**
   * 429 响应头 `Retry-After` 里的秒数；非 429 或响应头缺失时为 null。
   *
   * 服务端只在 429 上发这个头（`error.rs` 的 `retry_after_secs`），
   * 所以这里读原始响应头而**不是**从错误信封里刨 —— 信封里只有三个中文字段。
   * 登录页要靠它把提交按钮按住到限流窗口结束，否则用户会连着撞。
   */
  readonly retryAfterSeconds: number | null

  constructor(
    status: number,
    code: string,
    message: string,
    nextStep: string | null = null,
    retryAfterSeconds: number | null = null,
  ) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
    this.nextStep = nextStep
    this.retryAfterSeconds = retryAfterSeconds
  }
}

/** `Retry-After` 是「秒数或 HTTP 日期」，这里只认服务端实际下发的秒数形式。 */
function readRetryAfterSeconds(response: Response): number | null {
  if (response.status !== 429) return null
  const raw = response.headers.get('Retry-After')
  if (!raw) return null
  const seconds = Number(raw.trim())
  return Number.isFinite(seconds) && seconds >= 0 ? seconds : null
}

export const TOKEN_STORAGE_KEY = 'quill-token'

export function getToken(): string {
  try {
    return window.localStorage.getItem(TOKEN_STORAGE_KEY) ?? ''
  } catch {
    return ''
  }
}

export function setToken(token: string): void {
  try {
    if (token) window.localStorage.setItem(TOKEN_STORAGE_KEY, token)
    else window.localStorage.removeItem(TOKEN_STORAGE_KEY)
  } catch {
    // A blocked storage API should not block the request itself.
  }
}

function isErrorEnvelope(value: unknown): value is ServerErrorBody {
  if (!value || typeof value !== 'object') return false
  const envelope = value as Record<string, unknown>
  if (!envelope.error || typeof envelope.error !== 'object') return false
  const inner = envelope.error as Record<string, unknown>
  return typeof inner.code === 'string' && typeof inner.detail === 'string'
}

export async function apiJson<T = undefined>(
  input: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers)
  if (init.body && !(init.body instanceof FormData) && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json')
  }
  const token = getToken()
  if (token && !headers.has('Authorization')) {
    headers.set('Authorization', `Bearer ${token}`)
  }

  const response = await fetch(input, {
    ...init,
    credentials: 'same-origin',
    headers,
  })

  if (response.status === 204) return undefined as T

  const isJson = response.headers.get('content-type')?.includes('application/json') ?? false
  const body: unknown = isJson ? await response.json() : undefined
  if (!response.ok) {
    if (isErrorEnvelope(body)) {
      const inner = body.error
      const nextStep = typeof inner.next_step === 'string' ? inner.next_step : null
      throw new ApiError(response.status, inner.code, inner.detail, nextStep, readRetryAfterSeconds(response))
    }
    throw new ApiError(
      response.status,
      'request_failed',
      `Request failed (${response.status})`,
      null,
      readRetryAfterSeconds(response),
    )
  }
  return body as T
}
