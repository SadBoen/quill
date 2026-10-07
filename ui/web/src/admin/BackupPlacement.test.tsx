import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { AdminTabs } from './AdminTabs'
import { AdminInstancePage } from './Admin'
import { WorkspacePage } from '../workspace/WorkspacePage'
// ?raw 把源文件当字符串读进来（用来查「AdminTabs 有几份定义」）。
import adminSrc from './Admin.tsx?raw'
import backupSrc from './AdminBackup.tsx?raw'
import tabsSrc from './AdminTabs.tsx?raw'

/**
 * 备份与升级**在哪儿**。
 *
 * 2026-10-07 之前它们在「工作区」页里，理由是「工作区自己的功能是空的，
 * 拿真实存在的备份路由顶替」。代价是用户以为备份属于工作区。
 * 参照 octop：它的备份归 `/admin/backend`、升级归 `/admin/advanced?tab=updates`
 * （`.octop-ref/octop/dashboard/src/routes/index.tsx:229,236,244`），
 * 都在 admin 区；它**压根没有工作区页**（`:213` 把 `/workspace` 重定向到专家页）。
 *
 * 这组判据盯的就是那条归属关系。
 */

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

function serverError(status: number, code: string, detail: string, nextStep: string): Response {
  return json({ error: { code, detail, next_step: nextStep } }, status)
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
  localStorage.clear()
})

/** 管理区的页签里必须有「备份与升级」，否则用户找不到它。 */
describe('备份与升级归 admin 区', () => {
  it('管理页签里有「备份与升级」', () => {
    render(
      <MemoryRouter>
        <AdminTabs />
      </MemoryRouter>,
    )
    expect(screen.getByRole('link', { name: '备份与升级' })).toHaveAttribute('href', '/admin/backup')
  })

  /**
   * 页签只有一份。若 `AdminTabs` 被复制到两个文件里，将来加一项必然只改一处，
   * 于是漂移 —— 「共享 MCP」就是这么消失的（见 `AdminTabs.tsx` 里那条注释）。
   *
   * 用 `?raw` 读源文件而不是 `node:fs`：测试文件不在 tsconfig 的 node types 里，
   * 直接 import `node:fs` 过不了 `tsc -b`。
   */
  it('管理页签只有一处定义，不在每个页各写一份', () => {
    const files = { 'Admin.tsx': adminSrc, 'AdminBackup.tsx': backupSrc, 'AdminTabs.tsx': tabsSrc }
    const defining = Object.entries(files)
      .filter(([, src]) => /export function AdminTabs\b/.test(src))
      .map(([name]) => name)
    expect(defining, 'AdminTabs 在多个文件里各定义了一份').toEqual(['AdminTabs.tsx'])
  })
})

/**
 * 工作区页不再有备份。
 *
 * 这条挡的是「搬回去」：只要工作区页还渲染着备份按钮，导航里两个入口
 * 操作同一份数据，改一处另一处不跟着变 —— 那正是「共享 MCP」的下场。
 */
describe('工作区页不再拿备份顶替', () => {
  it('工作区页里没有导出/校验备份的按钮', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => serverError(501, 'not_implemented', '尚未实现。', '等后端实现。')),
    )
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <MemoryRouter>
        <QueryClientProvider client={client}>
          <WorkspacePage />
        </QueryClientProvider>
      </MemoryRouter>,
    )

    await waitFor(() =>
      expect(screen.getByText(/工作区文件接口尚未接通/)).toBeInTheDocument(),
    )
    // 这四个词一个都不许出现 —— 出现就说明备份又被塞回工作区了。
    for (const label of ['导出备份', '校验备份', '准备升级', '还原要先停掉服务端']) {
      expect(screen.queryByText(new RegExp(label)), `工作区页不该再有「${label}」`).toBeNull()
    }
  })

  it('工作区页说明文件接口没接通，并指向备份与升级的新位置', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => serverError(501, 'not_implemented', '尚未实现。', '等后端实现。')),
    )
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <MemoryRouter>
        <QueryClientProvider client={client}>
          <WorkspacePage />
        </QueryClientProvider>
      </MemoryRouter>,
    )

    await waitFor(() => expect(screen.getByText(/api\/workspace\/files/)).toBeInTheDocument())
    // 说清搬走了 —— 只把东西挪走而不说，用户会以为功能掉了。
    expect(screen.getByText(/已经搬到/)).toBeInTheDocument()
  })
})

/**
 * 「实例设置」不许有 disabled 的保存按钮。
 *
 * 这一页以前有四张卡片（默认人格 / 外部体检服务 / 配额 / 网络抓取），
 * 输入框全 `readOnly`、保存按钮全 `disabled`、占位全写「（未接通）」。
 * 四个点不动的按钮看起来是设置入口，用户会一个一个去试，试完只得出
 * 「这功能坏了」—— 而真相是后端压根没那条路由。
 *
 * 项目纪律：不画没有后端的假开关/假按钮。
 */
describe('实例设置不画点不动的按钮', () => {
  it.each(['保存默认人格', '保存配额', '保存网络策略'])('没有「%s」这个点不动的按钮', async (label) => {
    render(
      <MemoryRouter>
        <QueryClientProvider client={new QueryClient()}>
          <AdminInstancePage />
        </QueryClientProvider>
      </MemoryRouter>,
    )
    expect(screen.queryByRole('button', { name: label }), `「${label}」是 disabled 的假按钮`).toBeNull()
  })

  it('逐项说清为什么不能改，并指出真该去哪改', async () => {
    render(
      <MemoryRouter>
        <QueryClientProvider client={new QueryClient()}>
          <AdminInstancePage />
        </QueryClientProvider>
      </MemoryRouter>,
    )
    // 四项都在，且都写明了「只读 / 未登记」——不能只说「未接通」三个字。
    for (const field of ['default_soul', 'quota_bytes', 'web_fetch_denylist']) {
      expect(screen.getByText(new RegExp(field))).toBeInTheDocument()
    }
    // 真该走哪条路也得说，否则用户以为这功能没法配了。
    expect(screen.getByText(/quill doctor/)).toBeInTheDocument()
  })
})
