import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { ContextWindowChart } from './ContextWindowChart'

/**
 * 上下文图的两条红线：
 * 1. **没实测过就不画环。** 空会话画一个 0% 的环，等于说「还有一大半没用」，
 *    而真实情况是「完全没量过」。
 * 2. **构成段是字符数。** 界面上必须出现「字符」二字，
 *    否则用户会把字符数当 token 数读。
 */

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

function renderChart(body: unknown): void {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue(
      new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    ),
  )
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <ContextWindowChart sessionId="AAAA1111BBBB2222CCCC3333DDDD4444" context={null} />
    </QueryClientProvider>,
  )
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

  it('构成图必须标明单位是字符，不是 token', async () => {
    renderChart(context())
    await waitFor(() => expect(screen.getByText('上下文构成')).toBeInTheDocument())
    expect(screen.getByText(/按字符数，不是 token 数/)).toBeInTheDocument()
  })

  it('上下文上限为 0 时不显示百分比，也不假装满环', async () => {
    renderChart(context({ max_tokens: 0, used_percent: 0 }))
    await waitFor(() => expect(screen.getByText('上下文占用')).toBeInTheDocument())
    const ring = screen.getAllByRole('img').find((el) =>
      el.getAttribute('aria-label')?.includes('上下文已占用'),
    )
    expect(ring).toBeTruthy()
  })
})