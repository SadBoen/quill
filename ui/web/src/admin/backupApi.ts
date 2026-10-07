import { ApiError, apiJson } from '../api/client'

/**
 * 备份与升级的接口。
 *
 * **为什么在 admin 而不是工作区**：这两个是实例运维能力，不是文件浏览的附属品。
 * 参照 octop —— 它的备份归 `/admin/backend`（`AdminStoragePage`）、
 * 升级归 `/admin/advanced?tab=updates`（`routes/index.tsx:229,236,244`），
 * 都在 admin 区；而 octop **压根没有工作区页**，
 * `{ path: "/workspace", element: <Navigate to="/experts" replace /> }`（`:213`）
 * 直接把 `/workspace` 重定向走。
 *
 * 我们之前把备份塞在工作区是因为**工作区自己的功能是空的**
 * （`/api/workspace/*` 全部未登记），拿备份来顶替 ——
 * 那是「有内容总比空页好」的妥协，代价是让用户以为备份属于工作区。
 *
 * export / verify 已实现（`POST` 返回 201 / 200，字段见下面的类型）。
 * restore 虽然也登记了，但服务端运行期间数据库文件被它自己占着，
 * 所以它**永远不会报成功** —— 页面上因此不提供还原按钮，只说明怎么做（见 RESTORE_COMMAND）。
 */
export const BACKUP_EXPORT_ROUTE = '/api/backup/export'
export const BACKUP_VERIFY_ROUTE = '/api/backup/verify'
export const BACKUP_RESTORE_ROUTE = '/api/backup/restore'
export const UPGRADE_CHECK_ROUTE = '/api/upgrade/check'
export const UPGRADE_PREPARE_ROUTE = '/api/upgrade/prepare'
export const UPGRADE_HISTORY_ROUTE = '/api/upgrade/history'
export const WORKSPACE_FILE_ROUTE = '/api/workspace/files'

/**
 * 还原命令原文，取自 `crates/quill-cli/src/cmd_backup.rs`：
 * `restore <备份目录>` 是子命令的必填位置参数，`--db` / `--root` / `--yes`
 * 是 `Opts::parse` 里的全局参数，缺 `--yes` 时 CLI 只打印预告、不写任何字节。
 * 路径在浏览器里拿不到，所以留占位符让用户自己换成服务端上的真实路径。
 */
export const RESTORE_COMMAND = 'quill restore <备份目录> --db <数据库路径> --root <数据根目录> --yes'

/** 没被备份的密钥材料：用户必须看见它，否则会误以为这份备份是全的。 */
export interface BackupExcluded {
  rel: string
  reason: string
}

/** `POST /api/backup/export` 的 201 响应。 */
export interface BackupExportResult {
  name: string
  dest: string
  db_sha256: string
  files: number
  total_bytes: number
  excluded: BackupExcluded[]
}

/** `POST /api/backup/verify` 的请求：与 export 同一个相对名。 */
export interface BackupVerifyRequest {
  name: string
}

/** `POST /api/backup/verify` 的 200 响应。`ok` 是字面量 true —— 校验失败走错误状态码。 */
export interface BackupVerifyResult {
  name: string
  ok: true
  files: number
  total_bytes: number
  db_sha256: string
}

/** `GET /api/upgrade/check` 的响应。这几条路由仍是 501，所以字段保持宽松。 */
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

export function exportBackup(name: string): Promise<BackupExportResult> {
  return apiJson<BackupExportResult>(BACKUP_EXPORT_ROUTE, {
    method: 'POST',
    body: JSON.stringify({ name }),
  })
}

export function verifyBackup(name: string): Promise<BackupVerifyResult> {
  return apiJson<BackupVerifyResult>(BACKUP_VERIFY_ROUTE, {
    method: 'POST',
    body: JSON.stringify({ name }),
  })
}

function isText(value: unknown): value is string {
  return typeof value === 'string'
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}

