import { apiJson } from '../api/client'

/**
 * `builtin` = **内置服务器**：随 quill 自带、跑在同一个进程里（内存管道），
 * 不起子进程也不用装东西。它的**名字**写在 `command` 里（目前只有 `memory`），
 * 名字清单来自服务端响应的 `builtin_servers` —— 别在前端硬编码一份。
 */
export type McpTransport = 'stdio' | 'streamable_http' | 'sse' | 'builtin'

/** MCP 服务端的最小可编辑形状（quill 未提供 openapi 类型，模块内自带）。 */
export interface McpServerConfig {
  name: string
  transport: McpTransport
  enabled_capabilities: string[] | null
  command?: string
  args?: string[]
  cwd?: string | null
  env?: Record<string, string>
  url?: string
  headers?: Record<string, string>
  max_concurrent_calls?: number
}

/**
 * 一台 MCP 服务器在**本轮真实探测**里的状态。
 *
 * 全部字段都来自服务端真的发起过 `initialize` + `tools/list` 之后的结果，
 * 没有任何一个是前端推断的。对端是 `mcp_client::probe`。
 */
export interface McpServerStatus {
  name: string
  /** 本轮是不是真的发起过协议握手。停用 / 传输方式没铺 / 缺 command 都会是 false。 */
  probed: boolean
  connected: boolean
  /** 按 `enabled_capabilities` 过滤之后真正会交给模型的工具条数。 */
  tool_count: number
  /** 探测不到时的白话原因，原样显示。 */
  error?: string | null
  protocol_version?: string | null
  server_info?: string | null
  /**
   * 这一轮**真的挂进对话工具表**的工具条数。
   *
   * 与 `tool_count` 是两件事：`tool_count` 是服务器自己报了几个（`tools/list`），
   * `mounted` 是其中有几个模型这一轮调得到。两者不相等时，原因在
   * `not_mounted` 里逐条写着。
   *
   * 少这一个字段，界面上就只能显示「3 个工具可用」，而模型那轮一个都调不到 ——
   * 没有报错、没有红字，刷新一次还是那样。
   */
  mounted?: number
  /**
   * 已挂载的工具：`[挂载名, 服务器自己报的原名]`。
   *
   * 两个名字都要显示：用户在自己的 MCP 配置里认的是原名，而模型调的是挂载名。
   * 只给其中一个，用户就没法把界面上这一行对回自己写的那份配置。
   */
  mounted_tools?: [string, string][]
  /** 没挂上的工具：`[原名, 白话原因]`。为空数组表示该挂的都挂了。 */
  not_mounted?: [string, string][]
}

/** `GET /api/extensions/mcp` 的响应；servers 字段缺失时按空数组处理。 */
export interface McpServerList {
  servers?: McpServerConfig[]
  /**
   * 内置服务器的名字清单（服务端给的，目前只有 `memory`）。
   *
   * 表单里 `transport=builtin` 那一栏的可选项就来自这里 —— 硬编码一份
   * 前端自己的名单，服务端加了内置服务器之后这边会安静地少一个选项。
   */
  builtin_servers?: string[]
  /**
   * 本轮探测过的服务器是不是**全部**连上了。
   *
   * 一台都没探测过时是 `false` —— 那不是「都通了」，那是「没查过」。
   * 靠 `probed` 把这两种情况分开说，别拿这个字段当「已连接」显示。
   */
  connected?: boolean
  /** 本轮真的发起过握手的台数。 */
  probed?: number
  connected_count?: number
  failed_count?: number
  /**
   * 本轮**真的挂进对话工具表**的工具总数（所有服务器加起来）。
   *
   * 与 `connected_count` 分开报：连上了不等于模型调得到。
   */
  mounted_count?: number
  /** 每台服务器的实测状态，与 `servers` 同序。 */
  status?: McpServerStatus[]
  /** 服务端对当前连通性状态的说明，原样显示，不改写。 */
  note?: string
}

export const MCP_ROUTE = '/api/extensions/mcp'

export function listMcpServers(): Promise<McpServerList> {
  return apiJson<McpServerList>(MCP_ROUTE)
}

/** 已登记但尚未实现，写入入口统一指向这个方法。 */
export function saveMcpServers(servers: McpServerConfig[]): Promise<McpServerList> {
  return apiJson<McpServerList>(MCP_ROUTE, { method: 'POST', body: JSON.stringify({ servers }) })
}

export function mcpServersOf(body: McpServerList | undefined): McpServerConfig[] {
  return body?.servers ?? []
}

/**
 * 一行在**渲染那一刻**到底存没存。
 *
 * - `saved`   服务端里有，且内容一模一样。
 * - `changed` 服务端里有同名行，但内容被草稿改过（**保存后才会生效**）。
 * - `new`     服务端里根本没有这一行（**只存在于本地草稿**）。
 *
 * ISSUE-010：草稿曾经与已保存行渲染成**一模一样**的卡片，用户以为存好了
 * 就关掉标签页，这条配置就永远丢了 —— 界面上没有一句在说这行还没落库。
 *
 * **只在有草稿时才用**：`hasDraft === false` 时所有行都必然是 `saved`，
 * 这就把「服务端归一化过的字段与表单字段长得不一样」造成的误判，
 * 限制在「用户手上正有未保存改动」这个窗口内。
 */
export type McpRowState = 'saved' | 'changed' | 'new'

export function mcpRowState(
  row: McpServerConfig,
  saved: McpServerConfig[],
  hasDraft: boolean,
): McpRowState {
  if (!hasDraft) return 'saved'
  const same = saved.find((item) => item.name === row.name)
  if (!same) return 'new'
  return normalizeRow(same) === normalizeRow(row) ? 'saved' : 'changed'
}

/**
 * 把一行压成一个可比较的字符串。
 *
 * `undefined` / `null` / `''` 在可选字段上**语义相同**（没填），
 * 归到一起，否则刚加载就会把没动过的行误判成 `changed`。
 * `env` / `headers` 按键排序，字段顺序不该被当成「用户改过了」。
 */
function normalizeRow(row: McpServerConfig): string {
  return JSON.stringify([
    row.name,
    row.transport,
    row.enabled_capabilities ?? null,
    row.command ?? '',
    row.args ?? [],
    row.cwd ?? '',
    sortedPairs(row.env),
    row.url ?? '',
    sortedPairs(row.headers),
    row.max_concurrent_calls ?? '',
  ])
}

function sortedPairs(value: Record<string, string> | undefined): string {
  const entries = Object.entries(value ?? {}).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
  return JSON.stringify(entries)
}

/** 按名字取某台服务器的实测状态。没有记录时返回 `undefined`——**不是**「已连接」。 */
export function mcpStatusOf(
  body: McpServerList | undefined,
  name: string,
): McpServerStatus | undefined {
  return body?.status?.find((item) => item.name === name)
}
