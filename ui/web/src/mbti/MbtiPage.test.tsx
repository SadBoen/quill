import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { MbtiPage } from './MbtiPage'
import { firstUnanswered, isComplete } from './api'

/**
 * 人格页的判据。
 *
 * 三件事会被真正弄坏，且都不会自己报错：
 *
 * 1. **答不完时不能静默出结果。** 用户漏题时若照提交，后端会拿 20 题算一个
 *    类型出来，而界面显示「28 题已答 20」—— 用户以为自己答完了。
 *    所以「还有 N 题没答」必须同时给出**并跳到第一道没答的题**。
 * 2. **「应用到专家」必须点名。** 后端已经拒绝空 expert_id，前端也不该
 *    把按钮点得动 —— 否则用户点一下得到一句报错。
 * 3. **重复应用不增长。** 这是后端保证的，前端要如实显示「已应用在谁身上」，
 *    否则用户会以为点第二遍是叠加了两套人格。
 */

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

function serverError(status: number, detail: string): Response {
  return json({ error: { code: 'bad_request', detail, next_step: '改一下再来' } }, status)
}

const QUESTIONS = Array.from({ length: 28 }, (_, i) => ({
  id: i + 1,
  dimension: ['EI', 'SN', 'TF', 'JP'][i % 4],
  question: `第 ${i + 1} 题`,
  option_a: `选 A ${i + 1}`,
  option_b: `选 B ${i + 1}`,
}))

const PROFILE = {
  code: 'ESTJ',
  name: '总经理',
  nickname: '尺子姐',
  summary: '出色的管理者',
  descriptors: '管理者、高效、果断、负责',
  color: '#B91C1C',
  symbol: '⚖',
  dimensions: {
    ei: ['E', 74],
    sn: ['S', 76],
    tf: ['T', 78],
    jp: ['J', 82],
  },
  behavior: {
    answerStyle: '清晰直接',
    casualChat: '友善但偏任务导向',
    conflict: '果断处理问题',
    creativity: '优化流程',
    emotion: '用行动表达支持',
    planning: '创建高效工作流',
  },
}

const RESULT = {
  row_id: 7,
  code: 'ESTJ',
  dimensions: {
    ei: ['E', 85],
    sn: ['S', 85],
    tf: ['T', 85],
    jp: ['J', 85],
  },
  applied_expert_id: null,
  created_at: 1_700_000_000_000,
  profile: PROFILE,
}

const EXPERTS = [
  {
    id: 'keeper',
    owner: 'x',
    display_name: '守规矩的',
    description: '',
    instructions: '先问口径。',
    model: null,
    visibility: 'private',
    default_enabled: true,
    is_builtin: false,
    is_general: false,
    source_template: null,
  },
]

interface Fetched {
  url: string
  body: unknown
}

/** 路由表：默认成功，按 URL 覆盖。 */
function stubFetch(
  overrides: Record<string, () => Response | Promise<Response>> = {},
): Fetched[] {
  const seen: Fetched[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      const body = init?.body ? JSON.parse(String(init.body)) : undefined
      seen.push({ url, body })
      for (const [frag, make] of Object.entries(overrides)) {
        if (url.includes(frag)) return make()
      }
      if (url.includes('/api/mbti/questions')) {
        return json({ questions: QUESTIONS, min_answers: 20 })
      }
      if (url.includes('/api/mbti/history')) return json({ history: [], current: null, keep: 20 })
      if (url.includes('/api/mbti/types')) return json({ types: [PROFILE] })
      if (url.includes('/api/mbti/test')) return json({ result: RESULT, profile: PROFILE })
      if (url.includes('/api/mbti/apply')) return json({ ok: true })
      if (url.includes('/api/experts')) return json({ experts: EXPERTS })
      return json({})
    }),
  )
  return seen
}

function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <MemoryRouter>
      <QueryClientProvider client={qc}>
        <MbtiPage />
      </QueryClientProvider>
    </MemoryRouter>,
  )
}

