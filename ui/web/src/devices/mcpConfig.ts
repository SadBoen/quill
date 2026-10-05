import type { McpServerConfig } from './api'

export type EditableMcpServer = McpServerConfig

/**
 * MCP 服务名的形状，**必须**与服务端 `mcp_repo::normalize_name` 逐条对齐。
 *
 * 这条约束是从一次真事故里长出来的：原来这里写的是
 * `[a-z][a-z0-9_]{0,31}`，只允许下划线；服务端却把 `_` 归一成 `-` 并返回
 * `company-search`。于是「保存 → 重新编辑」这一圈直接死掉 —— 编辑框里回填的
 * `company-search` 过不了页面自己的 `pattern`，浏览器拦下提交，用户只看到一个
 * 说不清来由的红框。旧写法还要求首字符是字母，而服务端允许数字开头。
 *
 * 规则（对应服务端 `normalize_name` + `mcp_servers` 的 CHECK）：
 * 归一后全小写；首尾是字母或数字；中间可含连字符但不得连续；长度 1~64。
 */
export const MCP_NAME_PATTERN = '[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?'

/** 与 `MCP_NAME_PATTERN` 等价的可执行版本，用于单测。 */
export const MCP_NAME_RE = new RegExp(`^${MCP_NAME_PATTERN}$`)

function lines(value: FormDataEntryValue | null): string[] {
  return String(value ?? '')
    .split('\n')
    .map((item) => item.trim())
    .filter(Boolean)
}

function parseKeyValues(value: FormDataEntryValue | null): Record<string, string> {
  return Object.fromEntries(lines(value).map((line) => {
    const separator = line.indexOf('=')
    return separator < 0 ? [line, ''] : [line.slice(0, separator).trim(), line.slice(separator + 1)]
  }))
}

export function formatMcpSecrets(value: Record<string, string> | undefined): string {
  return Object.entries(value ?? {}).map(([key, item]) => `${key}=${item}`).join('\n')
}

export function mcpCapabilityMode(value: string[] | null | undefined): 'all' | 'none' | 'exact' {
  if (value == null) return 'none'
  return value.length === 0 ? 'all' : 'exact'
}

export function mcpSecretMap(server: EditableMcpServer | null): Record<string, string> {
  if (!server) return {}
  return server.transport === 'stdio' ? server.env ?? {} : server.headers ?? {}
}

function sameSink(left: EditableMcpServer, right: EditableMcpServer): boolean {
  if (left.name !== right.name || left.transport !== right.transport) return false
  if (left.transport === 'stdio' && right.transport === 'stdio') {
    return left.command === right.command
      && JSON.stringify(left.args ?? []) === JSON.stringify(right.args ?? [])
      && (left.cwd ?? null) === (right.cwd ?? null)
  }
  return left.transport !== 'stdio' && right.transport !== 'stdio' && left.url === right.url
}

function readEnabledCapabilities(data: FormData): string[] | null | 'empty' {
  const mode = String(data.get('capability_mode'))
  if (mode === 'none') return null
  if (mode === 'all') return []
  const values = data.getAll('enabled_capabilities')
    .flatMap((value) => String(value).split(','))
    .map((value) => value.trim())
    .filter(Boolean)
  return values.length ? [...new Set(values)] : 'empty'
}

export function readMcpForm(
  data: FormData,
  existing: EditableMcpServer | null,
  serverSide: boolean,
  translate: (key: string, options?: Record<string, unknown>) => string,
): EditableMcpServer | string {
  const enabled = readEnabledCapabilities(data)
  if (enabled === 'empty') return translate('mcp.exactRequired', { defaultValue: '请至少勾选一个能力，或改用「全部启用」。' })
  const transport = String(data.get('transport')) as EditableMcpServer['transport']
  const common = {
    name: String(data.get('name')),
    enabled_capabilities: enabled,
    ...(serverSide ? { max_concurrent_calls: Number(data.get('max_concurrent_calls')) } : {}),
  }
  const secrets = parseKeyValues(data.get('secrets'))
  let candidate: EditableMcpServer
  if (transport === 'stdio') {
    candidate = {
      ...common,
      transport,
      command: String(data.get('command')),
      args: lines(data.get('args')),
      cwd: String(data.get('cwd') ?? '').trim() || null,
      env: secrets,
    }
  } else {
    candidate = {
      ...common,
      transport,
      url: String(data.get('url')),
      headers: secrets,
    }
  }

  if (existing && Object.keys(mcpSecretMap(existing)).length) {
    if (!sameSink(existing, candidate)) {
      if (!Object.keys(secrets).length || Object.values(secrets).some((value) => value === '<redacted>' || value === '')) {
        return translate('mcp.reenterSecrets', { defaultValue: '换了连接地址，请重新输入密钥。' })
      }
    } else if (!data.get('editing')) {
      const merged = { ...mcpSecretMap(existing), ...secrets }
      candidate = candidate.transport === 'stdio'
        ? { ...candidate, env: merged }
        : { ...candidate, headers: merged }
    }
  }
  return candidate
}
