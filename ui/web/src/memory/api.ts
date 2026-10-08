import { apiJson } from '../api/client'

/** `GET /api/wiki/pages` 的响应：`{ "pages": ["a.md", ...] }`。 */
export interface WikiPageList {
  pages: string[]
}

/** `GET /api/wiki/pages/{path}` 的响应。 */
export interface WikiPage {
  path: string
  content: string
  /**
   * 乐观并发用的版本号（服务端算的是**磁盘原文的 sha256**）。
   * 保存时必须把它原样带回去；不带或带错，服务端会回 409 而不是覆盖。
   */
  version?: string | null
  title?: string
  page_type?: string
  warnings?: string[]
}

/** `PUT /api/wiki/pages/{path}` 的响应。 */
export interface WikiWriteResult {
  path: string
  version: string
  index_entries: number
  created: boolean
}

/** `GET /api/wiki/index` 的响应。 */
export interface WikiIndex {
  index: string | null
  present: boolean
}

/** `GET /api/wiki/log` 的响应。 */
export interface WikiLog {
  log: string | null
  present: boolean
}

export function listPages(): Promise<WikiPageList> {
  return apiJson<WikiPageList>('/api/wiki/pages')
}

export function readPage(path: string): Promise<WikiPage> {
  return apiJson<WikiPage>(`/api/wiki/pages/${pagePath(path)}`)
}

export function readIndex(): Promise<WikiIndex> {
  return apiJson<WikiIndex>('/api/wiki/index')
}

export function readLog(): Promise<WikiLog> {
  return apiJson<WikiLog>('/api/wiki/log')
}

/**
 * 写一页（新建或覆盖）。`expectedVersion` 的两种语义与服务端一一对应：
 * - `null` = 「这一页必须还不存在」（新建）；它已经存在 → 409；
 * - 字符串 = 「我看到的就是这一版」；对不上（含页面已被删）→ 409，
 *   而 409 的 detail 里带着**当前**版本号，用户据此重取再改。
 */
export function writePage(
  path: string,
  content: string,
  expectedVersion: string | null,
): Promise<WikiWriteResult> {
  return apiJson<WikiWriteResult>(`/api/wiki/pages/${pagePath(path)}`, {
    method: 'PUT',
    body: JSON.stringify({ content, expected_version: expectedVersion }),
  })
}

/** 删一页。删除**必须**带版本号（缺它服务端回 400）—— 删除不可逆。 */
export function deletePage(
  path: string,
  expectedVersion: string,
): Promise<{ path: string; deleted: boolean; index_entries: number }> {
  return apiJson(
    `/api/wiki/pages/${pagePath(path)}?expected_version=${encodeURIComponent(expectedVersion)}`,
    { method: 'DELETE' },
  )
}

/** 资料库的写入入口（Q058 已接通）：新建/覆盖 `PUT`，删除 `DELETE`，都带版本号。 */
export const WIKI_WRITE_ROUTE = 'PUT /api/wiki/pages/{path}'

/** 按段编码路径，`/` 保留为分隔符。 */
function pagePath(path: string): string {
  return path
    .split('/')
    .filter((part) => part && part !== '.')
    .map((part) => encodeURIComponent(part))
    .join('/')
}
