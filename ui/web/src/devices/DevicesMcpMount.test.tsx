import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { DeviceListPage } from './Devices'

/**
 * 这组盯的是**界面上说的话与后端给的字段一致**。
 *
 * 背景：`GET /api/extensions/mcp` 里的 `tool_count`（服务器报了几个）与
 * `mounted`（真的挂进对话工具表的有几个）是两个字段，而且会不相等 ——
 * 能力被关掉、挂载名与已有工具撞上、schema 不是 object，三种都会让挂载数变少。
 *
 * 组件里**不许**由 `tool_count` 推断挂载状态：那样界面会显示「3 个工具可用」，
 * 而模型那一轮一个都调不到，没有报错、没有红字，刷新一次还是那样。
 * 下面每一条都把两种不相等的情况钉住。
 */

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } })
}

function statusOf(over: Record<string, unknown>): Record<string, unknown> {
  return {
    name: 'notes',
    probed: true,
    connected: true,
    tool_count: 3,
    error: null,
    protocol_version: '2025-06-18',
    server_info: 'quill-test-stub 1.0.0',
    mounted: 3,
    mounted_tools: [
      ['notes__read-note', 'read-note'],
      ['notes__list-notes', 'list-notes'],
      ['notes__echo-third', 'echo-third'],
    ],
    not_mounted: [],
    ...over,
  }
}

async function renderPage(status: Record<string, unknown>): Promise<void> {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(() =>
      Promise.resolve(
        jsonResponse({
          servers: [{ name: 'notes', transport: 'stdio', command: 'x', enabled_capabilities: [] }],
          status: [status],
          connected: true,
          probed: 1,
          connected_count: 1,
          failed_count: 0,
          mounted_count: status.mounted,
          note: '服务端给的话。',
        }),
      ),
    ),
  )
  render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <DeviceListPage />
      </QueryClientProvider>
    </MemoryRouter>,
  )
  // `note` 渲染在与其它文字**同一个** `<p>` 里，所以只能按子串找 ——
  // 按整段精确匹配会失败，而那种失败与「note 根本没渲染」长得一样。
  await waitFor(() => expect(screen.getByText(/服务端给的话/)).toBeInTheDocument())
}

describe('MCP 挂载状态必须与后端给的字段一致', () => {
  it('全挂上时：显示挂了几个，并把挂载名与原名都列出来', async () => {
    await i18n.changeLanguage('zh-CN')
    await renderPage(statusOf({}))

    expect(screen.getByText(/真的挂进对话工具表的有 3 个/)).toBeInTheDocument()
    // 挂载名与原名都要看得见：用户认的是原名，模型调的是挂载名。
    expect(screen.getByText('notes__read-note')).toBeInTheDocument()
    expect(screen.getByText(/原名：read-note/)).toBeInTheDocument()
  })

  it('服务器报了 3 个但只挂上 1 个时：显示 1，且逐条说清没挂上的原因', async () => {
    await i18n.changeLanguage('zh-CN')
    // 这是最容易骗人的一种状态：tool_count=3 看上去一切正常，
    // 而模型那一轮只调得到 1 个。
    await renderPage(
      statusOf({
        mounted: 1,
        mounted_tools: [['notes__read-note', 'read-note']],
        not_mounted: [
          ['list-notes', '挂载名与已有工具相同，挂进去会顶掉它'],
          ['echo-third', '服务器给的 inputSchema 不是 object 类型，模型无从得知该传什么参数'],
        ],
      }),
    )

    expect(screen.getByText(/真的挂进对话工具表的有 1 个/)).toBeInTheDocument()
    // 断言语义，而不是断言某句固定文案 —— 文案可以改，语义不能。
    expect(screen.queryByText(/真的挂进对话工具表的有 3 个/)).not.toBeInTheDocument()
    expect(screen.getByText(/这些工具没挂上/)).toBeInTheDocument()
    expect(screen.getByText('list-notes')).toBeInTheDocument()
    expect(screen.getByText(/会顶掉它/)).toBeInTheDocument()
    expect(screen.getByText(/inputSchema 不是 object/)).toBeInTheDocument()
  })

  it('一个都没挂上时：显示 0 并说明「模型调不到」，不许显示成「3 个工具可用」', async () => {
    await i18n.changeLanguage('zh-CN')
    await renderPage(
      statusOf({ mounted: 0, mounted_tools: [], not_mounted: [['read-note', '服务器报了 0 个工具']] }),
    )
    expect(screen.getByText(/真的挂进对话工具表的有 0 个/)).toBeInTheDocument()
    expect(screen.getByText(/这些工具没挂上/)).toBeInTheDocument()
  })

  it('后端没给 mounted 字段时按 0 报，不许拿 tool_count 顶替', async () => {
    await i18n.changeLanguage('zh-CN')
    // 老服务端（字段还不存在）时的表现。拿 3 来顶替就是凭空多说三个工具。
    await renderPage({ ...statusOf({}), mounted: undefined, mounted_tools: undefined })
    expect(screen.getByText(/真的挂进对话工具表的有 0 个/)).toBeInTheDocument()
  })

  it('没连上时不显示挂载数 —— 「连不上」与「0 个工具」是两回事', async () => {
    await i18n.changeLanguage('zh-CN')
    await renderPage(
      statusOf({ connected: false, probed: true, error: '拉起失败。下一步：先装依赖。' }),
    )
    expect(screen.getByText(/没连上/)).toBeInTheDocument()
    expect(screen.getByText(/拉起失败/)).toBeInTheDocument()
    expect(screen.queryByText(/真的挂进对话工具表的有/)).not.toBeInTheDocument()
  })

  it('连上了但一个工具都没有时，原因必须显示出来', async () => {
    await i18n.changeLanguage('zh-CN')
    // ISSUE-014：本地开关或服务器自报把工具关掉。这不是连接失败，
    // 所以「没连上」那个分支走不到 —— 少了这里，界面上只剩一句
    // 「已连上，0 个工具」，用户不知道该改配置还是该换服务器。
    await renderPage(
      statusOf({
        mounted: 0,
        mounted_tools: [],
        not_mounted: [],
        tool_count: 0,
        error: '服务器在 initialize 里自报的能力里没有 tools。下一步：确认这台服务器是否该提供工具。',
      }),
    )
    expect(screen.getByText(/已连上/)).toBeInTheDocument()
    expect(screen.getByText(/自报的能力里没有 tools/)).toBeInTheDocument()
    expect(screen.getByText(/下一步/)).toBeInTheDocument()
  })
})
