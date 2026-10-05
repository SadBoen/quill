import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { ExpertsPage } from './ExpertsPage'

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

function expert(id: string, name: string, description = ''): Record<string, unknown> {
  return {
    id,
    owner: 'me',
    display_name: name,
    description,
    instructions: '',
    model: null,
    visibility: 'private',
    default_enabled: true,
    is_builtin: false,
    source_template: null,
  }
}

function team(id: string, name: string, leader: string, members: string[]): Record<string, unknown> {
  return {
    team_id: id,
    name,
    description: '',
    leader_id: leader,
    member_ids: members,
    created_at: 1,
    updated_at: 1,
  }
}

/** 两个接口都要接上：成员下拉和主持人下拉都吃 /api/experts。 */
function stubApi(teams: unknown[], experts: unknown[]): ReturnType<typeof vi.fn> {
  const fetchMock = vi.fn().mockImplementation((url: string) => {
    if (String(url).startsWith('/api/teams')) return Promise.resolve(jsonResponse({ teams }))
    if (String(url).startsWith('/api/experts')) return Promise.resolve(jsonResponse({ experts }))
    return Promise.resolve(jsonResponse({ code: 'not_found', message: '未找到' }, 404))
  })
  vi.stubGlobal('fetch', fetchMock)
  return fetchMock
}

/** 表单里的 label 包着 <small> 提示，getByLabelText 匹配不到全等文本，这里按 name 取。 */
function leaderSelect(): HTMLSelectElement {
  return document.querySelector('select[name="leader_id"]') as HTMLSelectElement
}

/** 空态引导和表单提交按钮都叫「创建团队」，所以表单内的查询一律收进这个作用域。 */
function createForm(): HTMLElement {
  return document.getElementById('teams-create-form') as HTMLElement
}

function openCreateForm(): void {
  fireEvent.click(screen.getAllByTestId('teams-new')[0] as HTMLElement)
}

function renderPage(): void {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <MemoryRouter initialEntries={['/experts?tab=team']}>
      <QueryClientProvider client={client}>
        <ExpertsPage />
      </QueryClientProvider>
    </MemoryRouter>,
  )
}

const THREE = [expert('a', '甲', '写作'), expert('b', '乙', '调研'), expert('c', '丙', '分析')]

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
  localStorage.clear()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

it('空态给三步说明，并如实说明「不会真的派工」', async () => {
  stubApi([], THREE)
  renderPage()
  await waitFor(() => expect(screen.getByText('还没有团队')).toBeInTheDocument())
  expect(screen.getByText('什么是专家团队？')).toBeInTheDocument()
  expect(screen.getByText('怎么创建')).toBeInTheDocument()
  expect(screen.getByText('现在能拿它做什么')).toBeInTheDocument()
  expect(screen.getByText(/还不会真的派工/)).toBeInTheDocument()
  // Octop 原文承诺「成员气泡会出现在同一房间」，quill 没有这个能力，不能照抄。
  expect(screen.queryByText(/成员气泡/)).not.toBeInTheDocument()
  // 也没有任何可以点的派工入口。
  expect(screen.queryByRole('button', { name: /派工|启动|开始/ })).not.toBeInTheDocument()
  // 空态只有引导区这一个「创建团队」（工具栏要等有团队才出现）。
  expect(screen.getAllByTestId('teams-new')).toHaveLength(1)
  expect(screen.getByRole('button', { name: '去内置专家' })).toBeInTheDocument()
})

it('专家不够三名时禁用创建，并说明原因', async () => {
  stubApi([], [expert('a', '甲'), expert('b', '乙')])
  renderPage()
  await waitFor(() => expect(screen.getByText('还没有团队')).toBeInTheDocument())
  // 一名主持人 + 两名成员才凑得齐，只有两名专家时按钮必须挡住。
  expect(screen.getByTestId('teams-new')).toBeDisabled()
  expect(screen.getByTestId('teams-new')).toHaveAttribute('title', '至少选择两名专家。')
})

