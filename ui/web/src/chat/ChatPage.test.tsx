import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
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

it('新建会话时发消息失败，必须把服务端写的「下一步」显示出来（ISSUE-035）', async () => {
  // 后端在 503 里把可执行的下一步都写好了：
  //   detail: 'request (9845 tokens) exceeds the available context size (8192 tokens)'
  //   next_step: '……调大 QUILL_LLM_MAX_CONTEXT_TOKENS，或减少这一轮挂着的技能……'
  // 也就是说**服务端尽到了责任**，前端把它吞掉才是问题所在。
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/api/experts')) return jsonResponse(EXPERTS)
    // 顺序有讲究：发消息的路径是 /api/sessions/{id}/messages，建会话是 /api/sessions。
    if (url.includes('/messages/stream')) {
      return jsonResponse(
        {
          error: {
            code: 'provider_rejected',
            detail:
              '模型调用失败：LLM 服务返回 HTTP 400：request (9845 tokens) exceeds the available context size (8192 tokens)',
            next_step: '下一步：调大 QUILL_LLM_MAX_CONTEXT_TOKENS，或减少这一轮挂着的技能。',
          },
        },
        503,
      )
    }
    if (url.endsWith('/api/sessions')) {
      return jsonResponse({ id: 'NEWSESSION01', title: '新对话', expert_id: 'general' })
    }
    if (url.includes('/api/sessions?')) return jsonResponse({ sessions: [] })
    if (url.includes('/healthz')) return jsonResponse({ llm: {} })
    return jsonResponse({ messages: [] })
  })

  renderPage('/chat', fetchMock)
  await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument())

  const box = await screen.findByRole('textbox', { name: '消息' })
  fireEvent.change(box, { target: { value: '1+1 等于几？' } })
  fireEvent.submit(box.closest('form') as HTMLFormElement)

  // 过去这里一直是失败的：catch 把 notice 记到「还没有 sessionId」上，
  // 而可见性是按 navigate 之后的**新**会话 id 过滤的，对不上就永远不渲染。
  await waitFor(() => expect(fetchMock.mock.calls.map((c) => String(c[0])).some((u) => u.includes('/messages/stream'))).toBe(true))
  await waitFor(() => expect(screen.getAllByRole('alert').length).toBeGreaterThan(0))
  // 这个 mock 对 GET/POST 一视同仁（fetch 拿不到 init 里的 method），
  // 所以拉历史也会拿到 503、可能多出一条横幅。这里断言的是
  // 「服务端写的下一步必须让人看得见」，不是「横幅只有一条」。
  const alert = screen
    .getAllByRole('alert')
    .map((n) => n.textContent ?? '')
    .join('\n')
  expect(alert).toContain('exceeds the available context size')
  // 项目硬规矩：错误必须带「下一步：…」
  expect(alert).toContain('下一步：')
})

