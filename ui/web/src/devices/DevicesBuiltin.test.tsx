import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import type { McpServerConfig } from './api'
import { DeviceListPage } from './Devices'

/**
 * 内置服务器（`transport=builtin`）在设备页上的出口。
 *
 * 背景：Q111 把 `transport='builtin'` 从「声明了做不到」补成了真实现
 * （`quill_core::builtin`，记忆服务器跑在内存管道上），但前端 `McpTransport`
 * 里根本没有这个值、表单也没有这一项 —— 用户从界面上**加不出来**。
 * 「后端做完了而开关没有出口，功能就等于没做」。
 *
 * 这组钉三件事：
 *   1. 传输方式下拉里有「内置」，且选中之后出现的是**内置服务器**那一栏（名字），
 *      不是地址、不是可执行文件；
 *   2. 可选项来自服务端的 `builtin_servers`，前端不自己维护一份会漂的名单；
 *   3. 提交出来的草稿是 `{transport:'builtin', command:'memory'}`，卡片上显示
 *      「内置」字样与那个名字 —— 没有 env / headers 那一栏可填。
 */

const SAVED: McpServerConfig[] = [
  { name: 'notes', transport: 'stdio', command: 'quill-mcp-stub', enabled_capabilities: [] },
]

/** 服务端会给的响应：一台 stdio + 内置名字清单。 */
function listBody(): unknown {
  return {
    servers: SAVED,
    builtin_servers: ['memory'],
    status: [
      {
        name: 'notes',
        probed: true,
        connected: true,
        tool_count: 0,
        error: null,
        mounted: 0,
        mounted_tools: [],
        not_mounted: [],
      },
    ],
    connected: true,
    probed: 1,
    connected_count: 1,
    failed_count: 0,
    mounted_count: 0,
    note: '这一轮真的探测过。',
  }
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } })
}

async function renderPage(): Promise<void> {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(() => Promise.resolve(jsonResponse(listBody()))),
  )
  render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <DeviceListPage />
      </QueryClientProvider>
    </MemoryRouter>,
  )
  await waitFor(() => expect(screen.getByTestId('mcp-card')).toBeInTheDocument())
}

/** 选中「内置」这一传输方式，返回表单里那一栏的 select。 */
async function pickBuiltin(): Promise<HTMLSelectElement> {
  fireEvent.change(screen.getByRole('combobox', { name: /传输方式/ }), {
    target: { value: 'builtin' },
  })
  return await screen.findByRole('combobox', { name: /内置服务器/ }) as HTMLSelectElement
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

describe('设备页能加一台内置服务器（memory）', () => {
  it('传输方式里可选「内置」，选完出现的是内置服务器那一栏（名字来自服务端清单）', async () => {
    await renderPage()
    const names = await pickBuiltin()

    // 可选项来自服务端的 `builtin_servers`，不是前端硬编码。
    const options = Array.from(names.options).map((option) => option.value)
    expect(options).toEqual(['memory'])
    expect(names.value).toBe('memory')

    // 「地址」那一栏必须消失：内置服务器没有地址。
    expect(screen.queryByPlaceholderText('https://mcp.example.com/mcp')).not.toBeInTheDocument()
    // 环境变量 / 请求头那一栏也不该在 —— 内置那台一个都不收。
    expect(screen.queryByPlaceholderText(/KEY=VALUE/)).not.toBeInTheDocument()
  })

  it('提交出来的草稿是内置那一台：卡片显示「内置」与名字，且状态是「未保存（新增）」', async () => {
    await renderPage()
    await pickBuiltin()
    fireEvent.change(screen.getByPlaceholderText('company-search'), { target: { value: 'mem' } })
    fireEvent.click(screen.getByRole('button', { name: /加入草稿/ }))

    const card = await waitFor(() => {
      const hit = screen.getAllByTestId('mcp-card').find((el) =>
        within(el).queryByRole('heading', { name: /^mem/ }),
      )
      if (!hit) throw new Error('草稿卡片还没出现')
      return hit
    })
    expect(card.getAttribute('data-row-state')).toBe('new')
    expect(card.textContent).toContain('内置')
    // 名字就是「哪一台」：地址那一格显示的是 `command`（memory）。
    expect(within(card).getByText('memory')).toBeInTheDocument()
    expect(within(card).getByTestId('mcp-unsaved-badge').textContent).toContain('新增')
  })
})
