import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { ThemeProvider } from '../theme/ThemeToggle'
import { AuthPage } from './auth'

function jsonResponse(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  })
}

const SETUP_READY = { setup_required: false, user_count: 3, registration_enabled: false }
const SETUP_NEEDED = { setup_required: true, user_count: 0, registration_enabled: false }
const ME_ALICE = { id: 'a'.repeat(32), username: 'alice', display_name: 'Alice', role: 'owner', is_admin: true, approval_mode: 'manual' }

/**
 * 登录页会并发打三个接口：`/api/auth/me`（判断是否已登录）、
 * `/api/setup/status`（首管闸门），以及用户触发的提交。
 */
function stubApi(handlers: {
  setup?: unknown
  me?: () => Response
  onLogin?: (body: unknown) => Response
  onInitialAdmin?: (body: unknown) => Response
}): ReturnType<typeof vi.fn> {
  const fetchMock = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
    const path = String(url)
    const method = init?.method ?? 'GET'
    if (path === '/api/auth/me') return Promise.resolve((handlers.me ?? (() => jsonResponse(ME_ALICE, 401)))())
    if (path === '/api/setup/status') return Promise.resolve(jsonResponse(handlers.setup ?? SETUP_READY))
    if (path === '/api/auth/login' && method === 'POST') {
      return Promise.resolve(handlers.onLogin?.(JSON.parse(String(init?.body))) ?? jsonResponse({
        access_token: 'session-token', token_type: 'Bearer', expires_in: 3600, expires_at: 1,
        user: { id: ME_ALICE.id, username: 'alice', role: 'owner' },
      }))
    }
    if (path === '/api/setup/initial-admin' && method === 'POST') {
      return Promise.resolve(handlers.onInitialAdmin?.(JSON.parse(String(init?.body))) ?? jsonResponse({
        id: ME_ALICE.id, username: 'alice', display_name: 'Alice', role: 'owner', status: 'active',
      }, 201))
    }
    return Promise.resolve(jsonResponse({ error: { code: 'not_found', detail: '未找到', next_step: '' } }, 404))
  })
  vi.stubGlobal('fetch', fetchMock)
  return fetchMock
}

function renderPage(): void {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <MemoryRouter initialEntries={['/login']}>
      <QueryClientProvider client={client}>
        <ThemeProvider>
          <AuthPage />
        </ThemeProvider>
      </QueryClientProvider>
    </MemoryRouter>,
  )
}

/**
 * 表单 label 里包着 `<small>` 提示，getByLabelText 匹配不到全等文本
 * （与 experts/TeamsTab.test.tsx 同一个坑），所以一律按 name 取。
 */
function field(name: string): HTMLInputElement {
  return document.querySelector(`input[name="${name}"]`) as HTMLInputElement
}

/** 登录表单要等 setup 闸门放行后才出现。 */
async function waitForLoginForm(): Promise<void> {
  await waitFor(() => expect(field('username')).toBeInTheDocument())
}