it('发送失败后仍能看到自己发出去的那条消息（ISSUE-039）', async () => {
  // 实测过：503 之后服务端 `/messages` 里**已经有**那条 user 消息
  // （status=complete），但界面停在欢迎屏上，用户那条消息看不见 ——
  // 用户会以为自己没发出去，重发一遍或者干脆以为 quill 坏了。
  let sent = 0
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/api/experts')) return jsonResponse(EXPERTS)
    if (url.endsWith('/messages/stream')) {
      sent += 1
      // 第一次是发送（失败），之后拉历史都应该拿到那条已落库的消息
      return jsonResponse(
        {
          error: {
            code: 'tool_loop_exhausted',
            detail: '模型连续 4 轮都在请求调用工具，没有给出正文。',
            next_step: '下一步：换个更直接的问法（把要什么一次说清楚）。',
          },
        },
        503,
      )
    }
    if (url.includes('/messages')) {
      return jsonResponse({
        messages: [
          {
            id: 'm1',
            seq: 1,
            role: 'user',
            status: 'complete',
            content: '帮我算一下 1+1',
            reasoning: '',
            input_tokens: 0,
            output_tokens: 0,
            turn_ms: 0,
            error_code: '',
            created_at: '2026-10-06T08:00:00Z',
          },
        ],
      })
    }
    if (url.endsWith('/api/sessions')) return jsonResponse({ id: 'NEWSESSION01', title: '新对话', expert_id: 'general' })
    if (url.includes('/api/sessions?')) return jsonResponse({ sessions: [] })
    if (url.includes('/healthz')) return jsonResponse({ llm: {} })
    return jsonResponse({})
  })

  renderPage('/chat', fetchMock)
  await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument())

  const box = await screen.findByRole('textbox', { name: '消息' })
  fireEvent.change(box, { target: { value: '帮我算一下 1+1' } })
  fireEvent.submit(box.closest('form') as HTMLFormElement)

  // 失败横幅照样要显示
  await waitFor(() => expect(screen.getByRole('alert')).toBeInTheDocument())
  // 这一条原来写的是**裸断言** `expect(sent).toBeGreaterThan(0)`。
  // 上面那个 waitFor 只要界面出现任何一条 alert 就满足 —— 而 alert 完全可能
  // 先由别的请求（比如 experts / healthz）触发，那一刻 POST /messages
  // 还没发出去。于是这条断言就成了一个竞态：机器慢、并发跑全套时更容易红。
  // 实测：Linux 上同一份代码第一轮全绿、第二轮红，单独跑又永远绿。
  // 它断言的是「发送确实发生过」，那就等它发生，而不是假设它已经发生。
  await waitFor(() => expect(sent).toBeGreaterThan(0))
  // 关键：欢迎屏不能再留着把消息挡住 / 抹掉。
  // 在**对话区里**找：失败时输入框会被塞回同样那句文本，按全文找会命中两处。
  await waitFor(() =>
    expect(within(screen.getByTestId('chat-transcript')).getByText('帮我算一下 1+1')).toBeInTheDocument(),
  )
  expect(screen.queryByText('开始一段对话')).not.toBeInTheDocument()
})
// ——— 流式输出（2026-10-06，M1-1）———

/** 一个由测试自己往里塞帧的响应体：一块一块塞，界面才会一块一块更新。 */
function controllableStream() {
  const encoder = new TextEncoder()
  let controller!: ReadableStreamDefaultController<Uint8Array>
  const stream = new ReadableStream<Uint8Array>({
    start(c) {
      controller = c
    },
  })
  return {
    response: () =>
      new Response(stream, { status: 200, headers: { 'content-type': 'text/event-stream' } }),
    send: (text: string) => controller.enqueue(encoder.encode(text)),
    close: () => controller.close(),
  }
}

function frame(event: string, data: unknown): string {
  return `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`
}

/**
 * 起一个对话页，并把那条流握在手里。
 * 返回 `emit` 让测试一条一条地把帧推给界面。
 */
async function startStreamingChat() {
  const s = controllableStream()
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/api/experts')) return jsonResponse(EXPERTS)
    if (url.includes('/messages/stream')) return s.response()
    if (url.includes('/messages')) return jsonResponse({ messages: [] })
    if (url.endsWith('/api/sessions')) return jsonResponse({ id: 'NEWSESSION01', title: '新对话', expert_id: 'general' })
    if (url.includes('/api/sessions?')) return jsonResponse({ sessions: [] })
    if (url.includes('/healthz')) return jsonResponse({ llm: { max_context_tokens: 8192 } })
    return jsonResponse({})
  })
  renderPage('/chat', fetchMock)
  await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument())
  const box = await screen.findByRole('textbox', { name: '消息' })
  fireEvent.change(box, { target: { value: '帮我看看' } })
  fireEvent.submit(box.closest('form') as HTMLFormElement)
  await waitFor(() => expect(fetchMock.mock.calls.some((c) => String(c[0]).includes('/messages/stream'))).toBe(true))
  return { emit: s.send, close: s.close }
}

const DONE = {
  session_id: 'NEWSESSION01',
  user_message: { id: 'U1', seq: 1, content: '帮我看看', created_at: 1 },
  reply: '最终答案。',
  reasoning: '',
  message: { id: 'A1', seq: 2, content: '最终答案。', created_at: 2 },
  finish_reason: 'Stop',
  usage: { input: 30, output: 8 },
  turn_ms: 1,
  tool_calls: [],
  tool_rounds: 0,
}

