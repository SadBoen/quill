import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MemoryRouter } from 'react-router-dom'

import i18n from '../i18n'
import { MemoryPage } from './MemoryPage'

/**
 * 资料库页的**写入**（queue Q059，接的是 Q058 的 `PUT`/`DELETE`）。
 *
 * 这一页最容易出的两种错，正是下面钉住的两条：
 *   1. 保存时不带 `version`（或带错）→ 服务端只能回 409，但界面**假装保存成功**，
 *      用户以为改动落地了，其实一个字都没写；
 *   2. 冲突（409）被吞掉 —— 服务端在 detail 里回了**当前**版本号，那是用户重试的
 *      唯一依据，界面必须原样端出来。
 */

const PAGE_PATH = 'concepts/入门.md'
const PAGE_URL = '/api/wiki/pages/concepts/%E5%85%A5%E9%97%A8.md'
const LIST_URL = '/api/wiki/pages'
const INDEX_URL = '/api/wiki/index'
const LOG_URL = '/api/wiki/log'

const PAGE = '---\ntitle: 入门\ntype: concept\ncreated: 2026-10-04\nupdated: 2026-10-04\n---\n\n这是入门页。\n'

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

/** 服务端错误信封：字段名见 `api/client.ts` 顶部注释（嵌套一层 `error`）。 */
function serverError(status: number, code: string, detail: string, nextStep: string): Response {
  return json({ error: { code, detail, next_step: nextStep } }, status)
}

interface Call {
  method: string
  url: string
  body: Record<string, unknown> | null
}

function renderPage(handlers: Record<string, () => Response>): { calls: Call[] } {
  const calls: Call[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string, init?: RequestInit) => {
      const method = init?.method ?? 'GET'
      calls.push({
        method,
        url,
        body: init?.body ? JSON.parse(String(init.body)) : null,
      })
      const handler = handlers[`${method} ${url}`] ?? handlers[url]
      if (handler) return handler()
      // 没点名的路由给「这台实例没有」——和真后端的 404 信封同形。
      return serverError(404, 'not_found', `${url} 未登记。`, '实现该路由。')
    }),
  )
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <MemoryPage />
      </QueryClientProvider>
    </MemoryRouter>,
  )
  return { calls }
}

/** 默认的只读三件套 + 这一页（version = v1）。 */
function baseHandlers(extra: Record<string, () => Response> = {}): Record<string, () => Response> {
  return {
    [LIST_URL]: () => json({ pages: [PAGE_PATH] }),
    [INDEX_URL]: () => json({ index: '- [[入门]]：入门页', present: true }),
    [LOG_URL]: () => json({ log: '## 2026-10-04\n\n建库', present: true }),
    [PAGE_URL]: () =>
      json({ path: PAGE_PATH, content: PAGE, title: '入门', version: 'v1' }),
    ...extra,
  }
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('资料库页：能编辑、能保存、冲突不吞', () => {
  it('保存时带上加载时那一版的 version，内容用改后的', async () => {
    const { calls } = renderPage(
      baseHandlers({
        [`PUT ${PAGE_URL}`]: () =>
          json({ path: PAGE_PATH, version: 'v2', index_entries: 1, created: false }),
      }),
    )

    // 等页面真的读出来（编辑按钮出现）。
    fireEvent.click(await screen.findByRole('button', { name: '编辑' }))
    const body = screen.getByLabelText('正文')
    fireEvent.change(body, { target: { value: PAGE.replace('这是入门页。', '改过了。') } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    await waitFor(() => {
      expect(calls.some((c) => c.method === 'PUT')).toBe(true)
    })
    const put = calls.find((c) => c.method === 'PUT')
    expect(put?.url).toBe(PAGE_URL)
    expect(put?.body?.expected_version).toBe('v1')
    expect(String(put?.body?.content)).toContain('改过了。')
  })

  it('409 冲突：把服务端的原话（含当前版本号）端出来，且不假装保存成功', async () => {
    renderPage(
      baseHandlers({
        [`PUT ${PAGE_URL}`]: () =>
          serverError(
            409,
            'conflict',
            '资料库页面 "concepts/入门.md" 的版本对不上：你手上那一版已经过期。当前版本是 "v2"。',
            '先取最新版本再重试；不要在旧版本上强行覆盖',
          ),
      }),
    )

    fireEvent.click(await screen.findByRole('button', { name: '编辑' }))
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('已经过期')
    expect(alert, '当前版本号是用户重试的唯一依据，必须端出来').toHaveTextContent('v2')
    // 仍在编辑态：没清草稿、没退出编辑 —— 也就是没把失败画成成功。
    expect(screen.getByRole('button', { name: '保存' })).toBeInTheDocument()
  })

  it('删除带上这一页的版本号（删除不可逆，服务端缺它就 400）', async () => {
    const { calls } = renderPage(
      baseHandlers({
        [`DELETE ${PAGE_URL}?expected_version=v1`]: () =>
          json({ path: PAGE_PATH, deleted: true, index_entries: 0 }),
      }),
    )

    fireEvent.click(await screen.findByRole('button', { name: '删除这一页' }))

    await waitFor(() => {
      expect(calls.some((c) => c.method === 'DELETE')).toBe(true)
    })
    const del = calls.find((c) => c.method === 'DELETE')
    expect(del?.url).toBe(`${PAGE_URL}?expected_version=v1`)
  })

  it('页面不存在时给「新建这一页」，保存按新建（expected_version = null）', async () => {
    const missing = 'concepts/没有这页.md'
    const missingUrl = '/api/wiki/pages/concepts/%E6%B2%A1%E6%9C%89%E8%BF%99%E9%A1%B5.md'
    const { calls } = renderPage({
      ...baseHandlers({
        [`PUT ${missingUrl}`]: () =>
          json({ path: missing, version: 'vNew', index_entries: 1, created: true }),
      }),
      [LIST_URL]: () => json({ pages: [PAGE_PATH, missing] }),
      // 让页面默认落在「不存在」那一页上：列表里第一项就是它。
      [missingUrl]: () => serverError(404, 'entity_not_found', '没有这一页。', '新建它。'),
    })

    // 切到不存在的那一页。
    fireEvent.click(await screen.findByRole('button', { name: PAGE_PATH }))
    fireEvent.click(await screen.findByRole('button', { name: missing }))

    fireEvent.click(await screen.findByRole('button', { name: '新建这一页' }))
    // 模板已经填好 frontmatter（服务端要求第一行 `---` 且必须闭合），否则第一次保存必 400。
    expect((screen.getByLabelText('正文') as HTMLTextAreaElement).value).toContain('---')
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    await waitFor(() => {
      expect(calls.some((c) => c.method === 'PUT')).toBe(true)
    })
    const put = calls.find((c) => c.method === 'PUT')
    expect(put?.url).toBe(missingUrl)
    expect(put?.body?.expected_version, '新建必须是 null，否则服务端按覆盖判').toBeNull()
  })
})
