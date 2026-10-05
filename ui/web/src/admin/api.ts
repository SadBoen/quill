import { apiJson } from '../api/client'

/** `GET /api/users` 的一条记录；quill 尚未实现该路由，字段先按最小集约定。 */
export interface AdminUser {
  id: string
  name: string
  email: string
  is_admin: boolean
  locked: boolean
  bytes_used: number
  quota_bytes: number
}

export const USERS_ROUTE = '/api/users'

/** quill 尚未实现 GET /api/users，失败时由调用方渲染服务端原文。 */
export function listUsers(): Promise<AdminUser[]> {
  return apiJson<AdminUser[]>(USERS_ROUTE)
}

export const ADMIN_CONFIG_ROUTE = '/api/admin/config'