it('工具栏按 Octop 顺序给全：计数、搜索、刷新、创建团队', async () => {
  stubApi([team('t1', '写作组', 'a', ['b', 'c']), team('t2', '调研组', 'b', ['a', 'c'])], THREE)
  renderPage()
  await waitFor(() => expect(screen.getByText('共 2 个团队')).toBeInTheDocument())
  expect(screen.getByLabelText('搜索团队')).toBeInTheDocument()
  expect(screen.getByTestId('teams-refresh')).toBeInTheDocument()
  expect(screen.getByTestId('teams-new')).toBeInTheDocument()
  // 团队 tab 没有卡片/表格切换，Octop 的团队工具栏也没有。
  expect(screen.queryByRole('button', { name: '卡片' })).not.toBeInTheDocument()
})

it('卡片分清主持人和成员，成员数带上下限，超出折叠成 +N', async () => {
  const many = ['m1', 'm2', 'm3', 'm4', 'm5', 'm6', 'm7', 'm8'].map((id) => expert(id, `成员${id}`))
  stubApi(
    [team('big', '大组', 'a', many.map((item) => String(item.id)))],
    [expert('a', '甲', '主持'), ...many],
  )
  renderPage()
  await waitFor(() => expect(screen.getByText('共 1 个团队')).toBeInTheDocument())
  const card = document.querySelector('article[data-team="big"]') as HTMLElement
  expect(within(card).getByText('主持人')).toBeInTheDocument()
  expect(within(card).getByText('成员')).toBeInTheDocument()
  expect(within(card).getByText('8 名（限 2~8）')).toBeInTheDocument()
  // MAX_VISIBLE_MEMBERS = 6（Octop TeamCard.tsx:56），多出来两个并成 +2
  expect(within(card).getByText('+2')).toBeInTheDocument()
  expect(within(card).getByText('只记账，未派工')).toBeInTheDocument()
  // 只留编辑与删除两个操作，没有启动/停止/重载/工作区/团队记忆。
  expect(within(card).getByRole('button', { name: '编辑' })).toBeInTheDocument()
  expect(within(card).getByRole('button', { name: '删除' })).toBeInTheDocument()
  for (const label of ['停止', '重载', '工作区', '团队记忆', '渠道']) {
    expect(screen.queryByRole('button', { name: new RegExp(label) })).not.toBeInTheDocument()
  }
})

it('成员不够两名时挡住提交', async () => {
  stubApi([], THREE)
  renderPage()
  await waitFor(() => expect(screen.getAllByTestId('teams-new')[0]).toBeInTheDocument())
  openCreateForm()
  const form = createForm()
  const submit = within(form).getByRole('button', { name: '创建团队' })
  expect(submit).toBeDisabled()
  fireEvent.change(within(form).getByRole('searchbox', { name: '搜索专家' }), { target: { value: '甲' } })
  fireEvent.click(within(form).getByRole('button', { name: /甲/ }))
  expect(within(form).getByText('已选 1 人')).toBeInTheDocument()
  expect(submit).toBeDisabled()
  // 换搜索词再选一个，够两人才放行。
  fireEvent.change(within(form).getByRole('searchbox', { name: '搜索专家' }), { target: { value: '丙' } })
  fireEvent.click(within(form).getByRole('button', { name: /丙/ }))
  expect(within(form).getByText('已选 2 人')).toBeInTheDocument()
  expect(submit).not.toBeDisabled()
})

it('主持人不能同时被选为成员', async () => {
  stubApi([], THREE)
  renderPage()
  await waitFor(() => expect(screen.getAllByTestId('teams-new')[0]).toBeInTheDocument())
  openCreateForm()
  const form = createForm()
  fireEvent.change(leaderSelect(), { target: { value: 'a' } })
  fireEvent.change(within(form).getByRole('searchbox', { name: '搜索专家' }), { target: { value: '甲' } })
  // 甲已经是主持人，候选里不该再出现。
  expect(within(form).queryByRole('button', { name: /甲/ })).not.toBeInTheDocument()
  fireEvent.change(within(form).getByRole('searchbox', { name: '搜索专家' }), { target: { value: '乙' } })
  fireEvent.click(within(form).getByRole('button', { name: /乙/ }))
  expect(within(form).getByText('已选 1 人')).toBeInTheDocument()
  // 把主持人换成乙，已选的乙必须被摘掉，否则提交会被后端 400。
  fireEvent.change(leaderSelect(), { target: { value: 'b' } })
  expect(within(form).getByText('已选 0 人')).toBeInTheDocument()
})

