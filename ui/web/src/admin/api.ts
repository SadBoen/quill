import { apiJson, ApiError } from '../api/client'

/**
 * `GET /api/users` 里的一条记录。
 *
 * 字段是**有来源的那些**，不是「界面上大概想要什么」：以前这里写的是
 * name / email / is_admin / locked / bytes_used / quota_bytes，其中
 * email、locked、配额在这台实例里根本没有来源 —— 报出来就是编的。
 * 现在与服务端 `api_users::user_json` 一字不差。
 */
export interface AdminUser {
  id: string
  username: string
  display_name: string
  role: 'owner' | 'member'
  status: 'active' | 'disabled'
  created_at_ms: number
  last_login_at_ms: number | null
  /**
   * 这个人的凭据是不是一枚 `QUILL_TOKENS` 环境变量令牌（而不是登录签发的会话令牌）。
   *
   * 注意它**不再**表示「停用挡不住他」—— 令牌鉴权会回 `users` 行核状态，
   * 停用对所有令牌都生效。它现在回答的是「登出能不能收回他的凭据」：
   * `/api/auth/logout` 只吊销会话行，收不回环境变量令牌。
   */
  has_env_token: boolean
}

export interface AdminUserList {
  users: AdminUser[]
  /** 过滤后的总数。翻页按钮靠它判断「还有没有下一页」。 */
  total: number
}

export const USERS_ROUTE = '/api/users'
export const ADMIN_CONFIG_ROUTE = '/api/admin/config'
export const USER_ROUTE = (id: string): string => `${USERS_ROUTE}/${encodeURIComponent(id)}`

export const DEFAULT_PAGE_SIZE = 50

function isText(v: unknown): v is string {
  return typeof v === 'string'
}

function isCount(v: unknown): v is number {
  return typeof v === 'number' && Number.isFinite(v)
}

/**
 * 响应自检。
 *
 * 与备份那边同一个理由：服务端回了 200 但字段残缺时，照单全收就会渲染出
 * 一张**看起来有数据、其实不成立**的表。读不懂就当没成功。
 */
export function isAdminUser(value: unknown): value is AdminUser {
  if (!value || typeof value !== 'object') return false
  const u = value as Record<string, unknown>
  if (!isText(u.id) || !isText(u.username) || !isText(u.display_name)) return false
  if (u.role !== 'owner' && u.role !== 'member') return false
  if (u.status !== 'active' && u.status !== 'disabled') return false
  if (!isCount(u.created_at_ms)) return false
  if (u.last_login_at_ms !== null && !isCount(u.last_login_at_ms)) return false
  if (typeof u.has_env_token !== 'boolean') return false
  return true
}

export function isAdminUserList(value: unknown): value is AdminUserList {
  if (!value || typeof value !== 'object') return false
  const v = value as Record<string, unknown>
  if (!isCount(v.total) || !Array.isArray(v.users)) return false
  return v.users.every(isAdminUser)
}

export function listUsers(
  offset: number,
  limit: number = DEFAULT_PAGE_SIZE,
): Promise<AdminUserList> {
  const q = new URLSearchParams({ offset: String(Math.max(0, offset)), limit: String(limit) })
  return apiJson<AdminUserList>(`${USERS_ROUTE}?${q.toString()}`)
}

/** `PATCH /api/users/{id}`，只做启停。 */
export function setUserStatus(id: string, status: 'active' | 'disabled'): Promise<AdminUser> {
  return apiJson<AdminUser>(USER_ROUTE(id), {
    method: 'PATCH',
    body: JSON.stringify({ status }),
  })
}

/** 这类失败对应哪一种「说了不算」的状态。 */
export type UserFailure = 'forbidden' | 'not-found' | 'self-disable' | 'bad-request' | 'unknown'

const FAILURE_BY_STATUS: Record<number, UserFailure> = {
  400: 'bad-request',
  403: 'forbidden',
  404: 'not-found',
  409: 'self-disable',
}

export function userFailure(error: unknown): UserFailure | null {
  if (!(error instanceof ApiError)) return null
  return FAILURE_BY_STATUS[error.status] ?? 'unknown'
}

type T = (key: string, options?: Record<string, unknown>) => string

/** 一句「哪里不行 + 下一步做什么」。服务端原文由 `ErrorNotice` 另外端出来。 */
export function userFailureLabel(failure: UserFailure, t: T): string {
  switch (failure) {
    case 'forbidden':
      return t('admin.userErrorForbidden', {
        defaultValue:
          '你不是 owner，看不了这份名册也改不了别人的账号（403）。下一步：换一个带 admin 的账号登录；本实例的 owner 是部署时由 QUILL_TOKENS 里的 `:admin` 决定的。',
      })
    case 'not-found':
      return t('admin.userErrorNotFound', {
        defaultValue:
          '这个账号已经不在名册里了（404）。下一步：刷新页面拿最新名册，再确认一次操作对象。',
      })
    case 'self-disable':
      return t('admin.userErrorSelfDisable', {
        defaultValue:
          '不能把自己停用（409）：停用之后这条会话立刻失效，你会把自己锁在门外，而且没有第二个 owner 能把你放回来。下一步：让别人停用你，或者改用另一个 owner 账号。',
      })
    case 'bad-request':
      return t('admin.userErrorBadRequest', {
        defaultValue:
          '请求没被接受（400）。下一步：status 只接受 active 或 disabled；id 请用名册里 id 字段的原文，不要自己拼。',
      })
    default:
      return t('admin.userErrorUnknown', {
        defaultValue:
          '这一步失败了，但服务端没说清楚原因。下一步：看服务端日志里带请求 ID 的那一行，或执行 `quill doctor` 打印完整诊断。',
      })
  }
}