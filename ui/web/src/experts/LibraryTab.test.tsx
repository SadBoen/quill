import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { LibraryTab } from './LibraryTab'

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

function expert(over: Record<string, unknown>): Record<string, unknown> {
  return {
    owner: 'me',
    display_name: '模板专家 1',
    description: 'd',
    instructions: '你是模板专家',
    model: null,
    visibility: 'private',
    default_enabled: true,
    is_builtin: false,
    source_template: null,
    ...over,
  }
}

function renderTab(experts: Record<string, unknown>[]): { fetchMock: ReturnType<typeof vi.fn> } {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const fetchMock = vi.fn().mockImplementation((_input: RequestInfo | URL, init?: RequestInit) => {
    if (!init || init.method === undefined) return Promise.resolve(jsonResponse({ experts }))
    if (init.method === 'POST') {
      const body = JSON.parse(String(init.body)) as Record<string, unknown>
      return Promise.resolve(jsonResponse(expert({ ...body, owner: 'me' }), 201))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetchMock)
  render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <LibraryTab />
      </QueryClientProvider>
    </MemoryRouter>,
  )
  return { fetchMock }
}

const TEMPLATE = 'ai-coding-coach'

function card(): HTMLElement {
  return document.querySelector(`[data-expert="${TEMPLATE}"]`) as HTMLElement
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

afterEach(() => {
  vi.unstubAllGlobals()
})

it('同一个模板可以反复生成，派生数量来自 source_template 而不是模板 id', async () => {
  const { fetchMock } = renderTab([
    expert({ id: 'ai-coding-coach-1', display_name: '模板专家 1', source_template: TEMPLATE }),
  ])
  await waitFor(() => expect(within(card()).getByText('已基于此模板创建 1 个')).toBeInTheDocument())

  // 旧行为在这里是 disabled：同 id 已存在就不让再点。
  const button = within(card()).getByRole('button', { name: '生成专家' })
  expect(button).not.toBeDisabled()
  fireEvent.click(button)

  const form = document.querySelector('#experts-generate-ai-coding-coach') as HTMLFormElement
  await waitFor(() => expect(form).toBeInTheDocument())
  const nameInput = form.querySelector('input[name="display_name"]') as HTMLInputElement
  const idInput = form.querySelector('input[name="id"]') as HTMLInputElement
  // 已有 1 个派生 → 序号 n = 2。显示名含中文，标识不取规整结果（那会得到
  // 没有辨识度的 ai-2），统一退回「模板id-序号」。
  expect(nameInput.value).toBe('AI 编程实战导师 2')
  expect(idInput.value).toBe('ai-coding-coach-2')

  fireEvent.submit(form)
  await waitFor(() => expect(fetchMock.mock.calls.some((call) => call[1]?.method === 'POST')).toBe(true))
  const post = fetchMock.mock.calls.find((call) => call[1]?.method === 'POST') as [unknown, RequestInit]
  const body = JSON.parse(String(post[1].body)) as Record<string, unknown>
  expect(body.id).toBe('ai-coding-coach-2')
  expect(body.source_template).toBe(TEMPLATE)
  expect(String(body.instructions).length).toBeGreaterThan(0)
})

it('生成面板预填模板人格全文，并如实说明原模板不受影响', async () => {
  renderTab([])
  await waitFor(() => expect(within(card()).getByRole('button', { name: '生成专家' })).toBeInTheDocument())
  fireEvent.click(within(card()).getByRole('button', { name: '生成专家' }))

  const form = await waitFor(() => document.querySelector('#experts-generate-ai-coding-coach') as HTMLFormElement)
  const textarea = form.querySelector('textarea[name="instructions"]') as HTMLTextAreaElement
  expect(textarea.value.trim().length).toBeGreaterThan(200)
  expect(Number(textarea.getAttribute('rows'))).toBeGreaterThanOrEqual(14)
  expect(within(card()).getByText(/原模板不受影响/)).toBeInTheDocument()
  // 只读来源说明，不做成可编辑控件。
  expect(within(card()).getByText(/此专家基于模板/)).toBeInTheDocument()
})

it('没做到能用的能力只给路由名，不在表单里放假开关', async () => {
  renderTab([])
  await waitFor(() => expect(within(card()).getByRole('button', { name: '生成专家' })).toBeInTheDocument())
  fireEvent.click(within(card()).getByRole('button', { name: '生成专家' }))
  const form = await waitFor(() => document.querySelector('#experts-generate-ai-coding-coach') as HTMLFormElement)

  expect(within(card()).getByText('GET /api/extensions/skills')).toBeInTheDocument()
  expect(within(card()).getByText('GET /api/extensions/plugins')).toBeInTheDocument()
  expect(within(card()).getByText('GET /api/extensions/mcp')).toBeInTheDocument()
  expect(within(card()).getByText('/api/cron')).toBeInTheDocument()
  const names = Array.from(form.querySelectorAll('input, select, textarea')).map((el) => el.getAttribute('name'))
  expect(names).not.toContain('skills')
  expect(names).not.toContain('mcp')
  expect(names).not.toContain('cron')
})

it('MCP 与技能包标「部分接通」，不标 501 —— 它们的路由是 200', async () => {
  // 这两条路由真的存在且返回 200（`GET /api/extensions/mcp` /
  // `GET /api/extensions/skills`），只是对话里还调不到。写成 501 等于
  // 让用户去「接」一个早就接好的东西，比不说还糟。
  renderTab([])
  await waitFor(() => expect(within(card()).getByRole('button', { name: '生成专家' })).toBeInTheDocument())
  fireEvent.click(within(card()).getByRole('button', { name: '生成专家' }))
  await waitFor(() => expect(within(card()).getByText('GET /api/extensions/mcp')).toBeInTheDocument())

  expect(within(card()).getByText(/部分接通 · 配置能存能读/)).toBeInTheDocument()
  expect(within(card()).getByText(/部分接通 · 能存能读/)).toBeInTheDocument()
  // 路由压根不存在的那两条才是「未接通」。
  expect(within(card()).getByText('未接通 · 501')).toBeInTheDocument()
  expect(within(card()).getByText('未接通 · 路由未注册')).toBeInTheDocument()
})

it('后端拒绝时显示服务端中文原文', async () => {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const fetchMock = vi.fn().mockImplementation((_input: RequestInfo | URL, init?: RequestInit) => {
    if (!init || init.method === undefined) return Promise.resolve(jsonResponse({ experts: [] }))
    return Promise.resolve(jsonResponse({ error: { code: 'conflict', detail: '标识 expert-x 已被占用', next_step: '换一个标识。' } }, 409))
  })
  vi.stubGlobal('fetch', fetchMock)
  render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <LibraryTab />
      </QueryClientProvider>
    </MemoryRouter>,
  )
  await waitFor(() => expect(within(card()).getByRole('button', { name: '生成专家' })).toBeInTheDocument())
  fireEvent.click(within(card()).getByRole('button', { name: '生成专家' }))
  const form = await waitFor(() => document.querySelector('#experts-generate-ai-coding-coach') as HTMLFormElement)
  fireEvent.submit(form)
  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('标识 expert-x 已被占用'))
})

it('展开区列出派生专家的显示名与 id，不画跳转链接', async () => {
  renderTab([
    expert({ id: 'ai-coding-coach-1', display_name: '程序员1号', source_template: TEMPLATE }),
    expert({ id: 'ai-coding-coach-2', display_name: '程序员2号', source_template: TEMPLATE }),
  ])
  await waitFor(() => expect(within(card()).getByText('已基于此模板创建 2 个')).toBeInTheDocument())
  fireEvent.click(within(card()).getByRole('button', { name: '生成专家' }))
  await waitFor(() => expect(within(card()).getByText('程序员1号')).toBeInTheDocument())
  expect(within(card()).getByText('ai-coding-coach-1')).toBeInTheDocument()
  expect(within(card()).getByText('程序员2号')).toBeInTheDocument()
  expect(card().querySelectorAll('a')).toHaveLength(0)
})