beforeEach(async () => {
  await i18n.changeLanguage('zh')
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('人格页', () => {
  it('没测过时明说没测过，并给一个能点的入口', async () => {
    stubFetch()
    mount()
    await waitFor(() => expect(screen.getByText(/还没测过/)).toBeTruthy())
    const start = screen.getByRole('button', { name: '开始测评' })
    expect(start).toBeTruthy()
  })

  it('有结果时画出类型名、四条光谱与六项说话风格', async () => {
    stubFetch({
      '/api/mbti/history': () => json({ history: [RESULT], current: RESULT, keep: 20 }),
    })
    mount()
    await waitFor(() => expect(screen.getByText('ESTJ')).toBeTruthy())
    expect(screen.getByText(/总经理/)).toBeTruthy()
    // 四根轴各一行
    for (const label of ['EI', 'SN', 'TF', 'JP']) {
      expect(screen.getByText(label)).toBeTruthy()
    }
    // 六项行为都出现
    expect(screen.getByText('清晰直接')).toBeTruthy()
    expect(screen.getByText('创建高效工作流')).toBeTruthy()
  })

  it('漏题时点交卷要说出漏了几题，并跳到第一道没答的', async () => {
    const seen = stubFetch()
    mount()
    await waitFor(() => expect(screen.getByRole('button', { name: '开始测评' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: '开始测评' }))

    const current = await screen.findByRole('heading', { name: '第 1 题' })
    expect(current).toBeTruthy()
    // 只答第一题，其余不答
    fireEvent.click(screen.getByRole('button', { name: /选 A 1/ }))

    fireEvent.click(screen.getByRole('button', { name: /交卷/ }))
    await waitFor(() => expect(screen.getByText(/还有 27 题没答/)).toBeTruthy())
    expect(screen.getByRole('heading', { name: '第 2 题' })).toBeTruthy()

    // 关键：一次提交请求都不能发出去。
    expect(seen.filter((s) => s.url.includes('/api/mbti/test')).length).toBe(0)
  })

  it('答够就能出结果，出完显示类型与档条', async () => {
    stubFetch()
    mount()
    await waitFor(() => expect(screen.getByRole('button', { name: '开始测评' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: '开始测评' }))

    // 一路选 A 到底
    for (let i = 1; i <= 28; i += 1) {
      const q = await screen.findByRole('heading', { name: `第 ${i} 题` })
      expect(q).toBeTruthy()
      fireEvent.click(screen.getByRole('button', { name: `选 A ${i}` }))
      if (i < 28) fireEvent.click(screen.getByRole('button', { name: '下一题' }))
    }

    fireEvent.click(await screen.findByRole('button', { name: '看结果' }))
    await waitFor(() => expect(screen.getByText('ESTJ')).toBeTruthy())
    expect(screen.getByRole('button', { name: '重新测一次' })).toBeTruthy()
  })

  it('没选专家时「应用」点不动 —— 宁可不给一个会报错的按钮', async () => {
    const seen = stubFetch({
      '/api/mbti/history': () => json({ history: [RESULT], current: RESULT, keep: 20 }),
    })
    mount()
    const apply = await screen.findByRole('button', { name: '应用这个人格' })
    expect((apply as HTMLButtonElement).disabled).toBe(true)

    fireEvent.change(screen.getByLabelText('写到哪个专家'), { target: { value: 'keeper' } })
    await waitFor(() =>
      expect((screen.getByRole('button', { name: '应用这个人格' }) as HTMLButtonElement).disabled).toBe(
        false,
      ),
    )
    fireEvent.click(screen.getByRole('button', { name: '应用这个人格' }))
    await waitFor(() =>
      expect(seen.filter((s) => s.url.includes('/api/mbti/apply')).length).toBe(1),
    )
    expect(seen.find((s) => s.url.includes('/api/mbti/apply'))?.body).toEqual({
      row_id: 7,
      expert_id: 'keeper',
      language: 'zh',
    })
  })

  it('已经应用过的结果要显示用在谁身上', async () => {
    stubFetch({
      '/api/mbti/history': () =>
        json({
          history: [{ ...RESULT, applied_expert_id: 'keeper' }],
          current: { ...RESULT, applied_expert_id: 'keeper' },
          keep: 20,
        }),
    })
    mount()
    await waitFor(() =>
      expect(screen.getByText(/当前已应用在专家 keeper 上/)).toBeTruthy(),
    )
  })

  it('档案缺失要说明白，不画空壳', async () => {
    stubFetch({
      '/api/mbti/history': () =>
        json({
          history: [{ ...RESULT, profile: null }],
          current: { ...RESULT, profile: null },
          keep: 20,
        }),
    })
    mount()
    await waitFor(() => expect(screen.getByText(/没有对应的档案/)).toBeTruthy())
  })

  it('后端说答得不够时，把它的原话显示出来', async () => {
    stubFetch({
      '/api/mbti/test': () => serverError(400, '至少要答 20 题才出结果，目前只答了 12 题。'),
    })
    mount()
    await waitFor(() => expect(screen.getByRole('button', { name: '开始测评' })).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: '开始测评' }))
    for (let i = 1; i <= 28; i += 1) {
      await screen.findByRole('heading', { name: `第 ${i} 题` })
      fireEvent.click(screen.getByRole('button', { name: `选 A ${i}` }))
      if (i < 28) fireEvent.click(screen.getByRole('button', { name: '下一题' }))
    }
    fireEvent.click(await screen.findByRole('button', { name: '看结果' }))
    await waitFor(() => expect(screen.getByText(/至少要答 20 题/)).toBeTruthy())
  })

  it('历史超过一次才列出来，且说清保留多少条', async () => {
    stubFetch({
      '/api/mbti/history': () =>
        json({
          history: [RESULT, { ...RESULT, row_id: 6, code: 'INFP' }],
          current: RESULT,
          keep: 20,
        }),
    })
    mount()
    await waitFor(() => expect(screen.getByText(/最近 20 次/)).toBeTruthy())
    expect(screen.getByText(/INFP/)).toBeTruthy()
  })
})

describe('纯函数', () => {
  it('第一道没答的题号找得出来', () => {
    const answered: Record<string, 'A' | 'B'> = { '1': 'A', '2': 'B' }
    expect(firstUnanswered(QUESTIONS, answered)).toBe(2)
    const all: Record<string, 'A' | 'B'> = {}
    for (const q of QUESTIONS) all[String(q.id)] = 'A'
    expect(firstUnanswered(QUESTIONS, all)).toBe(-1)
  })

  it('答够门槛才算完 —— 门槛来自接口，不写死 28', () => {
    const mark = (n: number): Record<string, 'A' | 'B'> =>
      Object.fromEntries(QUESTIONS.slice(0, n).map((q) => [String(q.id), 'A' as const]))
    expect(isComplete(QUESTIONS, mark(20), 20)).toBe(true)
    expect(isComplete(QUESTIONS, mark(19), 20)).toBe(false)
    // 题库为空时不该判成「答完了」
    expect(isComplete([], {}, 20)).toBe(false)
  })
})