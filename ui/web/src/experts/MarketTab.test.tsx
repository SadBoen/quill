import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { MarketTab } from './MarketTab'

/**
 * 市场页要同时说准几件事，少一件就是在骗人：
 *
 * 1. **上游地址要显示**（这不是内置列表）。
 * 2. **「没接通」「连不上上游」「上游说 0 个」是三件事**，混成一个空列表
 *    就等于告诉用户「市场里没有」。
 * 3. **上游没给的就是没给**：空 summary 显示「上游没给说明」，
 *    `total: null` 显示「共 ? 个」，都不许拿 slug / 本页条数顶替。
 * 4. **三个技能状态各有各的中文说法**，failed 必须带出原因。
 * 5. **mutation 的错误必须可见**，且要带着服务端的原文与「下一步」。
 * 6. **装进来的技能是停用的**，界面要写出来并给下一步。
 */

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

const LIST = {
  page: 1,
  page_size: 20,
  total: 3348,
  host: 'https://api.skillhub.cn',
  items: [
    {
      slug: 'pdf-toolkit',
      display_name: 'PDF 工具箱',
      summary: '上游给的说明。',
      scene: 'office',
      skill_slugs: ['pdf-extract', 'pdf-merge'],
      skill_count: 2,
      installed: false,
    },
    {
      slug: 'wechat-ops',
      display_name: '微信运营',
      summary: '',
      scene: 'ops',
      skill_slugs: ['wechat-post'],
      skill_count: 1,
      installed: true,
    },
  ],
}

const INSTALL_OK = {
  already_installed: false,
  expert: { id: 'hub-pdf-toolkit', display_name: 'PDF 工具箱', description: '处理 PDF' },
  persona: { source_file: 'skillsets/pdf-toolkit.md', chars: 1234 },
  skills: [
    { slug: 'pdf-extract', status: 'installed' },
    { slug: 'pdf-merge', status: 'failed', reason: '下载失败：上游返回 502' },
    { slug: 'pdf-sign', status: 'already_present' },
  ],
}

function renderTab(handler: (url: string, init?: RequestInit) => Promise<Response>): void {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  vi.stubGlobal('fetch', vi.fn((input: RequestInfo | URL, init?: RequestInit) => (
    handler(String(input), init)
  )))
  render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <MarketTab />
      </QueryClientProvider>
    </MemoryRouter>,
  )
}

/**
 * 列表成功、安装走 installRes 的默认行为；多数用例共用。
 * `url` 刻意不用：分页/路由的断言不在这一层，分开写更清楚。
 */
function withInstall(installRes: () => Promise<Response>): (url: string, init?: RequestInit) => Promise<Response> {
  return (_url, init) => {
    if (init?.method === 'POST') return installRes()
    return Promise.resolve(jsonResponse(LIST))
  }
}

/**
 * 等某一张卡渲染出来。
 *
 * **必须在这里断言非 null 再返回**：`waitFor` 靠「回调抛异常」判定要不要重试，
 * 而 `querySelector` 找不到时只是返回 null —— 直接 `waitFor(() => cardOf(x))`
 * 会在第一帧就「成功」，错误拖到 `within(null)` 才炸，还指向错误的行。
 */
