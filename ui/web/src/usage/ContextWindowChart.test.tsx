import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { ContextWindowChart } from './ContextWindowChart'
import type { SessionContext } from './contextApi'

/**
 * 上下文图的两条红线：
 * 1. **没实测过就不画环。** 空会话画一个 0% 的环，等于说「还有一大半没用」，
 *    而真实情况是「完全没量过」。
 * 2. **构成段是字符数。** 界面上必须出现「字符」二字，
 *    否则用户会把字符数当 token 数读。
 *
 * 第三条是后来才补的：**「没量过」和「没拉到」必须长得不一样。**
 * 以前这里是 `if (!data) return null`，于是 404/500 与「还没量过」
 * 在界面上完全同形 —— 失败是静默的。现在失败走 `ErrorNotice`
 * （`role="alert"`），和那句灰字提示是两种不同的东西。
 */

const SESSION_ID = 'AAAA1111BBBB2222CCCC3333DDDD4444'

function context(over: Record<string, unknown> = {}) {
  return {
    max_tokens: 32768,
    used_tokens: 3042,
    used_percent: 9,
    segments: [
      { key: 'system_prompt', chars: 242 },
      { key: 'tool_definitions', chars: 3100 },
      { key: 'skills', chars: 0 },
      { key: 'mcp', chars: 0 },
      { key: 'conversation', chars: 1800 },
    ],
    segment_unit: 'chars',
    ...over,
  }
}

/** 一份有实测值的上下文，类型与 `loadSessionContext` 的返回一致。 */
const MEASURED: SessionContext = {
  max_tokens: 32768,
  used_tokens: 3042,
  used_percent: 9,
  segments: [
    { key: 'system_prompt', chars: 242 },
    { key: 'tool_definitions', chars: 3100 },
    { key: 'skills', chars: 0 },
    { key: 'mcp', chars: 0 },
    { key: 'conversation', chars: 1800 },
  ],
  segment_unit: 'chars',
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

/** 服务端的错误信封，字段名见 `api/client.ts` 顶部的注释。 */
function serverError(status = 404): Response {
  return json(
    {
      error: {
        code: 'session_not_found',
        detail: '会话不存在或已被删除。',
        next_step: '回到会话列表重新选一个。',
      },
    },
    status,
  )
}

/**
 * 统一入口。**故意不写死 200** —— 以前这个 helper 只会回 200，
 * 于是新增的错误分支一行都没被执行过。
 */
function renderWith(
  fetchImpl: () => Promise<Response>,
  contextProp: SessionContext | null = null,
): { container: HTMLElement; unmount: () => void } {
  vi.stubGlobal('fetch', vi.fn(fetchImpl))
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <ContextWindowChart sessionId={SESSION_ID} context={contextProp} />
    </QueryClientProvider>,
  )
}

/** 永远不 resolve 的 fetch —— 请求还「在飞」的那一段。 */
function neverResolves(): Promise<Response> {
  return new Promise<Response>(() => {})
}

function renderChart(body: unknown): void {
  renderWith(() => Promise.resolve(json(body)))
}

function ringAriaLabel(): string | null | undefined {
  return screen
    .getAllByRole('img')
    .find((el) => el.getAttribute('aria-label')?.includes('上下文已占用'))
    ?.getAttribute('aria-label')
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
  vi.unstubAllGlobals()
})