it('模型还在写的时候，界面就已经把那半句显示出来了', async () => {
  const { emit, close } = await startStreamingChat()

  emit(frame('user_message', DONE.user_message))
  emit(frame('delta', { kind: 'text', text: '我先' }))
  emit(frame('delta', { kind: 'text', text: '查一下。' }))

  // 关键是**在 done 之前**就能看见 —— 等 done 才出现的话，那就是一次性返回。
  await waitFor(() => expect(screen.getByText('我先查一下。')).toBeInTheDocument())
  expect(screen.queryByText('等待模型返回…')).not.toBeInTheDocument()

  emit(frame('done', DONE))
  close()
  await waitFor(() => expect(screen.queryByText('生成中…')).not.toBeInTheDocument())
})

it('工具往返那一轮的正文会被抹掉，不会和最终答案连成两段', async () => {
  const { emit, close } = await startStreamingChat()

  emit(frame('user_message', DONE.user_message))
  emit(frame('delta', { kind: 'text', text: '我先查一下。' }))
  // **先等它真的显示出来**再谈抹掉。少了这一步，`not.toBeInTheDocument()`
  // 会在第一帧还没渲染时就成立 —— 那条断言就成了永远为真的空话。
  await waitFor(() => expect(screen.getByText('我先查一下。')).toBeInTheDocument())

  emit(frame('discard', { round: 0, text: '我先查一下。', reasoning: '' }))
  // 服务端只把**最终那一轮**的正文放进 done，所以中途那句绝不能留下来。
  await waitFor(() => expect(screen.queryByText('我先查一下。')).not.toBeInTheDocument())

  emit(frame('delta', { kind: 'text', text: '最终答案。' }))
  emit(frame('done', DONE))
  close()
  await waitFor(() => expect(screen.getByText('最终答案。')).toBeInTheDocument())
})

it('工具失败要显示成失败，不能一律显示成「已调用」', async () => {
  const { emit, close } = await startStreamingChat()

  emit(frame('user_message', DONE.user_message))
  emit(frame('tool_call', { name: 'no_such_tool', arguments: {} }))
  await waitFor(() => expect(screen.getByTestId('chat-live-tools')).toBeInTheDocument())
  expect(screen.getByText('正在调用工具 no_such_tool…')).toBeInTheDocument()

  emit(frame('tool_result', { name: 'no_such_tool', ok: false }))
  await waitFor(() => expect(screen.getByText('工具 no_such_tool 失败')).toBeInTheDocument())
  expect(screen.queryByText('工具 no_such_tool 已返回')).not.toBeInTheDocument()

  emit(frame('done', DONE))
  close()
})

it('流中途报错要把服务端的「下一步」显示出来', async () => {
  const { emit, close } = await startStreamingChat()

  emit(frame('user_message', DONE.user_message))
  emit(
    frame('error', {
      code: 'provider_unavailable',
      detail: '连不上模型服务',
      next_step: '先确认端点活着',
    }),
  )
  close()

  const alert = await screen.findByRole('alert')
  expect(alert.textContent ?? '').toContain('连不上模型服务')
  expect(alert.textContent ?? '').toContain('先确认端点活着')
  // 半句话不能留在界面上冒充答案。
  expect(screen.queryByText('生成中…')).not.toBeInTheDocument()
})

it('思考增量与正文分开，不会被当成答案渲染', async () => {
  const { emit, close } = await startStreamingChat()
  emit(frame('user_message', DONE.user_message))
  emit(frame('delta', { kind: 'reasoning', text: '先确认口径' }))
  emit(frame('delta', { kind: 'text', text: '结论如下' }))
  await waitFor(() => expect(screen.getByText('结论如下')).toBeInTheDocument())
  // 思考在折叠区里，界面上默认只露出 summary。
  expect(screen.getByText('思考过程')).toBeInTheDocument()
  emit(frame('done', DONE))
  close()
})

it('侧栏的会话列表请求排除两种 agent 内部会话', async () => {
  // 后端用 `exclude_kind` 把 agent 内部会话藏起来（Q025）：`team_leader` 是建团的
  // 副产品，`team_member` 是派工真跑一轮时每个成员的工作会话。它们都不该出现在
  // 用户侧栏里（前端不按 kind 分组，一律当普通对话渲染）。这条钉的是**出站那一半** ——
  // 后端接口本身另有 `session_kind_filter_http.rs` 覆盖。
  const fetchMock = routedFetch()
  renderPage('/chat', fetchMock)
  await waitFor(() => {
    const listCalls = fetchMock.mock.calls
      .map(([input]) => String(input))
      .filter((url) => url.includes('/api/sessions?'))
    expect(listCalls).toEqual(['/api/sessions?exclude_kind=team_leader,team_member'])
  })
})