/**
 * 响应自检。
 *
 * 这里必须**逐字段核对**而不是直接把 `data` 当真：备份做完了但摘要字段是空的，
 * 界面上照样会出现「导出完成」—— 那正是这个项目最不能出的错。
 * 读不懂就当没成功，让界面显示读不懂，而不是显示成功。
 */
export function isBackupExportResult(value: unknown): value is BackupExportResult {
  if (!value || typeof value !== 'object') return false
  const r = value as Record<string, unknown>
  if (!isText(r.name) || !isText(r.dest) || !isText(r.db_sha256)) return false
  if (!isCount(r.files) || !isCount(r.total_bytes)) return false
  if (!Array.isArray(r.excluded)) return false
  return r.excluded.every((entry) => {
    if (!entry || typeof entry !== 'object') return false
    const e = entry as Record<string, unknown>
    return isText(e.rel) && isText(e.reason)
  })
}

export function isBackupVerifyResult(value: unknown): value is BackupVerifyResult {
  if (!value || typeof value !== 'object') return false
  const r = value as Record<string, unknown>
  return isText(r.name) && r.ok === true && isCount(r.files) && isCount(r.total_bytes) && isText(r.db_sha256)
}

export type BackupFailure =
  | 'invalid-name'
  | 'dest-not-empty'
  | 'backup-corrupt'
  | 'source-unavailable'
  | 'unknown-backup'

/**
 * 状态码 → 失败种类。每种失败问的是不同的问题，所以界面上要不同的话：
 * 名字不合法（改名字）、目标已存在且非空（换个名字）、备份自身损坏（别用这份）、
 * 备份源不可用（先修环境）、校验时找不到这份备份（名字写错了）。
 *
 * 422 与 409 必须分开：409 在服务端是「换个名字就能成」，
 * 校验失败换个名字不会有任何改变 —— 把 422 也映射成改名，
 * 就是让用户去做一件和真实原因无关的事。
 */
const FAILURE_BY_STATUS: Record<number, BackupFailure> = {
  400: 'invalid-name',
  404: 'unknown-backup',
  409: 'dest-not-empty',
  422: 'backup-corrupt',
  503: 'source-unavailable',
}

export function backupFailure(error: unknown): BackupFailure | null {
  if (!(error instanceof ApiError)) return null
  return FAILURE_BY_STATUS[error.status] ?? null
}

type T = (key: string, options?: Record<string, unknown>) => string

/** 一句「哪里坏了 + 下一步做什么」。服务端原文由 `ErrorNotice` 另外端出来。 */
export function backupFailureLabel(failure: BackupFailure, t: T): string {
  switch (failure) {
    case 'invalid-name':
      return t('workspace.backupErrorInvalidName', {
        defaultValue:
          '备份名不合法：只接受备份根目录下的相对名，绝对路径、盘符和 .. 服务端一律拒（400）。下一步：改成一个纯名字，例如 backup-2026-10-06。',
      })
    case 'dest-not-empty':
      return t('workspace.backupErrorDestNotEmpty', {
        defaultValue:
          '这个备份名已经存在，而且里面不是空的，服务端不会往里写（409）。下一步：换一个没用过的备份名再导出。',
      })
    case 'backup-corrupt':
      return t('workspace.backupErrorCorrupt', {
        defaultValue:
          '这份备份自己坏了（422）：清单解析不了，或者文件内容与清单里的摘要对不上。这不是「换个名字」能解决的。下一步：这份备份不要拿去还原，改用另一份备份；想确认坏在哪，看服务端返回的 detail，它会逐个点名对不上的文件。',
      })
    case 'source-unavailable':
      return t('workspace.backupErrorSourceUnavailable', {
        defaultValue:
          '备份源现在不可用（503）：服务端取不到数据根目录或数据库。这不是「备份完成」。下一步：先在服务端跑 quill doctor 体检，修好之后再回来重试。',
      })
    case 'unknown-backup':
      return t('workspace.backupErrorUnknownBackup', {
        defaultValue:
          '没有找到这份备份（404）：服务端备份根目录下没有这个名字。下一步：核对名字拼写，或者先导出一份再校验。',
      })
  }
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