it('编辑时锁定团队标识', async () => {
  stubApi([team('locked-id', '写作组', 'a', ['b', 'c'])], THREE)
  renderPage()
  await waitFor(() => expect(screen.getByText('共 1 个团队')).toBeInTheDocument())
  fireEvent.click(screen.getByRole('button', { name: '编辑' }))
  const idInput = document.querySelector('input[name="team_id"]') as HTMLInputElement
  expect(idInput).toBeDisabled()
  expect(idInput.value).toBe('locked-id')
  expect(screen.getByText('标识创建后不可改：它就是这个团队的地址。')).toBeInTheDocument()
  expect(screen.getByRole('button', { name: '保存修改' })).toBeInTheDocument()
})

it('创建时按契约发 member_ids，编辑走 PATCH', async () => {
  const fetchMock = stubApi([], THREE)
  renderPage()
  await waitFor(() => expect(screen.getAllByTestId('teams-new')[0]).toBeInTheDocument())
  fireEvent.click(screen.getAllByTestId('teams-new')[0] as HTMLElement)
  const form = createForm()
  const idInput = form.querySelector('input[name="team_id"]') as HTMLInputElement
  fireEvent.change(idInput, { target: { value: 'Bad Id' } })
  expect(within(form).getByRole('button', { name: '创建团队' })).toBeDisabled()
  fireEvent.change(idInput, { target: { value: 'writing-team' } })
  fireEvent.change(form.querySelector('input[name="team_name"]') as HTMLInputElement, { target: { value: '写作组' } })
  fireEvent.change(leaderSelect(), { target: { value: 'a' } })
  fireEvent.change(within(form).getByRole('searchbox', { name: '搜索专家' }), { target: { value: '乙' } })
  fireEvent.click(within(form).getByRole('button', { name: /乙/ }))
  fireEvent.change(within(form).getByRole('searchbox', { name: '搜索专家' }), { target: { value: '丙' } })
  fireEvent.click(within(form).getByRole('button', { name: /丙/ }))
  fireEvent.click(within(form).getByRole('button', { name: '创建团队' }))

  await waitFor(() => expect(fetchMock.mock.calls.some((call) => (call[1] as RequestInit | undefined)?.method === 'POST')).toBe(true))
  const post = fetchMock.mock.calls.find((call) => (call[1] as RequestInit | undefined)?.method === 'POST') as [string, RequestInit]
  expect(String(post[0])).toBe('/api/teams')
  expect(JSON.parse(String(post[1].body))).toEqual({
    team_id: 'writing-team',
    name: '写作组',
    description: null,
    leader_id: 'a',
    member_ids: ['b', 'c'],
  })
})

it('后端报错时显示服务端中文原文', async () => {  // 列表先正常返回空，创建才 409：否则整个 tab 会卡在加载失败态，看不到错误原文。
  const fetchMock = vi.fn().mockImplementation((url: string, init?: RequestInit) => {
    if (init?.method === 'POST') {
      return Promise.resolve(jsonResponse({ error: { code: 'team_id_taken', detail: '这个团队标识已被占用。', next_step: '换一个标识。' } }, 409))
    }
    if (String(url).startsWith('/api/teams')) return Promise.resolve(jsonResponse({ teams: [] }))
    return Promise.resolve(jsonResponse({ experts: THREE }))
  })
  vi.stubGlobal('fetch', fetchMock)
  renderPage()
  await waitFor(() => expect(screen.getAllByTestId('teams-new')[0]).toBeInTheDocument())
  openCreateForm()
  const form = createForm()
  fireEvent.change(form.querySelector('input[name="team_id"]') as HTMLInputElement, { target: { value: 'dup' } })
  fireEvent.change(form.querySelector('input[name="team_name"]') as HTMLInputElement, { target: { value: '重名组' } })
  fireEvent.change(leaderSelect(), { target: { value: 'a' } })
  fireEvent.change(within(form).getByRole('searchbox', { name: '搜索专家' }), { target: { value: '乙' } })
  fireEvent.click(within(form).getByRole('button', { name: /乙/ }))
  fireEvent.change(within(form).getByRole('searchbox', { name: '搜索专家' }), { target: { value: '丙' } })
  fireEvent.click(within(form).getByRole('button', { name: /丙/ }))
  fireEvent.click(within(form).getByRole('button', { name: '创建团队' }))
  await waitFor(() => expect(screen.getAllByRole('alert').some((node) => node.textContent?.includes('这个团队标识已被占用'))).toBe(true))
})

