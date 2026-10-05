import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { ExpertsPage } from './ExpertsPage'

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

function renderPage(entry = '/experts'): void {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <MemoryRouter initialEntries={[entry]}>
      <QueryClientProvider client={client}>
        <ExpertsPage />
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

it('后端 501 时显示服务端中文原文，不白屏也不显示假数据', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(
    jsonResponse({ error: { code: 'not_implemented', detail: '该路由尚未实现', next_step: '先实现它。' } }, 501),
  ))
  renderPage()
  await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('该路由尚未实现'))
  expect(screen.getByRole('tab', { name: '我的专家' })).toBeInTheDocument()
  expect(screen.queryByText('还没有专家')).not.toBeInTheDocument()
})

it('未接通的 tab 如实标注状态与下一步', () => {
  renderPage('/experts?tab=market')
  expect(screen.getByText('未接通')).toBeInTheDocument()
  expect(screen.getByText(/市场数据来源/)).toBeInTheDocument()
})

it('列表区分有无人格的专家', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(jsonResponse({
    experts: [
      { id: 'a', owner: 'me', display_name: '甲', description: '写作', instructions: '你是甲', model: null, visibility: 'private', default_enabled: true, is_builtin: false },
      { id: 'b', owner: 'me', display_name: '乙', description: '', instructions: '', model: 'qwen3:8b', visibility: 'private', default_enabled: false, is_builtin: false },
    ],
  })))
  renderPage()
  await waitFor(() => expect(screen.getByText('已设置人格')).toBeInTheDocument())
  expect(screen.getByText('未设置人格')).toBeInTheDocument()
  expect(screen.getByText('跟随实例默认模型')).toBeInTheDocument()
  // 填了 model 的专家必须同时标出「暂未参与路由」，否则界面会让人以为已经在按专家切模型。
  expect(screen.getByText('qwen3:8b （已保存，暂未参与路由）')).toBeInTheDocument()
})

it('标识非法时禁用创建按钮', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(jsonResponse({ experts: [] })))
  renderPage()
  await waitFor(() => expect(screen.getByTestId('experts-new')).toBeInTheDocument())
  fireEvent.click(screen.getByTestId('experts-new'))
  const submit = screen.getByRole('button', { name: '创建专家' })
  const idInput = document.querySelector('input[name="id"]') as HTMLInputElement
  expect(submit).not.toBeDisabled()
  fireEvent.change(idInput, { target: { value: 'Bad Id' } })
  expect(submit).toBeDisabled()
  fireEvent.change(idInput, { target: { value: 'good-id' } })
  expect(submit).not.toBeDisabled()
})

const TWO_EXPERTS = {
  experts: [
    { id: 'a', owner: 'me', display_name: '甲', description: '写作', instructions: '你是甲', model: null, visibility: 'private', default_enabled: true, is_builtin: false, source_template: null },
    { id: 'b', owner: 'me', display_name: '乙', description: '', instructions: '', model: 'qwen3:8b', visibility: 'private', default_enabled: false, is_builtin: true, source_template: 'stock-assistant' },
  ],
}

it('工具栏按 Octop 顺序给全：计数、搜索、视图切换、刷新、新建、去内置专家', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(jsonResponse(TWO_EXPERTS)))
  renderPage()
  await waitFor(() => expect(screen.getByText('共 2 个专家')).toBeInTheDocument())
  expect(screen.getByLabelText('搜索专家')).toBeInTheDocument()
  expect(screen.getByRole('button', { name: '卡片' })).toHaveAttribute('aria-pressed', 'true')
  expect(screen.getByRole('button', { name: '表格' })).toHaveAttribute('aria-pressed', 'false')
  expect(screen.getByTestId('experts-refresh')).toBeInTheDocument()
  expect(screen.getAllByTestId('experts-new').length).toBeGreaterThan(0)
  expect(screen.getByRole('button', { name: '去内置专家' })).toBeInTheDocument()
})

