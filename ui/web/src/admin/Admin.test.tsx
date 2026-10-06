import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import type { ReactNode } from 'react'

import i18n from '../i18n'
import { AdminUsersPage } from './Admin'
import { DEFAULT_PAGE_SIZE } from './api'

/**
 * 这组测试盯的是**用户会不会被误导**，不是「渲染出来没有」。
 *
 * 这一页以前是纯装饰：接口返 501、`AdminUser` 里的 email/locked/quota_bytes
 * 在这台实例里根本没有来源、翻页按钮只改前端 state。三个都是「界面上看着
 * 挺像那么回事、其实不成立」。所以每条测试都必须能说明它拦住的是哪一种。
 */
function user(over: Partial<Record<string, unknown>> = {}) {
  return {
    id: 'u-bob',
    username: 'bob',
    display_name: 'Bob',
    role: 'member' as const,
    status: 'active' as const,
    created_at_ms: 1,
    last_login_at_ms: null,
    has_env_token: false,
    ...over,
  }
}

type Handler = () => Promise<{ status: number; body: unknown }>

function harness(
  handlers: { list?: Handler; patch?: Handler } = {},
  spy?: (req: { url: string; method: string; body: unknown }) => void,
) {
  const calls: { url: string; method: string; body: unknown }[] = []
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = (init?.method ?? 'GET').toUpperCase()
    const body = init?.body ? JSON.parse(String(init.body)) : undefined
    calls.push({ url, method, body })
    spy?.({ url, method, body })
    // **按方法分流**，不靠「路径以什么开头」猜。原来用 startsWith 匹配，
    // `/api/users` 会把 `PATCH /api/users/u-bob` 也吃掉 ——
    // 409 那条用例于是永远命中 200，测的是一个不存在的分支。
    const h = method === 'PATCH' ? handlers.patch : handlers.list
    const r = h
      ? await h()
      : { status: 404, body: { error: { code: 'not_found', detail: '没有这个路由' } } }
    return new Response(JSON.stringify(r.body), {
      status: r.status,
      headers: { 'content-type': 'application/json' },
    })
  })
  vi.stubGlobal('fetch', fetchMock)

  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const ui = (): ReactNode => (
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <AdminUsersPage />
      </MemoryRouter>
    </QueryClientProvider>
  )
  render(ui())
  return { calls }
}

beforeEach(async () => {
  // `src/test/setup.ts` 的 afterEach 会把语言切回 en，所以**每条**测试都得
  // 自己切回中文。只在文件顶部设一次是没用的 —— 第一条之后就被冲掉了。
  await i18n.changeLanguage('zh-CN')
  vi.unstubAllGlobals()
})

describe('用户名单：真的有数据', () => {
  it('渲染真实字段，不渲染那些没有来源的字段', async () => {
    harness({ list: () => Promise.resolve({ status: 200, body: { users: [user()], total: 1 } }) })

    expect(await screen.findByTestId('user-row-bob')).toBeInTheDocument()
    expect(screen.getByText('Bob')).toBeInTheDocument()
    // 服务端给的就是 username 与 display_name，不再是发明的 name/email。
    expect(screen.queryByText('bob@example.com')).toBeNull()
    expect(screen.queryByText('超配额')).toBeNull()
  })

  it('服务端返 200 但字段残缺时，不许渲染成一张有数据的表', async () => {
    // 老接口类型是「数组」，现在是 {users,total}。要是服务端还没换，
    // 这条会把一个不认识的形状照单全收 —— 那正是最该拦住的事。
    harness({ list: () => Promise.resolve({ status: 200, body: [{ id: 'u-bob' }] }) })

    await waitFor(() => expect(screen.queryByTestId('user-row-bob')).toBeNull())
    expect(screen.queryByText('Bob')).toBeNull()
  })

  it('名册里有一个人字段残缺时，整张表都不许当成有效数据渲染', async () => {
    // 这一条专门盖住「信封对、里面某个人不对」的情形 ——
    // 只把整体当数组/对象判一下是抓不到它的，必须逐个人核字段。
    harness({
      list: () =>
        Promise.resolve({ status: 200, body: { users: [{ id: 'u-bob' }], total: 1 } }),
    })

    await waitFor(() => expect(screen.getByTestId('users-unreadable')).toBeInTheDocument())
    expect(screen.queryByTestId('user-row-bob')).toBeNull()
    expect(screen.queryByText('Bob')).toBeNull()
  })

  it('名册为空时说明是空的，不显示任何示例行', async () => {
    harness({ list: () => Promise.resolve({ status: 200, body: { users: [], total: 0 } }) })

    expect(await screen.findByText('名册是空的。')).toBeInTheDocument()
    expect(screen.queryByTestId('user-row-bob')).toBeNull()
  })
})

