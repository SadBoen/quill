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
