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

/** `GET /api/extensions/mcp` 的响应；servers 字段缺失时按空数组处理。 */
export interface McpServerList {
  servers?: McpServerConfig[]
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
