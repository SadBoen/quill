import { apiJson, setToken } from '../api/client'
import type { User } from '../api/types'

export interface ServerVersion {
  version: string
}

/** `GET /api/auth/me`：quill 里当前账号的唯一真实信息源。 */
export function loadAccount(): Promise<User> {
  return apiJson<User>('/api/auth/me')
}

/** `GET /api/version`：用于在页面上确认连到的是哪个实例。 */
export function loadServerVersion(): Promise<ServerVersion> {
  return apiJson<ServerVersion>('/api/version')
}

/**
 * quill 的令牌只存在浏览器里（localStorage `quill-token`），
 * 所以「退出登录」是纯前端动作：`POST /api/auth/logout` 在 quill 里尚未实现。
 */
export function forgetLocalToken(): void {
  setToken('')
}