describe('翻页是真的翻页', () => {
  it('上一页在第一页禁用；下一页按 total 判断而不是按本页条数', async () => {
    const urls: string[] = []
    harness(
      { list: () => Promise.resolve({ status: 200, body: { users: [user()], total: 120 } }) },
      (r) => urls.push(r.url),
    )

    await screen.findByTestId('user-row-bob')
    // 本页只有 1 条，但 total 是 120 —— 旧逻辑按本页条数判断，
    // 会得出「只有 1 条，没有下一页」，于是永远翻不到第二页。
    expect(screen.getByTestId('users-next-page')).not.toBeDisabled()
    expect(screen.getByTestId('users-previous-page')).toBeDisabled()
    expect(screen.getByText('共 120 个账号')).toBeInTheDocument()

    fireEvent.click(screen.getByTestId('users-next-page'))
    await waitFor(() => expect(urls.some((u) => u.includes(`offset=${DEFAULT_PAGE_SIZE}`))).toBe(true))
  })

  it('请求带上 offset 与 limit，不靠只改本地 state', async () => {
    const urls: string[] = []
    harness(
      { list: () => Promise.resolve({ status: 200, body: { users: [user()], total: 120 } }) },
      (r) => urls.push(r.url),
    )

    await screen.findByTestId('user-row-bob')
    await waitFor(() => expect(urls[0]).toContain('offset=0'))
    expect(urls[0]).toContain(`limit=${DEFAULT_PAGE_SIZE}`)
  })
})

describe('停用按钮说清它挡得住什么', () => {
  it('仍持有环境变量令牌的人，界面上必须说出来', async () => {
    harness({
      list: () =>
        Promise.resolve({ status: 200, body: { users: [user({ has_env_token: true })], total: 1 } }),
    })

    expect(await screen.findByTestId('user-env-token-bob')).toHaveTextContent('停用挡不住他')
  })

  it('没有令牌的人不重复喊那句警告', async () => {
    harness({ list: () => Promise.resolve({ status: 200, body: { users: [user()], total: 1 } }) })

    await screen.findByTestId('user-row-bob')
    expect(screen.queryByTestId('user-env-token-bob')).toBeNull()
  })

  it('点停用真的发出 PATCH，且带的是 id 原文', async () => {
    const calls: { url: string; method: string; body: unknown }[] = []
    harness(
      { list: () => Promise.resolve({ status: 200, body: { users: [user()], total: 1 } }) },
      (r) => calls.push(r),
    )

    await screen.findByTestId('user-row-bob')
    fireEvent.click(screen.getByTestId('user-toggle-bob'))

    await waitFor(() => {
      const patch = calls.find((c) => c.method === 'PATCH')
      expect(patch).toBeTruthy()
      expect(patch!.url).toContain('/api/users/u-bob')
      expect(patch!.body).toEqual({ status: 'disabled' })
    })
  })

  it('409 自己停用时，说的是「别把自己锁在门外」，不是通用错误', async () => {
    harness({
      list: () => Promise.resolve({ status: 200, body: { users: [user()], total: 1 } }),
      patch: () =>
        Promise.resolve({
          status: 409,
          body: {
            error: {
              code: 'conflict',
              detail: '不能把自己停用',
              next_step: '让别人停用你。',
            },
          },
        }),
    })

    await screen.findByTestId('user-row-bob')
    fireEvent.click(screen.getByTestId('user-toggle-bob'))

    // 403/409 各说各的话，不许都退化成「操作失败」。
    await waitFor(() => expect(screen.getByTestId('users-failure-help')).toBeInTheDocument())
    expect(screen.getByTestId('users-failure-help')).toHaveTextContent('锁在门外')
  })

  it('403 说清是权限问题、该换个账号，而不是让人去查服务端日志', async () => {
    harness({
      list: () => Promise.resolve({ status: 403, body: { error: { code: 'forbidden', detail: '只有 owner 才能' } } }),
    })

    const help = await screen.findByTestId('users-failure-help')
    expect(help).toHaveTextContent('换一个带 admin 的账号登录')
    expect(help).not.toHaveTextContent('服务端日志')
  })

  it('说明区讲的是「刻意不做」而不是「尚未实现」', async () => {
    harness({ list: () => Promise.resolve({ status: 200, body: { users: [user()], total: 1 } }) })

    const note = await screen.findByTestId('users-not-allowed')
    expect(note).toHaveTextContent('刻意不提供建号与删号')
    expect(note).not.toHaveTextContent('尚未实现')
  })
})