/**
 * 账号域的前端接口层。
 *
 * 契约以 `crates/quill-server/src/api_auth.rs` 为准，这里只做形状对齐，不加业务判断：
 * - `GET /api/setup/status` 决定登录页显示「首管引导」还是「登录表单」。
 * - `POST /api/setup/initial-admin` 只在 users 表为空时成功，重复调用 409。
 * - `POST /api/auth/login` 拿会话令牌；失败时**故意**不区分「口令错」和「用户不存在」。
 *
 * 「注册」在这里**没有**对应函数：服务端根本没有 `/api/auth/register`，
 * 前端也不该凭空造一个入口。
 */

import { apiJson } from '../api/client'

/** `GET /api/setup/status` 的响应。 */
export interface SetupStatus {
  setup_required: boolean
  user_count: number
  /** 服务端恒为 false；保留字段是为了让前端不必靠「路由 404」推断注册已关闭。 */
  registration_enabled: boolean
}

/** `POST /api/auth/login` / `POST /api/auth/refresh` 的响应（两者同形）。 */
export interface SessionResponse {
  access_token: string
  token_type: string
  expires_in: number
  expires_at: number
  user: { id: string; username: string; role: string }
}

/** `POST /api/setup/initial-admin` 的 201 响应。 */
export interface InitialAdminResponse {
  id: string
  username: string
  display_name: string
  role: string
  status: string
}

/** `POST /api/auth/logout` 的响应。幂等：重复登出也返回 200 + `revoked:false`。 */
export interface LogoutResponse {
  revoked: boolean
  revoked_count: number
  note: string
}

export function loadSetupStatus(): Promise<SetupStatus> {
  return apiJson<SetupStatus>('/api/setup/status')
}

/**
 * `POST /api/setup/initial-admin`。
 *
 * 服务端用 `only_keys` 白名单校验请求体，**多传任何一个字段都 400**，
 * 所以这里只在 display_name 非空时才带上它。
 */
export function createInitialAdmin(input: {
  username: string
  password: string
  display_name?: string
}): Promise<InitialAdminResponse> {
  const body: { username: string; password: string; display_name?: string } = {
    username: input.username,
    password: input.password,
  }
  if (input.display_name) body.display_name = input.display_name
  return apiJson<InitialAdminResponse>('/api/setup/initial-admin', {
    method: 'POST',
    body: JSON.stringify(body),
  })
}

/**
 * `POST /api/auth/login`。
 *
 * 失败时后端对「用户不存在」和「口令错误」返回**逐字相同**的 401（防用户名枚举），
 * 所以这里不做任何区分，也不缓存「这个用户名不存在」之类的结论。
 */
export function signIn(input: { username: string; password: string }): Promise<SessionResponse> {
  return apiJson<SessionResponse>('/api/auth/login', {
    method: 'POST',
    body: JSON.stringify({ username: input.username, password: input.password }),
  })
}

/**
 * `POST /api/auth/logout` —— 吊销当前会话令牌。
 *
 * 幂等：拿一个已经登出过的令牌（或环境变量令牌）再调一次仍是 200 + `revoked:false`，
 * 所以调用方**不应该**把 `revoked:false` 当失败 —— 用户确实已经登出了。
 */
export function endSession(): Promise<LogoutResponse> {
  return apiJson<LogoutResponse>('/api/auth/logout', { method: 'POST' })
}
