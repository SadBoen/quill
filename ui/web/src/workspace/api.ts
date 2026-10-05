import { apiJson } from '../api/client'

/**
 * quill 没有工作区文件接口（`/api/workspace/*` 全部未登记），
 * 工作区页因此改接真实存在的备份与升级路由：`/api/backup/*`、`/api/upgrade/*`。
 * 这几个路由在当前实例上已登记但尚未实现，调用会拿到 501 与服务端原文。
 */
export const BACKUP_EXPORT_ROUTE = '/api/backup/export'
export const BACKUP_VERIFY_ROUTE = '/api/backup/verify'
export const UPGRADE_CHECK_ROUTE = '/api/upgrade/check'
export const UPGRADE_PREPARE_ROUTE = '/api/upgrade/prepare'
export const UPGRADE_HISTORY_ROUTE = '/api/upgrade/history'
export const WORKSPACE_FILE_ROUTE = '/api/workspace/files'

/** `POST /api/backup/export` 的响应。字段名待后端定稿，先按可选约定。 */
export interface BackupArtifact {
  path?: string
  size_bytes?: number
  checksum?: string
  created_at?: string
}

/** `POST /api/backup/verify` 的请求与响应。 */
export interface BackupVerifyRequest {
  path?: string
}

export interface BackupVerifyResult {
  ok?: boolean
  detail?: string
}

/** `GET /api/upgrade/check` 的响应。 */
export interface UpgradeCheck {
  current_version?: string
  latest_version?: string
  update_available?: boolean
}

/** `GET /api/upgrade/history` 的响应。 */
export interface UpgradeHistoryEntry {
  version?: string
  started_at?: string
  finished_at?: string
  status?: string
}

export interface UpgradeHistory {
  entries?: UpgradeHistoryEntry[]
}

export function exportBackup(): Promise<BackupArtifact> {
  return apiJson<BackupArtifact>(BACKUP_EXPORT_ROUTE, { method: 'POST', body: JSON.stringify({}) })
}

export function verifyBackup(body: BackupVerifyRequest): Promise<BackupVerifyResult> {
  return apiJson<BackupVerifyResult>(BACKUP_VERIFY_ROUTE, { method: 'POST', body: JSON.stringify(body) })
}

export function checkUpgrade(): Promise<UpgradeCheck> {
  return apiJson<UpgradeCheck>(UPGRADE_CHECK_ROUTE)
}

export function prepareUpgrade(): Promise<UpgradeHistoryEntry> {
  return apiJson<UpgradeHistoryEntry>(UPGRADE_PREPARE_ROUTE, { method: 'POST', body: JSON.stringify({}) })
}

export function listUpgradeHistory(): Promise<UpgradeHistory> {
  return apiJson<UpgradeHistory>(UPGRADE_HISTORY_ROUTE)
}