async function waitForCard(slug: string): Promise<HTMLElement> {
  return await waitFor(() => {
    const card = document.querySelector(`[data-slug="${slug}"]`)
    expect(card).not.toBeNull()
    return card as HTMLElement
  })
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

afterEach(() => {
  vi.unstubAllGlobals()
})

it('渲染上游给的条目，并显示上游地址与总数', async () => {
  renderTab(withInstall(() => Promise.resolve(jsonResponse(INSTALL_OK))))
  const card = await waitForCard('pdf-toolkit')

  expect(within(card).getByText('PDF 工具箱')).toBeInTheDocument()
  expect(within(card).getByText('上游给的说明。')).toBeInTheDocument()
  expect(within(card).getByText('场景 office')).toBeInTheDocument()
  expect(within(card).getByText('含 2 个技能')).toBeInTheDocument()
  expect(within(card).getByText('pdf-extract、pdf-merge')).toBeInTheDocument()
  // 上游地址必须露出来：这**不是内置列表**。
  expect(screen.getByText(/https:\/\/api\.skillhub\.cn/)).toBeInTheDocument()
  expect(screen.getByText(/第 1 页 · 共 3348 个/)).toBeInTheDocument()
  // 上游没给的字段不许编：卡片上只有名字、slug、技能数，
  // 没有任何评分/安装量的**数字**。这里查的是卡片区，
  // 因为页面顶部那句说明本来就写着「没有评分」。
  const cards = document.querySelectorAll('.experts-market-card')
  expect(Array.from(cards).some((card) => /评分|安装量|次下载|\d+(\.\d+)?\s*[★次]/.test(card.textContent ?? ''))).toBe(false)
})

it('上游没给说明就说没给，不拿 slug 顶替', async () => {
  renderTab(withInstall(() => Promise.resolve(jsonResponse(INSTALL_OK))))
  const card = await waitForCard('wechat-ops')

  expect(within(card).getByText('上游没给说明。没有说明就是没有，不拿 slug 顶替。')).toBeInTheDocument()
  // slug 仍然作为标识显示，但它没有占据说明的位置。
  expect(within(card).getByText('wechat-ops')).toBeInTheDocument()
})

it('已安装的条目标「已安装」且按钮禁用', async () => {
  renderTab(withInstall(() => Promise.resolve(jsonResponse(INSTALL_OK))))
  const card = await waitForCard('wechat-ops')

  // 「已安装」出现两次是有意的：徽标说明状态，按钮说明它不可再点。
  expect(within(card).getAllByText('已安装')).toHaveLength(2)
  expect(within(card).getByRole('button', { name: '已安装' })).toBeDisabled()
  // 已装的条目不给安装入口。
  expect(within(card).queryByRole('button', { name: '安装为我的专家' })).toBeNull()
  // 没装的条目才给安装按钮。
  const pending = await waitForCard('pdf-toolkit')
  expect(within(pending).getByRole('button', { name: '安装为我的专家' })).not.toBeDisabled()
})

it('安装后逐条显示三种技能状态、失败原因、人格来源与停用提示', async () => {
  renderTab(withInstall(() => Promise.resolve(jsonResponse(INSTALL_OK))))
  const card = await waitForCard('pdf-toolkit')
  fireEvent.click(within(card).getByRole('button', { name: '安装为我的专家' }))

  await waitFor(() => expect(within(card).getByText(/已装入专家「PDF 工具箱」/)).toBeInTheDocument())
  expect(within(card).getByText('已装入')).toBeInTheDocument()
  expect(within(card).getByText('本来就有')).toBeInTheDocument()
  expect(within(card).getByText('没装上')).toBeInTheDocument()
  // failed 的原因原样显示，不改写。
  expect(within(card).getByText('下载失败：上游返回 502')).toBeInTheDocument()
  expect(within(card).getByText(/skillsets\/pdf-toolkit\.md · 1234 字/)).toBeInTheDocument()
  // 停用是刻意的：必须写出来 + 给下一步。
  expect(within(card).getByText(/装进来的技能一律是停用的/)).toBeInTheDocument()
  expect(within(card).getByText(/去「技能包」页把它需要的那几个打开/)).toBeInTheDocument()
})

it('already_installed 时说清没有覆盖', async () => {
  renderTab(withInstall(() => Promise.resolve(jsonResponse({ ...INSTALL_OK, already_installed: true }))))
  const card = await waitForCard('pdf-toolkit')
  fireEvent.click(within(card).getByRole('button', { name: '安装为我的专家' }))

  await waitFor(() => expect(within(card).getByText(/已经装过了，这次没有覆盖它/)).toBeInTheDocument())
  expect(within(card).queryByText(/已装入专家/)).toBeNull()
})

it('安装失败时显示服务端原文与下一步，不静默', async () => {
  renderTab(withInstall(() => Promise.resolve(jsonResponse(
    { error: { code: 'hub_unavailable', detail: '上游市场连不上', next_step: '稍后重试。' } },
    503,
  ))))
  const card = await waitForCard('pdf-toolkit')
  fireEvent.click(within(card).getByRole('button', { name: '安装为我的专家' }))

  await waitFor(() => expect(within(card).getByRole('alert')).toHaveTextContent('上游市场连不上'))
  expect(within(card).getByText('稍后重试。')).toBeInTheDocument()
})

it('上游返回 0 条时说清是上游说的，不是没接通', async () => {
  renderTab(() => Promise.resolve(jsonResponse({ ...LIST, items: [], total: 0 })))
  await waitFor(() => expect(screen.getByText(/上游这一次返回了 0 个专家包/)).toBeInTheDocument())
  expect(screen.queryByText(/路由.*没有接通/)).toBeNull()
  expect(screen.queryByRole('alert')).toBeNull()
})

it('路由未接通（501）与上游连不上是两种说法', async () => {
  renderTab(() => Promise.resolve(jsonResponse(
    { error: { code: 'not_implemented', detail: '该路由尚未实现', next_step: '先实现它。' } },
    501,
  )))
  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('路由'))
  expect(screen.getByText(/后端这条路由.*还没有接通/)).toBeInTheDocument()
  expect(screen.queryByText(/上游这一次返回了 0 个/)).toBeNull()
})

