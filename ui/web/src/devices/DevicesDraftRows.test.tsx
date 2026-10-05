import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { mcpRowState, type McpServerConfig } from './api'
import { DeviceListPage } from './Devices'

/**
 * ISSUE-010：草稿曾经与已保存配置渲染成**一模一样**的卡片。
 *
 * 实测复现（浏览器真点）：填完表单点「加入草稿」，草稿立刻以同样的图标、
 * 同样的传输方式、同样的「编辑 / 删除」出现在已保存列表里，
 * 而 `curl /api/extensions/mcp` 里**根本没有这一行**。
 * 用户以为存好了就关掉标签页 —— 这条配置就永远丢了。
 *
 * 另外：对**未保存**的草稿点「删除」，抓网络面板会发现**一个请求都没发**，
 * 只是本地移除。所以草稿行上的按钮文案写「删除 X」本身就是骗人的。
 *
 * 这组盯住两件事：
 *   1. 界面渲染的卡片数 **==** 服务端返回的条数，草稿单独计数，不许混算；
 *   2. 没被动过的已保存行 **不许** 被误判成「有改动」——
 *      否则一打开页面就满屏「未保存」，那和没标一样糟。
 */

const SAVED: McpServerConfig[] = [
  { name: 'notes', transport: 'stdio', command: 'quill-mcp-stub', enabled_capabilities: [] },
  { name: 'filesystem', transport: 'stdio', command: 'mcp-server-git', enabled_capabilities: [] },
]

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } })
}

