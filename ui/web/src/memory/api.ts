import { apiJson } from '../api/client'

/** `GET /api/wiki/pages` 的响应：`{ "pages": ["a.md", ...] }`。 */
export interface WikiPageList {
  pages: string[]
}

/** `GET /api/wiki/pages/{path}` 的响应。 */
export interface WikiPage {
  path: string
  content: string
  title?: string
  page_type?: string
  warnings?: string[]
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

/** 资料库的写入入口：quill 只登记了只读路由，这里统一给出下一步该接哪个。 */
export const WIKI_WRITE_ROUTE = 'PUT /api/wiki/pages/{path}'

/** 按段编码路径，`/` 保留为分隔符。 */
function pagePath(path: string): string {
  return path
    .split('/')
    .filter((part) => part && part !== '.')
    .map((part) => encodeURIComponent(part))
    .join('/')
}
