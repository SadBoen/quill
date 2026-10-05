import { apiJson } from '../api/client'

export type McpTransport = 'stdio' | 'streamable_http' | 'sse'

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
  /** 本轮是不是真的发起过协议握手。停用 / 非 stdio / 缺 command 都会是 false。 */
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

/** 按名字取某台服务器的实测状态。没有记录时返回 `undefined`——**不是**「已连接」。 */
export function mcpStatusOf(
  body: McpServerList | undefined,
  name: string,
): McpServerStatus | undefined {
  return body?.status?.find((item) => item.name === name)
}