it('删除走二次确认，取消不发请求', async () => {
  const fetchMock = stubApi([team('t1', '写作组', 'a', ['b', 'c'])], THREE)
  renderPage()
  await waitFor(() => expect(screen.getByText('共 1 个团队')).toBeInTheDocument())
  fireEvent.click(screen.getByRole('button', { name: '删除' }))
  const dialog = screen.getByRole('dialog')
  // 删的是团队名单，成员专家本身不受影响 —— 不能写成 Octop 的「工作区目录也会被永久删除」。
  expect(dialog).toHaveTextContent('成员专家本身不受影响')
  expect(dialog).not.toHaveTextContent('工作区')
  const before = fetchMock.mock.calls.length
  fireEvent.click(within(dialog).getByRole('button', { name: '取消' }))
  expect(fetchMock.mock.calls.length).toBe(before)
  fireEvent.click(screen.getByRole('button', { name: '删除' }))
  fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: '删除' }))
  await waitFor(() => expect(fetchMock.mock.calls.some((call) => (call[1] as RequestInit | undefined)?.method === 'DELETE')).toBe(true))
})

/** 往选择器里按顺序勾选若干成员。 */
function pickMembers(form: HTMLElement, names: string[]): void {
  const search = within(form).getByRole('searchbox', { name: '搜索专家' })
  for (const name of names) {
    fireEvent.change(search, { target: { value: name } })
    fireEvent.click(within(form).getByRole('button', { name: new RegExp(name) }))
  }
}

function fillDraft(form: HTMLElement, id: string, name: string, leader: string): void {
  fireEvent.change(form.querySelector('input[name="team_id"]') as HTMLInputElement, { target: { value: id } })
  fireEvent.change(form.querySelector('input[name="team_name"]') as HTMLInputElement, { target: { value: name } })
  fireEvent.change(leaderSelect(), { target: { value: leader } })
}

function callWithMethod(fetchMock: ReturnType<typeof vi.fn>, method: string): [string, RequestInit] {
  return fetchMock.mock.calls.find((call) => (call[1] as RequestInit | undefined)?.method === method) as [string, RequestInit]
}

it('member_ids 提交前按 id 升序，与勾选顺序无关', async () => {
  // 契约要求 member_ids 升序；选择器按勾选顺序收集，先勾 c 再勾 b 就是 ['c','b']。
  const fetchMock = stubApi([], THREE)
  renderPage()
  await waitFor(() => expect(screen.getAllByTestId('teams-new')[0]).toBeInTheDocument())
  openCreateForm()
  const form = createForm()
  fillDraft(form, 'writing-team', '写作组', 'a')
  pickMembers(form, ['丙', '乙'])
  expect(within(form).getByText('已选 2 人')).toBeInTheDocument()
  fireEvent.click(within(form).getByRole('button', { name: '创建团队' }))
  await waitFor(() => expect(fetchMock.mock.calls.some((call) => (call[1] as RequestInit | undefined)?.method === 'POST')).toBe(true))
  expect(JSON.parse(String(callWithMethod(fetchMock, 'POST')[1].body)).member_ids).toEqual(['b', 'c'])
})

it('编辑保存走 PATCH，且请求体不带 team_id', async () => {
  const fetchMock = stubApi([team('t1', '写作组', 'a', ['b', 'c'])], THREE)
  renderPage()
  await waitFor(() => expect(screen.getByText('共 1 个团队')).toBeInTheDocument())
  fireEvent.click(screen.getByRole('button', { name: '编辑' }))
  const form = document.getElementById('teams-form-t1') as HTMLElement
  fireEvent.change(form.querySelector('input[name="team_name"]') as HTMLInputElement, { target: { value: '写作组（改）' } })
  fireEvent.click(within(form).getByRole('button', { name: '保存修改' }))
  await waitFor(() => expect(fetchMock.mock.calls.some((call) => (call[1] as RequestInit | undefined)?.method === 'PATCH')).toBe(true))
  const patch = callWithMethod(fetchMock, 'PATCH')
  expect(String(patch[0])).toBe('/api/teams/t1')
  const body = JSON.parse(String(patch[1].body))
  expect(body).not.toHaveProperty('team_id')
  expect(body.name).toBe('写作组（改）')
  expect(body.member_ids).toEqual(['b', 'c'])
})