function fillCredentials(username = 'alice', password = 'correct-horse-battery'): void {
  fireEvent.change(field('username'), { target: { value: username } })
  fireEvent.change(field('password'), { target: { value: password } })
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
  localStorage.clear()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

it('setup_required 时只显示首管引导，不给任何登录入口', async () => {
  stubApi({ setup: SETUP_NEEDED })
  renderPage()

  await waitFor(() => expect(screen.getByRole('heading', { name: '创建初始管理员' })).toBeInTheDocument())
  // 此时 users 表为空，没有任何账号可登录：显示登录表单只会让人对着必然 401 的表单点。
  // 注意首管表单**自己**也有 username/password 字段，所以这里认的是登录专属的标记。
  expect(document.querySelector('input[name="token"]')).toBeNull()
  expect(document.querySelector('input[name="display_name"]')).toBeInTheDocument()
  expect(screen.queryByRole('tab')).not.toBeInTheDocument()
  expect(screen.queryByRole('button', { name: '登录' })).not.toBeInTheDocument()
  expect(screen.queryByRole('button', { name: '连接' })).not.toBeInTheDocument()
  expect(screen.getByRole('button', { name: '创建管理员' })).toBeInTheDocument()
  // 首管表单要说明「注册之后永久关闭」，否则用户会以为漏了注册入口。
  expect(screen.getByText(/注册通道会永久关闭/)).toBeInTheDocument()
})

it('首管表单只发契约允许的三个字段，显示名留空时整个键都不出现', async () => {
  let captured: Record<string, unknown> | null = null
  stubApi({
    setup: SETUP_NEEDED,
    onInitialAdmin: (body) => {
      captured = body as Record<string, unknown>
      return jsonResponse({ id: ME_ALICE.id, username: 'alice', display_name: 'alice', role: 'owner', status: 'active' }, 201)
    },
  })
  renderPage()
  await waitFor(() => expect(screen.getByRole('button', { name: '创建管理员' })).toBeInTheDocument())

  fireEvent.change(field('username'), { target: { value: 'alice' } })
  fireEvent.change(field('password'), { target: { value: 'correct-horse-battery' } })
  fireEvent.click(screen.getByRole('button', { name: '创建管理员' }))

  // 服务端 only_keys 白名单：多传任何一个字段都 400。
  await waitFor(() => expect(captured).not.toBeNull())
  expect(captured).toEqual({ username: 'alice', password: 'correct-horse-battery' })
})

it('登录成功写入服务端签发的令牌', async () => {
  stubApi({ setup: SETUP_READY, me: () => jsonResponse(ME_ALICE) })
  // 首次 me 探测必须是 401，否则页面会直接跳走，根本轮不到提交。
  let meCalls = 0
  const fetchMock = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
    const path = String(url)
    if (path === '/api/auth/me') {
      meCalls += 1
      return Promise.resolve(jsonResponse(ME_ALICE, meCalls === 1 ? 401 : 200))
    }
    if (path === '/api/setup/status') return Promise.resolve(jsonResponse(SETUP_READY))
    if (path === '/api/auth/login' && init?.method === 'POST') {
      return Promise.resolve(jsonResponse({
        access_token: 'session-token', token_type: 'Bearer', expires_in: 3600, expires_at: 1,
        user: { id: ME_ALICE.id, username: 'alice', role: 'owner' },
      }))
    }
    return Promise.resolve(jsonResponse({ error: { code: 'not_found', detail: '未找到', next_step: '' } }, 404))
  })
  vi.stubGlobal('fetch', fetchMock)
  renderPage()

  await waitForLoginForm()
  fillCredentials()
  fireEvent.click(screen.getByRole('button', { name: '登录' }))

  await waitFor(() => expect(localStorage.getItem('quill-token')).toBe('session-token'))
  // 登录请求体只带 username + password，契约规定多传会 400。
  const login = fetchMock.mock.calls.find((call) => String(call[0]) === '/api/auth/login') as [string, RequestInit]
  expect(JSON.parse(String(login[1].body))).toEqual({ username: 'alice', password: 'correct-horse-battery' })
})

it('401 原样显示服务端 detail，且不因文案差异暗示用户名是否存在', async () => {
  // 后端对「用户不存在」和「口令错误」返回逐字相同的 401（防用户名枚举）。
  const DETAIL = '用户名或口令不正确。'
  const respond = (): Response =>
    jsonResponse({ error: { code: 'unauthorized', detail: DETAIL, next_step: '确认口令后重试。' } }, 401)

  stubApi({ setup: SETUP_READY, onLogin: respond })
  renderPage()
  await waitForLoginForm()
  fillCredentials('alice', 'wrong-password')
  fireEvent.click(screen.getByRole('button', { name: '登录' }))

  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent(DETAIL))
  // next_step 单独一行。
  expect(screen.getByText('确认口令后重试。')).toBeInTheDocument()
  // 登录失败后不得写令牌。
  expect(localStorage.getItem('quill-token')).toBeNull()
  // 前端不得自己加任何「该用户名不存在」式的提示。
  expect(screen.getByRole('alert')).not.toHaveTextContent('不存在')
  expect(screen.getByRole('alert')).not.toHaveTextContent('未注册')

  // 换个同样不存在的用户名，得到的是逐字相同的界面。
  stubApi({ setup: SETUP_READY, onLogin: respond })
  renderPage()
  await waitForLoginForm()
  fillCredentials('nobody-at-all', 'wrong-password')
  fireEvent.click(screen.getByRole('button', { name: '登录' }))
  await waitFor(() => expect(screen.getAllByRole('alert')[0]).toHaveTextContent(DETAIL))
})