async function renderPage(): Promise<void> {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(() =>
      Promise.resolve(
        jsonResponse({
          servers: SAVED,
          status: [],
          connected: false,
          probed: 0,
          connected_count: 0,
          failed_count: 0,
          mounted_count: 0,
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
  await waitFor(() => expect(screen.getAllByTestId('mcp-card')).toHaveLength(SAVED.length))
}

/** 填一张新的 streamable_http 草稿卡并提交。 */
function submitDraft(name: string, url: string): void {
  fireEvent.change(screen.getByPlaceholderText('company-search'), { target: { value: name } })
  fireEvent.change(screen.getByPlaceholderText('https://mcp.example.com/mcp'), { target: { value: url } })
  fireEvent.click(screen.getByRole('button', { name: /加入草稿/ }))
}

function cards(): HTMLElement[] {
  return screen.queryAllByTestId('mcp-card')
}

function cardOf(name: string): HTMLElement {
  const hit = cards().find((el) => within(el).queryByRole('heading', { name: new RegExp(`^${name}`) }))
  if (!hit) throw new Error(`没有名为 ${name} 的卡片：${cards().length} 张`)
  return hit
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

describe('ISSUE-010：草稿行不许伪装成已保存的行', () => {
  it('没有草稿时，渲染出的卡片数就等于服务端返回的条数，且一个「未保存」标记都没有', async () => {
    await renderPage()

    expect(cards()).toHaveLength(SAVED.length)
    expect(screen.queryAllByTestId('mcp-unsaved-badge')).toHaveLength(0)
    expect(screen.queryByTestId('mcp-unsaved-banner')).not.toBeInTheDocument()
    // 每一行都必须明确落在 `saved` 上，而不是「没有 data-row-state」。
    for (const card of cards()) {
      expect(card.getAttribute('data-row-state')).toBe('saved')
    }
  })

  it('加入草稿后：服务端那条不动它就仍是 saved，只有新行挂「未保存（新增）」', async () => {
    await renderPage()
    submitDraft('browser-e2e-probe', 'https://mcp.example.com/probe')

    await waitFor(() => expect(cards()).toHaveLength(SAVED.length + 1))
    expect(cardOf('notes').getAttribute('data-row-state')).toBe('saved')
    expect(cardOf('filesystem').getAttribute('data-row-state')).toBe('saved')
    expect(cardOf('browser-e2e-probe').getAttribute('data-row-state')).toBe('new')

    // 未保存标记必须**长在那一行自己身上**，不能只写在页面顶部。
    const badges = within(cardOf('browser-e2e-probe')).getAllByTestId('mcp-unsaved-badge')
    expect(badges).toHaveLength(1)
    expect(badges[0].textContent).toContain('未保存')
    expect(within(cardOf('notes')).queryByTestId('mcp-unsaved-badge')).toBeNull()
  })

  it('草稿行的按钮说「丢弃草稿」，不许沿用已保存行的「删除 X」', async () => {
    await renderPage()
    submitDraft('browser-e2e-probe', 'https://mcp.example.com/probe')
    await waitFor(() => expect(cards()).toHaveLength(SAVED.length + 1))

    // 已保存行仍然是「删除 notes」…
    expect(cardOf('notes').getAttribute('data-row-state')).toBe('saved')
    expect(within(cardOf('notes')).getByRole('button', { name: '删除 notes' })).toBeInTheDocument()
    // …而服务端根本没有的那一行，必须说「丢弃草稿 browser-e2e-probe」。
    expect(
      within(cardOf('browser-e2e-probe')).getByRole('button', { name: '丢弃草稿 browser-e2e-probe' }),
    ).toBeInTheDocument()
  })

  it('顶部横幅把「关掉标签页就丢了」和下一步说清楚', async () => {
    await renderPage()
    expect(screen.queryByTestId('mcp-unsaved-banner')).not.toBeInTheDocument()

    submitDraft('browser-e2e-probe', 'https://mcp.example.com/probe')
    const banner = await screen.findByTestId('mcp-unsaved-banner')
    expect(banner.textContent).toContain('1 条改动还没保存')
    expect(banner.textContent).toContain('下一步')
  })

  it('改一条已存在的配置时标成「有改动」，且说清删除要保存后才生效', async () => {
    await renderPage()
    fireEvent.click(within(cardOf('notes')).getByRole('button', { name: /编辑/ }))
    await waitFor(() => expect(screen.getByPlaceholderText('company-search')).toHaveValue('notes'))

    // 按**值**定位而不是 placeholder —— `command` 的 placeholder 是字面量
    // 「python」，真正区分它的是从已保存配置回填进来的值。
    fireEvent.change(screen.getByDisplayValue('quill-mcp-stub'), { target: { value: '另一个可执行文件' } })
    // 精确匹配「保存」：顶栏那个是「保存 MCP 配置」，用 /保存/ 会同时命中两个按钮。
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    await waitFor(() => expect(cardOf('notes').getAttribute('data-row-state')).toBe('changed'))
    expect(within(cardOf('notes')).getByTestId('mcp-unsaved-badge').textContent).toContain('有改动')
    expect(within(cardOf('notes')).getByRole('button', { name: /删除 notes（保存后才生效）/ })).toBeInTheDocument()
    // 没动过的那一行不许跟着变成 changed。
    expect(cardOf('filesystem').getAttribute('data-row-state')).toBe('saved')
  })
})

describe('mcpRowState：判定只认「服务端有没有这一行、内容变没变」', () => {
  const row: McpServerConfig = { name: 'a', transport: 'stdio', command: 'x', enabled_capabilities: [] }

  it('没有草稿时恒为 saved —— 避免一打开页面就满屏「未保存」', () => {
    expect(mcpRowState({ ...row, command: 'y' }, [{ ...row, command: 'z' }], false)).toBe('saved')
    expect(mcpRowState(row, [], false)).toBe('saved')
  })

  it('服务端没有同名行 → new；内容不同 → changed；一模一样 → saved', () => {
    expect(mcpRowState(row, [], true)).toBe('new')
    expect(mcpRowState(row, [{ ...row, command: 'y' }], true)).toBe('changed')
    expect(mcpRowState(row, [{ ...row }], true)).toBe('saved')
  })

  it('可选字段的 undefined / null / 空串是同一件事，不能被判成「用户改过了」', () => {
    expect(
      mcpRowState({ ...row, cwd: undefined }, [{ ...row, cwd: null }], true),
    ).toBe('saved')
    expect(mcpRowState({ ...row, cwd: '' }, [{ ...row, cwd: undefined }], true)).toBe('saved')
  })

  it('env 的键顺序不影响判定', () => {
    expect(
      mcpRowState({ ...row, env: { A: '1', B: '2' } }, [{ ...row, env: { B: '2', A: '1' } }], true),
    ).toBe('saved')
  })

  it('真的改了 env 的值就是 changed', () => {
    expect(
      mcpRowState({ ...row, env: { A: '1' } }, [{ ...row, env: { A: '2' } }], true),
    ).toBe('changed')
  })
})
