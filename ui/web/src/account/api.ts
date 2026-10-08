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
 * 忘掉本机那份令牌（localStorage `quill-token`）。
 *
 * **它不是登出本身**：真正的登出走 `auth/api.ts` 的 `endSession()` →
 * `POST /api/auth/logout`（服务端会吊销这个会话，那个端点是**实现了的**）。
 * 这里只抹掉浏览器里那一份 —— 调用方必须**先** `endSession()` **再**调它；
 * 顺序反了就会留下一个服务端仍然有效的会话。
 *
 * （这条注释原来写的是「`POST /api/auth/logout` 在 quill 里尚未实现」——
 *   那句是假的，2026-10-09 逐条核前端路由时改掉。）
 */
export function forgetLocalToken(): void {
  setToken('')
}
