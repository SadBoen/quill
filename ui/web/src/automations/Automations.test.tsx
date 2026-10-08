import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter } from 'react-router-dom'

import i18n from '../i18n'
import { AutomationsPage } from './Automations'

/**
 * 自动化页的「点了必失败」红线（queue Q071）。
 *
 * `/api/cron*` 在 quill 后端**没有登记**（`grep -rn cron crates/quill-server/src` 零命中），
 * 所以这一页最容易长出一个能填、能提交、点下去必然 404 的表单 —— 用户会以为定时任务
 * 建成了，然后等它永远不跑。`api/capability.ts` 的 `routeMissing` 就是为这个加的：
 * 只认「服务端明说没这条路由」（404 + `not_found`），据此不画按钮、不画表单。
 *
 * 与 `capability.test.ts` 的分工：那边测判据本身，这边测**这一页真的照判据办事**。
 */

const CRON_LIST_URL = '/api/cron?limit=50&offset=0'
const DREAM_LIST_URL = '/api/dream?limit=50&offset=0'

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

/** 服务端错误信封：字段名见 `api/client.ts` 顶部的注释（嵌套一层 `error`）。 */
function serverError(status: number, code: string, detail: string, nextStep: string): Response {
  return json({ error: { code, detail, next_step: nextStep } }, status)
}

function renderPage(handlers: Record<string, () => Response>): {
  requests: string[]
} {
  const requests: string[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string, init?: RequestInit) => {
      requests.push(`${init?.method ?? 'GET'} ${url}`)
      const handler = handlers[url]
      if (!handler) {
        // 没点名的路由一律当成「这台实例没有」—— 这正是 /api/cron 与 /api/dream 的现状。
        return serverError(404, 'not_found', `${url} 未登记。`, '实现该路由。')
      }
      return handler()
    }),
  )
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <AutomationsPage />
      </QueryClientProvider>
    </MemoryRouter>,
  )
  return { requests }
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('定时任务：没这条路由就不画能点的东西', () => {
  it('/api/cron 未登记 → 不画「新建自动化」，也不画表单', async () => {
    // 两条路由都点名写成 404，别靠 renderPage 的兜底：这一页**同时**依赖两个未登记
    // 的路由（cron 与 dream），测试里明写出来，读的人才知道自己在模拟什么。
    const { requests } = renderPage({
      [DREAM_LIST_URL]: () => serverError(404, 'not_found', '/api/dream 未登记。', '实现该路由。'),
    })

    // 等页面**真的**进到「没这个能力」那一支（只等 fetch 发出会抢在 React Query
    // 写 error 之前断言，那是夹具的竞态，不是实现的问题）。
    await screen.findByText(/未在本实例登记/)

    expect(
      screen.queryByRole('button', { name: '新建自动化' }),
      '路由都没登记，画这个按钮就是画一个点了必然 404 的东西',
    ).toBeNull()
    // 一个必然失败的写请求都不该发出去。
    expect(requests.filter((r) => r.startsWith('POST ') || r.startsWith('PATCH '))).toHaveLength(0)
  })

  it('如实说清是「没这个能力」，并给出下一步，而不是假装「还没有任务」', async () => {
    renderPage({})

    const note = await screen.findByText(/未在本实例登记/)
    expect(note).toHaveTextContent('/api/cron')
    // 「还没有定时任务」是把「没这个能力」说成「还没建」——两句不能混。
    expect(screen.queryByText('还没有定时任务。')).toBeNull()
  })

  it('别的失败不算「没这个能力」：500 时按钮要留着（不能把能用的功能藏起来）', async () => {
    renderPage({
      [CRON_LIST_URL]: () =>
        serverError(500, 'internal', '数据库锁住了。', '稍后重试。'),
    })

    await screen.findByRole('alert')
    expect(screen.getByRole('button', { name: '新建自动化' })).toBeEnabled()
  })

  it('路由真接通后，列表照常渲染（守住「不是永远只画提示」）', async () => {
    renderPage({
      [CRON_LIST_URL]: () =>
        json({
          items: [
            {
              id: 'job-1',
              name: '每日摘要',
              schedule: { type: 'every', every_seconds: 86400 },
              last_fired_at: null,
              next_fire_at: '2026-10-10T09:00:00Z',
              session_id: null,
            },
          ],
          next_offset: null,
        }),
    })

    expect(await screen.findByText('每日摘要')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '新建自动化' })).toBeEnabled()
    expect(screen.queryByText('还没有定时任务。')).toBeNull()
  })
})