it('上游连不上（503）显示错误原文与下一步，不画空列表', async () => {
  renderTab(() => Promise.resolve(jsonResponse(
    { error: { code: 'hub_unreachable', detail: '无法连接上游市场', next_step: '检查服务端出网。' } },
    503,
  )))
  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('无法连接上游市场'))
  expect(screen.getByText('检查服务端出网。')).toBeInTheDocument()
  expect(screen.queryByText(/上游这一次返回了 0 个/)).toBeNull()
  // 空列表一项都不画。
  expect(document.querySelectorAll('.experts-market-card')).toHaveLength(0)
})

it('卡片与列表两种看法都能切，且偏好被记住', async () => {
  // 同一页里两种版式：切换只改排法，不改内容 —— 断言的是那份列表的 data-view，
  // 因为条目仍然是同一批（同一个 data-slug 节点，不是两套组件）。
  renderTab(withInstall(() => Promise.resolve(jsonResponse(INSTALL_OK))))
  await waitForCard('pdf-toolkit')

  const list = document.querySelector('.experts-market-list')
  expect(list?.getAttribute('data-view')).toBe('card')

  fireEvent.click(screen.getByTestId('experts-market-view-list'))
  expect(list?.getAttribute('data-view')).toBe('list')
  expect(document.querySelectorAll('[data-slug="pdf-toolkit"]')).toHaveLength(1)
  // 记忆键与「我的专家」共用一个：在那边选过列表，这边不该又变回卡片。
  expect(localStorage.getItem('quill:experts:view')).toBe('list')

  fireEvent.click(screen.getByTestId('experts-market-view-card'))
  expect(list?.getAttribute('data-view')).toBe('card')
  expect(localStorage.getItem('quill:experts:view')).toBe('card')
})

it('装完那张卡横跨整行，结果说明不会挤在窄格里', async () => {
  renderTab(withInstall(() => Promise.resolve(jsonResponse(INSTALL_OK))))
  const card = await waitForCard('pdf-toolkit')
  expect(card.getAttribute('data-expanded')).toBeNull()

  fireEvent.click(within(card).getByRole('button', { name: '安装为我的专家' }))
  await waitFor(() => expect(card.getAttribute('data-expanded')).toBe('true'))
  // 仍然只有一张卡：一个包两条路径各画一遍，用户会以为装了两个。
  expect(document.querySelectorAll('[data-slug="pdf-toolkit"]')).toHaveLength(1)
})

it('上游没给总数时显示「共 ? 个」，分页控件仍然保留', async () => {
  renderTab(() => Promise.resolve(jsonResponse({ ...LIST, total: null })))
  await waitFor(() => expect(screen.getByText(/第 1 页 · 共 \? 个（上游没给总数/)).toBeInTheDocument())
  // 本页只有 2 条 < page_size，所以「下一页」禁用；但控件必须在，
  // 否则用户连「还有没有下一页」都不知道。
  expect(screen.getByRole('button', { name: '上一页' })).toBeDisabled()
  expect(screen.getByRole('button', { name: '下一页' })).toBeInTheDocument()
  expect(screen.getByRole('button', { name: '下一页' })).toBeDisabled()
})

it('装完重取回来的「已安装」不能把逐条安装结果顶掉', async () => {
  // 装完会失效市场列表重取，重取回来的那一项 installed 已经是 true。
  // 若界面先判 installed，刚出现的「哪个技能没装上、为什么」会被一个
  // 禁用按钮顶掉 —— 用户根本来不及看，那这份报告就等于没给。
  let installed = false
  renderTab((_url, init) => {
    if (init?.method === 'POST') {
      installed = true
      return Promise.resolve(jsonResponse(INSTALL_OK))
    }
    const body = installed
      ? { ...LIST, items: LIST.items.map((i) => (i.slug === 'pdf-toolkit' ? { ...i, installed: true } : i)) }
      : LIST
    return Promise.resolve(jsonResponse(body))
  })
  const card = await waitForCard('pdf-toolkit')
  fireEvent.click(within(card).getByRole('button', { name: '安装为我的专家' }))
  await waitFor(() => expect(within(card).getByText(/已装入专家「PDF 工具箱」/)).toBeInTheDocument())

  const after = await waitForCard('pdf-toolkit')
  await waitFor(() => expect(within(after).getAllByText('已安装').length).toBeGreaterThan(0))
  // 重取已完成（installed 变成 true），报告仍完整在屏幕上。
  expect(within(after).getByText('没装上')).toBeInTheDocument()
  expect(within(after).getByText('下载失败：上游返回 502')).toBeInTheDocument()
})