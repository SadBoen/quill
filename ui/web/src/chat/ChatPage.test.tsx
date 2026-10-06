import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { ChatPage } from './ChatPage'

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

/** 服务端返回的专家列表。`is_general` 由服务端给，前端不许自己认 id。 */
const EXPERTS = {
  experts: [
    {
      id: 'general',
      owner: 'me',
      display_name: '通用助手',
      description: '不带特殊人格的通用助手',
      instructions: '你是这个工作台上的通用助手。',
      model: null,
      visibility: 'user_authored',
      default_enabled: true,
      is_builtin: false,
      is_general: true,
      source_template: null,
    },
  ],
}

function renderPage(entry: string, fetchMock: ReturnType<typeof vi.fn>): void {
  vi.stubGlobal('fetch', fetchMock)
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <MemoryRouter initialEntries={[entry]}>
      <QueryClientProvider client={client}>
        <Routes>
          <Route path="/chat" element={<ChatPage />} />
          <Route path="/chat/:sessionId" element={<ChatPage />} />
        </Routes>
      </QueryClientProvider>
    </MemoryRouter>,
  )
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
  localStorage.clear()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

/** 按路径返回对应载荷的 fetch 桩。 */
function routedFetch(overrides: Record<string, unknown> = {}) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/api/experts')) return jsonResponse(EXPERTS)
    if (url.includes('/api/sessions?')) return jsonResponse({ sessions: [] })
    if (url.includes('/healthz')) return jsonResponse({ llm: { max_context_tokens: 8192, compaction_threshold_tokens: 8000 } })
    for (const [suffix, body] of Object.entries(overrides)) {
      if (url.includes(suffix)) return jsonResponse(body)
    }
    return jsonResponse({})
  })
}

it('专家下拉里不再有「未选角色」这个选项', async () => {
  renderPage('/chat', routedFetch())
  await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument())

  const readOptions = () =>
    Array.from(screen.getByRole('combobox').querySelectorAll('option')).map((o) => o.textContent ?? '')

  // 过去这里有一个空 value 的「（默认）」选项，选它等于「不选角色」——
  // 而空 expert_id 并不表示「默认角色」，是根本没有角色在跑。
  // 只剩真实存在的通用专家一条，不多不少。
  await waitFor(() => expect(readOptions()).toEqual(['通用助手']))
  expect(readOptions().some((text) => text.includes('未选角色'))).toBe(false)
  expect(readOptions().some((text) => text.includes('（默认）'))).toBe(false)
})

it('没显式选角色时，默认落在通用专家上而不是空值', async () => {
  renderPage('/chat', routedFetch())
  await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument())
  await waitFor(() => expect(screen.getByRole('combobox')).toHaveValue('general'))
})

it('界面显示真实的通用专家名，不显示编出来的「默认」', async () => {
  renderPage('/chat', routedFetch())
  // 「通用助手」在侧栏和下拉里各出现一次，所以用 getAllByText
  await waitFor(() => expect(screen.getAllByText('通用助手').length).toBeGreaterThan(0))
  // 「默认」是一个并不存在的角色名，用户会以为有个叫「默认」的角色在管他。
  expect(screen.queryByText('默认')).not.toBeInTheDocument()
  expect(screen.queryByText('默认（未选角色）')).not.toBeInTheDocument()
})

it('专家还没加载出来时说实话「加载中」，不假装有个叫「默认」的角色', async () => {
  // 让 /api/experts 一直挂着，模拟慢网络
  const slow = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/api/experts')) return new Promise<Response>(() => {})
    if (url.includes('/api/sessions?')) return jsonResponse({ sessions: [] })
    if (url.includes('/healthz')) return jsonResponse({ llm: {} })
    return jsonResponse({})
  })
  renderPage('/chat', slow)
  await waitFor(() => expect(screen.getByText('角色加载中…')).toBeInTheDocument())
})