it('429 显示 Retry-After 秒数并禁用提交按钮', async () => {
  stubApi({
    setup: SETUP_READY,
    onLogin: () => jsonResponse(
      { error: { code: 'too_many_requests', detail: '登录尝试过于频繁，已被临时挡下。', next_step: '等 30 秒后重试。' } },
      429,
      { 'Retry-After': '30' },
    ),
  })
  renderPage()
  await waitForLoginForm()
  fillCredentials()
  const submit = screen.getByRole('button', { name: '登录' })
  fireEvent.click(submit)

  await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('30'))
  // 限流窗口内必须挡住提交：用户再点就是无谓的 429。
  expect(screen.getByRole('button', { name: '登录' })).toBeDisabled()
  // 也不能自作主张发请求去轮询限流状态。
  expect(screen.getByRole('alert')).toHaveTextContent('登录尝试过于频繁')
})

it('两种登录方式并列，切换时清空另一种的输入', async () => {
  stubApi({ setup: SETUP_READY })
  renderPage()
  await waitForLoginForm()

  // 令牌入口必须保留：QUILL_TOKENS 用户靠它进站。
  expect(screen.getByRole('tab', { name: '账号登录' })).toBeInTheDocument()
  expect(screen.getByRole('tab', { name: '访问令牌' })).toBeInTheDocument()

  fillCredentials('alice', 'correct-horse-battery')
  fireEvent.click(screen.getByRole('tab', { name: '访问令牌' }))

  // 切到令牌：账号口令的输入（含半截口令）不能留在 DOM 里。
  await waitFor(() => expect(field('token')).toBeInTheDocument())
  expect(field('username')).toBeNull()
  expect(field('token').value).toBe('')

  fireEvent.change(field('token'), { target: { value: 'dev-token' } })
  fireEvent.click(screen.getByRole('tab', { name: '账号登录' }))
  await waitForLoginForm()
  expect(field('username').value).toBe('')
  expect(field('password').value).toBe('')
})

it('setup 状态查不到时不猜，照样给登录表单', async () => {
  vi.stubGlobal('fetch', vi.fn().mockImplementation((url: string) => {
    if (String(url) === '/api/setup/status') {
      return Promise.resolve(jsonResponse({ error: { code: 'internal_error', detail: '查询失败', next_step: '看日志。' } }, 500))
    }
    return Promise.resolve(jsonResponse(ME_ALICE, 401))
  }))
  renderPage()

  // 绝大多数实例已初始化过，因这里失败就把人挡在门外比多显示一个引导按钮糟得多。
  await waitForLoginForm()
  expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument()
  expect(screen.queryByRole('button', { name: '创建管理员' })).not.toBeInTheDocument()

  // 关键：探测失败时**仍然要能真的登录**。
  // 之前这里渲染的是一张只有标题和重试按钮、没有任何输入框的卡片 ——
  // 用户撞到一次瞬时 500 就彻底登不进去了，而测试只断言了重试按钮存在，
  // 于是这个死胡同一直是绿的。
  expect(field('username')).toBeInTheDocument()
  expect(field('password')).toBeInTheDocument()

  // 并且它得真的能用：填了能提交。
  fireEvent.change(field('username'), { target: { value: 'alice' } })
  fireEvent.change(field('password'), { target: { value: 'correct-horse-battery' } })
  fireEvent.submit(field('username').closest('form') as HTMLFormElement)
  await waitFor(() => {
    const called = (vi.mocked(fetch).mock.calls as unknown[][]).map((c) => c[0])
    expect(called).toContain('/api/auth/login')
  })
})

