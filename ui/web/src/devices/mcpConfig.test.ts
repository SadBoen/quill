import { describe, expect, it } from 'vitest'

import { MCP_NAME_PATTERN, MCP_NAME_RE, mcpCapabilityMode, mcpSecretMap, readMcpForm } from './mcpConfig'

/**
 * 这组表子与 `crates/quill-server/src/mcp_repo.rs` 的
 * `every_normalized_name_passes_the_frontend_pattern` 钉的是**同一个方向**：
 * 服务端归一后**会返回给前端**的名字，必须全部通过前端的 `pattern`。
 *
 * 方向不能反过来要求。「前端拒的都必须被服务端拒」是不成立的：前端比服务端
 * 严一点没问题（那是提前拦下，用户看到的是 pattern 提示）；反过来才会出事。
 */
const SERVER_OUTPUTS: string[] = [
  'a', // 单字符
  'ab', // 两字符
  'filesystem',
  'company-search', // `_` 被归一成 `-` 之后的样子
  'my-tools',
  'a-b-c', // 来自 `a_b_c`
  '1tool', // 服务端允许数字开头
  'a'.repeat(64), // 长度上限
]

describe('MCP 服务名的形状与服务端对齐', () => {
  it('服务端会返回的名字，页面自己必须收得下', () => {
    for (const name of SERVER_OUTPUTS) {
      expect(MCP_NAME_RE.test(name), `服务端会返回 ${name}，页面不该拒`).toBe(true)
    }
  })

  it('空、大写、连字符在首尾、超长都收不下', () => {
    // 这些前端提前拒掉。服务端对其中几条其实是收的（见下面「刻意严于服务端」），
    // 提前拦下对用户更好。
    for (const name of ['', 'A', 'aB', '-ab', 'ab-', 'a b c', 'a'.repeat(65)]) {
      expect(MCP_NAME_RE.test(name), `${JSON.stringify(name)} 不该通过`).toBe(false)
    }
  })

  it('刻意严于服务端：下划线被提前拦下，不让用户填了却被静默改写', () => {
    // 服务端会把 `company_search` 归一成 `company-search`。如果表单放行，
    // 用户填的名字和存下来的不一样却没人告诉他。
    expect(MCP_NAME_RE.test('company_search')).toBe(false)
    expect(MCP_NAME_RE.test('a_b_c')).toBe(false)
  })

  it('刻意宽于服务端：连续连字符交给服务端去拒', () => {
    // 服务端 `normalize_name` 明确拒绝 `--`（归一后仍是连续连字符）。
    // 前端放行不构成数据错误，只是把拒绝推迟到提交那一刻 —— 而那条错误信息
    // 比浏览器的 pattern 提示更能说明问题，所以这里不重复实现这条规则。
    expect(MCP_NAME_RE.test('a--b')).toBe(true)
  })

  it('pattern 字符串与可执行版本是同一份东西', () => {
    // 两份一旦漂移，测试就会绿着骗人：input 上挂的是 pattern 字符串，
    // 而断言用的是 MCP_NAME_RE。
    expect(MCP_NAME_RE.source).toBe(`^${MCP_NAME_PATTERN}$`)
  })
})

describe('能力开关三态', () => {
  it('null 是全禁、空数组是全开、其余是按名单', () => {
    // 三态在服务端与数据库里是分开的，UI 不能把它们压成两态。
    expect(mcpCapabilityMode(null)).toBe('none')
    expect(mcpCapabilityMode([])).toBe('all')
    expect(mcpCapabilityMode(['read'])).toBe('exact')
    expect(mcpCapabilityMode(undefined)).toBe('none')
  })
})

describe('内置服务器（transport=builtin）', () => {
  const t = (key: string, options?: Record<string, unknown>): string =>
    String(options?.defaultValue ?? key)

  function formBody(fields: Record<string, string>): FormData {
    const data = new FormData()
    data.set('capability_mode', 'all')
    for (const [key, value] of Object.entries(fields)) data.set(key, value)
    return data
  }

  it('提交出来的是 command=名字，没有 url / headers / env', () => {
    // 内置服务器就在本进程里跑：地址与环境变量都不存在。混进任何一项，
    // 服务端的交叉校验会 400（「url 不该出现在 builtin 配置上」），
    // 而用户只是从下拉框里选了个「内置」。
    const out = readMcpForm(
      formBody({ name: 'mem', transport: 'builtin', command: 'memory' }),
      null,
      false,
      t,
    )
    if (typeof out === 'string') throw new Error(`表单应当通过，实际被拒：${out}`)
    expect(out).toEqual({
      name: 'mem',
      transport: 'builtin',
      command: 'memory',
      enabled_capabilities: [],
    })
    expect(out).not.toHaveProperty('url')
    expect(out).not.toHaveProperty('headers')
    expect(out).not.toHaveProperty('env')
  })

  it('内置行没有密钥可迁移 —— mcpSecretMap 不许把 env 或 headers 搬过来', () => {
    // 两种传输的「机密字段」是两列（env / headers）。内置那台一个都没有，
    // 借用任何一种都会在保存时把用户的密钥写进不该写的地方。
    expect(mcpSecretMap({ name: 'm', transport: 'builtin', enabled_capabilities: [], command: 'memory' })).toEqual({})
    expect(mcpSecretMap({ name: 'm', transport: 'stdio', enabled_capabilities: [], env: { A: '1' } })).toEqual({ A: '1' })
    expect(mcpSecretMap({ name: 'm', transport: 'sse', enabled_capabilities: [], headers: { B: '2' } })).toEqual({ B: '2' })
  })
})