describe('上下文窗口图', () => {
  it('没有实测值时明说没量过，不画空环', async () => {
    renderChart(context({ used_tokens: null, used_percent: null }))
    await waitFor(() => expect(screen.getByText(/没有实测值/)).toBeInTheDocument())
    expect(screen.queryByRole('img')).toBeNull()
  })

  it('有实测值时给出可读的环形图标签', async () => {
    renderChart(context())
    await waitFor(() => expect(screen.getByText('上下文占用')).toBeInTheDocument())
    // canvas 对读屏软件是黑的，所以必须有 aria-label。
    const ring = screen.getAllByRole('img').find((el) =>
      el.getAttribute('aria-label')?.includes('上下文已占用'),
    )
    expect(ring).toBeTruthy()
    // 标签用紧凑写法（3k / 33k），和 Octop 的 formatTokenK 一致。
    expect(ring?.getAttribute('aria-label')).toContain('3k')
    expect(ring?.getAttribute('aria-label')).toContain('9%')
  })

  it('构成图必须声明分段是估算，而不是假装它是真值', async () => {
    renderChart(context())
    await waitFor(() => expect(screen.getByText('上下文构成')).toBeInTheDocument())
    // 每个分段值前面带 `~`（照 octop）。说明句只交代「哪些是真值」，
    // 不再用一整句「quill 没有分词器」去解释符号 —— 那是实现细节。
    expect(screen.getByText(/分段为估算值/)).toBeInTheDocument()
    const aria = screen
      .getAllByRole('img')
      .map((el) => el.getAttribute('aria-label') ?? '')
      .join(' ')
    expect(aria).toContain('~')
    expect(aria).not.toContain('字符')
  })

  it('上下文上限为 0 时不显示百分比，也不假装满环', async () => {
    renderChart(context({ max_tokens: 0, used_percent: 0 }))
    await waitFor(() => expect(screen.getByText('上下文占用')).toBeInTheDocument())
    const ring = screen.getAllByRole('img').find((el) =>
      el.getAttribute('aria-label')?.includes('上下文已占用'),
    )
    expect(ring).toBeTruthy()
  })

  it('切语言后环形图的无障碍标签跟着换，不留在旧语言上', async () => {
    renderChart(MEASURED)
    await waitFor(() => expect(ringAriaLabel()).toContain('上下文已占用'))

    await i18n.changeLanguage('en')

    await waitFor(() =>
      expect(
        screen
          .getAllByRole('img')
          .some((el) => el.getAttribute('aria-label')?.includes('Context is using')),
      ).toBe(true),
    )
    expect(screen.queryByText('上下文占用')).toBeNull()
  })
})

describe('「没量过」和「没拉到」必须是两件事', () => {
  it('同一个「没有数据」的条件，三种状态给出三种不同的结果', async () => {
    // 1) 请求在飞
    const { container } = renderWith(neverResolves)
    // 既没有报错误，也没有谎称「没量过」，更没有画一个 0% 的环。
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.queryByText(/没有实测值/)).toBeNull()
    expect(screen.queryByRole('img')).toBeNull()
    // 已知局限：加载中这一段目前什么都不渲染（没有骨架屏、没有 aria-busy），
    // 读屏用户拿不到「正在加载」这个信息。见交付报告里的说明。
    expect(container).toBeEmptyDOMElement()
    cleanup()

    // 2) 拉到了，但没实测值
    renderChart(context({ used_tokens: null, used_percent: null }))
    await waitFor(() => expect(screen.getByText(/没有实测值/)).toBeInTheDocument())
    // 是提示，不是错误。
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.queryByRole('img')).toBeNull()
    cleanup()

    // 3) 没拉到
    renderWith(() => Promise.resolve(serverError()))
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('会话不存在或已被删除。')
    // 失败时绝不能顺手说一句「没量过」—— 那会把用户的注意力引向错误的方向。
    expect(screen.queryByText(/没有实测值/)).toBeNull()
    expect(screen.queryByRole('img')).toBeNull()
  })

  it('请求失败时把服务端给的错误原样端出来：说明、错误码、下一步都在', async () => {
    renderWith(() => Promise.resolve(serverError()))

    const alert = await screen.findByRole('alert')
    // detail：到底发生了什么
    expect(alert).toHaveTextContent('会话不存在或已被删除。')
    // code：可检索的标识
    expect(alert).toHaveTextContent('session_not_found')
    // next_step：单独一行，比错误本身更需要被读到
    expect(alert).toHaveTextContent('回到会话列表重新选一个。')
  })

  it('服务端没给错误信封时也照样报错，不静默', async () => {
    renderWith(() => Promise.resolve(new Response('boom', { status: 500 })))

    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Request failed (500)')
  })

  it('已有 context 数据时，请求失败也不进错误分支', async () => {
    // `data = context ?? query.data`：调用方已经拿到数据（比如聊天页顺手传的），
    // 就没理由因为一次后台刷新失败而把图收起来。
    renderWith(() => Promise.resolve(serverError(500)), MEASURED)

    await waitFor(() => expect(screen.getByText('上下文占用')).toBeInTheDocument())
    expect(screen.queryByRole('alert')).toBeNull()
  })
})
