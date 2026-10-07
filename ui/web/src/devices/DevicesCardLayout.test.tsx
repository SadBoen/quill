import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { DeviceListPage } from './Devices'
// jsdom 不加载 CSS，算不了布局，所以样式判据直接读源文件。
import overridesCss from '../overrides.css?raw'
import devicesRowCss from './devices-row.css?raw'

/**
 * 卡片与列表这两种看法共用同一批内容组件（`McpFacts` / `McpActions`），
 * 但两边的容器宽度差一个数量级：列表是整幅宽，卡片是 260px 起的一栏。
 * 于是有两种错只有在这个页面上会发生，而它们在 jsdom 里完全看不出来：
 *
 * 1. **文字顶破容器。** flex 子项默认 `min-width: auto`，不肯比内容窄。
 *      工具名（`notes__echo-third`）加上「服务器里的原名：…」在 260px 的卡片
 *      里就是顶破右边界，`.mcp-tool-item code` 的 `word-break: break-all`
 *      因为压根没有可断的宽度而不起作用。
 * 2. **卡片底边不齐。** vendor 的 `.device-card` 是 `display: block`，
 *      按钮跟着内容浮在中间；内容长短不同时三张卡的按钮高低不一。
 *
 * 这两条只能读 CSS 源文件判：断言的是「该有的声明在不在」，
 * 而不是「算出来的盒子是什么」—— 后者 jsdom 给不了。
 */

function servers(...names: string[]): unknown {
  return {
    servers: names.map((name) => ({ name, transport: 'stdio', command: 'x', enabled_capabilities: [] })),
    status: names.map((name) => ({
      name,
      probed: true,
      connected: true,
      tool_count: 1,
      error: null,
      mounted: 1,
      mounted_tools: [[`${name}__a-very-long-mounted-tool-name`, 'a-very-long-remote-tool-name']],
      not_mounted: [],
    })),
    connected: true,
    probed: names.length,
    connected_count: names.length,
    failed_count: 0,
    mounted_count: names.length,
    note: '探测说明。',
  }
}

async function renderPage(names: string[], view?: 'card' | 'list'): Promise<void> {
  await i18n.changeLanguage('zh-CN')
  if (view) localStorage.setItem('quill:devices:view', view)
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation(() =>
      Promise.resolve(
        new Response(JSON.stringify(servers(...names)), {
          status: 200,
          headers: { 'content-type': 'application/json' },
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
  await waitFor(() => expect(screen.getByText(/探测说明/)).toBeInTheDocument())
}

beforeEach(() => {
  localStorage.clear()
})

describe('卡片窄栏里的两处布局错', () => {
  it('工具名的子项允许被压窄，长名字才能断行而不是顶破卡片', () => {
    // 不断言「有没有 min-width」这种写法细节，要断言的是它挡的那件事：
    // 子项得能被压到比内容窄，overflow-wrap 才有意义。
    const rule = /^\.mcp-tool-item > \* \{([^}]*)\}/m.exec(devicesRowCss)
    expect(rule, '.mcp-tool-item 的子项规则不见了').not.toBeNull()
    expect(rule![1], '子项不肯变窄，flex 容器就不会给它可断的宽度').toMatch(/min-width:\s*0/)
  })

  it('卡片是竖向 flex，按钮才能贴到卡底', () => {
    const rule = /^\.device-card \{([^}]*)\}/m.exec(overridesCss)
    expect(rule, '.device-card 的本地覆盖不见了').not.toBeNull()
    expect(rule![1], 'display: block 时 margin-top:auto 不起作用，按钮会浮在内容末尾').toMatch(
      /flex-direction:\s*column/,
    )
    // 贴底靠的是这条，缺了它竖向 flex 也没意义。
    expect(overridesCss).toMatch(/\.device-card \.form-actions \{[^}]*margin-top:\s*auto/)
  })
})

describe('两种看法都能改同一个服务', () => {
  it.each(['card', 'list'] as const)('%s 形态下编辑与删除都在', async (view) => {
    await renderPage(['solo'], view)
    const row = screen.getByTestId(view === 'list' ? 'mcp-row' : 'mcp-card')
    // aria-label 里带服务名，两个按钮都找得到。
    expect(row.querySelector('[aria-label="编辑 solo"]')).not.toBeNull()
    expect(row.querySelector('[aria-label="删除 solo"]')).not.toBeNull()
  })
})