it('描述清空后提交 null，与后端「显式 null = 清除」一致', async () => {
  const withDescription = { ...team('t1', '写作组', 'a', ['b', 'c']), description: '旧描述' }
  const fetchMock = stubApi([withDescription], THREE)
  renderPage()
  await waitFor(() => expect(screen.getByText('共 1 个团队')).toBeInTheDocument())
  fireEvent.click(screen.getByRole('button', { name: '编辑' }))
  const form = document.getElementById('teams-form-t1') as HTMLElement
  const textarea = form.querySelector('textarea[name="team_description"]') as HTMLTextAreaElement
  expect(textarea.value).toBe('旧描述')
  fireEvent.change(textarea, { target: { value: '' } })
  fireEvent.click(within(form).getByRole('button', { name: '保存修改' }))
  await waitFor(() => expect(fetchMock.mock.calls.some((call) => (call[1] as RequestInit | undefined)?.method === 'PATCH')).toBe(true))
  expect(JSON.parse(String(callWithMethod(fetchMock, 'PATCH')[1].body)).description).toBeNull()
})

it('选满八名后挡住第九名，并说明原因', async () => {
  const many = ['m1', 'm2', 'm3', 'm4', 'm5', 'm6', 'm7', 'm8', 'm9'].map((id) => expert(id, `成员${id}`))
  stubApi([], [expert('a', '甲', '主持'), ...many])
  renderPage()
  await waitFor(() => expect(screen.getAllByTestId('teams-new')[0]).toBeInTheDocument())
  openCreateForm()
  const form = createForm()
  fillDraft(form, 'big-team', '大组', 'a')
  const search = within(form).getByRole('searchbox', { name: '搜索专家' })
  for (const id of ['m1', 'm2', 'm3', 'm4', 'm5', 'm6', 'm7', 'm8']) {
    fireEvent.change(search, { target: { value: `成员${id}` } })
    fireEvent.click(within(form).getByRole('button', { name: new RegExp(`成员${id}`) }))
  }
  expect(within(form).getByText('已选 8 人')).toBeInTheDocument()
  fireEvent.change(search, { target: { value: '成员m9' } })
  const ninth = within(form).getByRole('button', { name: /成员m9/ })
  expect(ninth).toBeDisabled()
  expect(ninth).toHaveAttribute('title', '最多选择八名专家，先取消一名才能再选。')
})

it('搜索能命中成员显示名，空态说的是「团队」不是「专家」', async () => {
  stubApi([team('t1', '写作组', 'a', ['b', 'c']), team('t2', '调研组', 'b', ['a', 'c'])], THREE)
  renderPage()
  await waitFor(() => expect(screen.getByText('共 2 个团队')).toBeInTheDocument())
  const search = screen.getByLabelText('搜索团队')
  // 按成员显示名找：乙 只在 t1 的成员里（t2 的成员是 甲/丙）。
  fireEvent.change(search, { target: { value: '乙' } })
  expect(screen.getByText('共 1 个团队')).toBeInTheDocument()
  expect(screen.getByText('写作组')).toBeInTheDocument()
  expect(screen.queryByText('调研组')).not.toBeInTheDocument()
  // 搜不到时不能出现「没有匹配的专家」——这里是团队列表。
  fireEvent.change(search, { target: { value: '不存在的团队' } })
  expect(screen.getByText('没有匹配的团队')).toBeInTheDocument()
  expect(screen.queryByText('没有匹配的专家')).not.toBeInTheDocument()
})

it('成员已从专家列表消失时如实说明，不静默少画一个头像', async () => {
  stubApi([team('t1', '写作组', 'a', ['b', 'ghost'])], THREE)
  renderPage()
  await waitFor(() => expect(screen.getByText('共 1 个团队')).toBeInTheDocument())
  expect(screen.getByText('有 1 名成员已不在专家列表里，这里只显示了还找得到的那几个。')).toBeInTheDocument()
})