// ——— 换会话（2026-10-07）———

/**
 * 「换会话时擦掉上一会话的画面」这条搬到了渲染期（见 ChatPage 里那段注释），
 * 搬动的理由是 lint 那条 `set-state-in-effect`。搬错了的后果用户直接可见：
 * 要么两个会话的话混在一屏，要么刚发完的那一屏被当成残留擦掉。
 * 之前没有测试覆盖这一段 —— 那次搬动正是靠上面那条流式用例撞出来的。
 */
it('切到另一个会话时，上一个会话的话不会留在屏幕上', async () => {
  /** 造一条字段齐全的消息：少一个字段，渲染时就会在 `messageBlocks` 上炸。 */
  const msg = (id: string, seq: number, role: 'user' | 'assistant', content: string) => ({
    id,
    seq,
    role,
    status: 'complete',
    content,
    reasoning: '',
    input_tokens: 0,
    output_tokens: 0,
    turn_ms: 0,
    error_code: '',
    created_at: seq,
  })

  const HISTORY: Record<string, ReturnType<typeof msg>[]> = {
    SESSION_A: [msg('A1', 1, 'user', 'A 会话里问的'), msg('A2', 2, 'assistant', 'A 会话的回答')],
    SESSION_B: [msg('B1', 1, 'user', 'B 会话里问的'), msg('B2', 2, 'assistant', 'B 会话的回答')],
  }

  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.includes('/api/experts')) return jsonResponse(EXPERTS)
    if (url.includes('/healthz')) return jsonResponse({ llm: { max_context_tokens: 8192 } })
    // 列表是 `/api/sessions?…`（`loadSessions()` 带 `exclude_kind` 查询串），
    // 而带 id 的会话详情/历史是 `/api/sessions/<id>/…` —— 两者必须分开判，
    // 否则 `/api/sessions/SESSION_A/messages` 会被当成列表。
    const hit = Object.keys(HISTORY).find((sid) => url.includes(`/sessions/${sid}/`))
    if (hit === 'SESSION_B') {
      // B 的历史**故意不返回**：让切换停在新数据落地之前。
      //
      // 这样才能测到「A 的话有没有被擦掉」。若 B 的历史立刻回来，
      // `setHistory(B)` 会把 A 覆盖掉，于是无论换会话时有没有做 reset，
      // 「A 不在屏幕上」这条断言都成立 —— 那个测试是空的。
      // 卡住 B 才能分辨这两种实现。
      return new Promise<Response>(() => {})
    }
    if (hit) return jsonResponse({ messages: HISTORY[hit] })
    if (/\/api\/sessions\/?(\?|$)/.test(url)) {
      return jsonResponse({
        sessions: [
          { id: 'SESSION_A', title: 'A 会话', expert_id: 'general', message_count: 2, model: 'm', last_active_at: 2 },
          { id: 'SESSION_B', title: 'B 会话', expert_id: 'general', message_count: 2, model: 'm', last_active_at: 1 },
        ],
      })
    }
    return jsonResponse({})
  })

  renderPage('/chat/SESSION_A', fetchMock)
  await waitFor(() => expect(screen.getByText('A 会话的回答')).toBeInTheDocument())

  // 侧栏默认是收起的，会话按钮不在可访问性树里，得先把它打开。
  // 这里按 class 找而不是按 aria-label：那条文案走 i18n，测试环境里
  // 解析出来的字面量不该成为这个用例的依赖。
  const toggle = document.querySelector('.chat-titlebar-toggle')
  expect(toggle).not.toBeNull()
  fireEvent.click(toggle as Element)
  fireEvent.click(await screen.findByRole('button', { name: /B 会话/ }))

  // B 的历史被故意卡住，所以这里断言的是**切换当下**：
  // A 的话必须已经消失。留着它就是两个会话混在一屏 ——
  // 而这正是换会话那段 reset 存在的原因。
  await waitFor(() => expect(screen.queryByText('A 会话的回答')).not.toBeInTheDocument())
  expect(screen.queryByText('A 会话里问的')).not.toBeInTheDocument()
})