it('「去内置专家」只切 tab，不另找接口', async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(TWO_EXPERTS))
  vi.stubGlobal('fetch', fetchMock)
  renderPage()
  await waitFor(() => expect(screen.getByRole('button', { name: '去内置专家' })).toBeInTheDocument())
  fireEvent.click(screen.getByRole('button', { name: '去内置专家' }))
  await waitFor(() => expect(screen.getByRole('tab', { name: '专家库' })).toHaveAttribute('aria-selected', 'true'))
  // 切过去的还是同一个专家列表接口：没有为这个按钮另开任何请求。
  for (const call of fetchMock.mock.calls) {
    expect(String(call[0])).toContain('/api/experts')
  }
})

it('搜索无结果与没有数据是两种空态', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(jsonResponse(TWO_EXPERTS)))
  renderPage()
  await waitFor(() => expect(screen.getByText('共 2 个专家')).toBeInTheDocument())
  fireEvent.change(screen.getByLabelText('搜索专家'), { target: { value: '不存在' } })
  expect(screen.getByText('没有匹配的专家')).toBeInTheDocument()
  expect(screen.queryByText('还没有专家')).not.toBeInTheDocument()
  // 有值才出现清除按钮
  fireEvent.click(screen.getByRole('button', { name: '清除' }))
  expect(screen.getByText('共 2 个专家')).toBeInTheDocument()
})

it('表格视图七列齐全，状态列只显示默认启用与否', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(jsonResponse(TWO_EXPERTS)))
  renderPage()
  await waitFor(() => expect(screen.getByRole('button', { name: '表格' })).toBeInTheDocument())
  fireEvent.click(screen.getByRole('button', { name: '表格' }))
  for (const header of ['名称', '专家 ID', '状态', '描述', '模型', '人格', '操作']) {
    expect(screen.getByRole('columnheader', { name: header })).toBeInTheDocument()
  }
  expect(localStorage.getItem('octop:experts-view')).toBe('table')
  expect(screen.getByText('默认启用')).toBeInTheDocument()
  expect(screen.getByText('未启用')).toBeInTheDocument()
  // 徽标照 Octop：内置 / 来自模板；空值一律破折号
  expect(screen.getByText('内置')).toBeInTheDocument()
  expect(screen.getByText('来自模板')).toBeInTheDocument()
  expect(screen.getAllByText('—').length).toBeGreaterThan(0)
  expect(screen.queryByRole('button', { name: '停止' })).not.toBeInTheDocument()
})

it('删除走二次确认，取消不发请求，确认后按软删口径提示', async () => {
  const fetchMock = vi.fn().mockImplementation((_url: string, init?: RequestInit) => (
    init?.method === 'DELETE'
      ? Promise.resolve(jsonResponse({ id: 'a', deleted: true, note: '已软删除：专家对所有用户（含属主）不可见，同名可再次创建。' }))
      : Promise.resolve(jsonResponse(TWO_EXPERTS))
  ))
  vi.stubGlobal('fetch', fetchMock)
  renderPage()
  await waitFor(() => expect(screen.getByText('共 2 个专家')).toBeInTheDocument())
  fireEvent.click(screen.getAllByRole('button', { name: '删除' })[0] as HTMLElement)
  const dialog = screen.getByRole('dialog')
  // 软删口径：不能写成 Octop 的「工作区目录也会被永久删除」
  expect(dialog).toHaveTextContent('软删除')
  expect(dialog).not.toHaveTextContent('工作区')
  const before = fetchMock.mock.calls.length
  fireEvent.click(within(dialog).getByRole('button', { name: '取消' }))
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  expect(fetchMock.mock.calls.length).toBe(before)

  fireEvent.click(screen.getAllByRole('button', { name: '删除' })[0] as HTMLElement)
  fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: '删除' }))
  await waitFor(() => expect(fetchMock.mock.calls.some((call) => (call[1] as RequestInit | undefined)?.method === 'DELETE')).toBe(true))
})

it('复制专家 ID 走剪贴板并给短暂反馈', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined)
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(jsonResponse(TWO_EXPERTS)))
  renderPage()
  await waitFor(() => expect(screen.getByText('共 2 个专家')).toBeInTheDocument())
  fireEvent.click(screen.getAllByRole('button', { name: '点击复制专家 ID' })[0] as HTMLElement)
  await waitFor(() => expect(writeText).toHaveBeenCalledWith('a'))
  expect(screen.getAllByText('已复制').length).toBeGreaterThan(0)
})
