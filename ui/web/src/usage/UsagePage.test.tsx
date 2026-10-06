import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { UsagePage } from './UsagePage'

/**
 * 用量统计页的红线测试。
 *
 * 这一页最容易出的错，是把服务端回的 `null` 渲染成 `0`。
 * 「缓存命中 0%」「工具耗时 0ms」看起来都像正常的统计，但它们是
 * quill 从没测量过的东西 —— 用户会拿它们做决策。
 */

function metrics(over: Record<string, unknown> = {}) {
  return {
    turns: 2,
    steps: 2,
    llm_duration_ms: 4000,
    tool_duration_ms: null,
    ttft_avg_ms: null,
    tok_per_s: 8.5,
    cache_hit_ratio: null,
    input_tokens: 1200,
    output_tokens: 80,
    cache_read_tokens: null,
    ...over,
  }
}

function report(over: Record<string, unknown> = {}) {
  return {
    totals: metrics({ turns: 2, steps: 2 }),
    sessions: [
      {
        id: 'AAAA1111BBBB2222CCCC3333DDDD4444',
        title: '用量测试会话',
        expert_id: 'general',
        last_active_at: 1,
        metrics: metrics(),
      },
    ],
    session_count: 1,
    limit: 200,
    truncated: false,
    ...over,
  }
}

function renderPage(body: unknown): void {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue(
      new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } }),
    ),
  )
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <UsagePage />
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

describe('用量统计页', () => {
  it('空会话列表时说清是空，而不是显示一张全 0 的表', async () => {
    renderPage(report({ sessions: [], session_count: 0, totals: metrics({ turns: 0, steps: 0 }) }))
    await waitFor(() => expect(screen.getByText(/还没有任何会话/)).toBeInTheDocument())
    expect(screen.queryByRole('table')).toBeNull()
  })

  it('没测到的指标显示「—」，绝不显示 0', async () => {
    renderPage(report())
    await waitFor(() => expect(screen.getByText('用量测试会话')).toBeInTheDocument())
    const row = screen.getByText('用量测试会话').closest('tr') as HTMLElement
    // 上游没上报缓存 token：这两格必须是没有依据的横杠
    expect(row.querySelector('[data-metric="cache_hit_ratio"]')?.textContent).toBe('—')
    expect(row.querySelector('[data-metric="cache_read_tokens"]')?.textContent).toBe('—')
    // quill 不记录工具耗时与首字延迟
    expect(row.querySelector('[data-metric="tool_duration_ms"]')?.textContent).toBe('—')
    expect(row.querySelector('[data-metric="ttft_avg_ms"]')?.textContent).toBe('—')
    // 有真值的才显示数字
    expect(row.querySelector('[data-metric="input_tokens"]')?.textContent).toBe('1.2k')
    expect(row.querySelector('[data-metric="tok_per_s"]')?.textContent).toBe('8.5')
  })

  it('列表有「最后活动」列，图上的柱子才对得回表里这一行', async () => {
    const when = new Date(2026, 9, 6, 9, 5).getTime()
    renderPage(
      report({
        sessions: [
          {
            id: 'AAAA1111BBBB2222CCCC3333DDDD4444',
            title: '',
            expert_id: null,
            last_active_at: when,
            metrics: metrics(),
          },
        ],
      }),
    )
    await waitFor(() => expect(screen.getByText('最后活动')).toBeInTheDocument())
    const row = screen.getByText('新对话').closest('tr') as HTMLElement
    // 侧栏里这些会话全都叫「新对话」，图上八根柱子同名就分不出谁是谁，
    // 所以每行必须带着真实时间。
    expect(row.querySelector('.usage-when')?.textContent).toBe('10-06 09:05')
  })

  it('服务端报了 0 命中就显示 0%，与「没报」区分开', async () => {
    renderPage(
      report({
        sessions: [
          {
            id: 'AAAA1111BBBB2222CCCC3333DDDD4444',
            title: '有缓存的会话',
            expert_id: null,
            last_active_at: 1,
            metrics: metrics({ cache_hit_ratio: 0, cache_read_tokens: 0 }),
          },
        ],
      }),
    )
    await waitFor(() => expect(screen.getByText('有缓存的会话')).toBeInTheDocument())
    const row = screen.getByText('有缓存的会话').closest('tr') as HTMLElement
    expect(row.querySelector('[data-metric="cache_hit_ratio"]')?.textContent).toBe('0%')
  })

  it('合计行独立存在，且用的是 totals 而不是第一行', async () => {
    renderPage(
      report({
        totals: metrics({ turns: 9, steps: 9, input_tokens: 9999 }),
        sessions: [
          {
            id: 'AAAA1111BBBB2222CCCC3333DDDD4444',
            title: '小会话',
            expert_id: null,
            last_active_at: 1,
            metrics: metrics(),
          },
        ],
      }),
    )
    await waitFor(() => expect(screen.getByText('小会话')).toBeInTheDocument())
    const totalRow = screen.getByText('合计').closest('tr') as HTMLElement
    expect(totalRow.querySelector('[data-metric="turns"]')?.textContent).toBe('9')
    expect(totalRow.querySelector('[data-metric="input_tokens"]')?.textContent).toBe('10.0k')
  })

  it('统计被截断时必须明说，不能让人以为这就是全部', async () => {
    renderPage(report({ truncated: true, session_count: 200 }))
    await waitFor(() => expect(screen.getByText('统计被截断')).toBeInTheDocument())
    expect(screen.getByText(/只统计了最近 200 个会话/)).toBeInTheDocument()
  })

  it('未截断时不显示截断告警', async () => {
    renderPage(report())
    await waitFor(() => expect(screen.getByText('用量测试会话')).toBeInTheDocument())
    expect(screen.queryByText('统计被截断')).toBeNull()
  })

  it('会话没绑定角色时说清「未绑定」，不留空白', async () => {
    renderPage(
      report({
        sessions: [
          {
            id: 'AAAA1111BBBB2222CCCC3333DDDD4444',
            title: '没角色的会话',
            expert_id: null,
            last_active_at: 1,
            metrics: metrics(),
          },
        ],
      }),
    )
    await waitFor(() => expect(screen.getByText('没角色的会话')).toBeInTheDocument())
    expect(screen.getByText('未绑定角色')).toBeInTheDocument()
  })

  it('没起标题的会话显示「新对话」，不把 32 位 hex 甩给用户', async () => {
    renderPage(
      report({
        sessions: [
          {
            id: 'AAAA1111BBBB2222CCCC3333DDDD4444',
            title: '',
            expert_id: null,
            last_active_at: 1,
            metrics: metrics(),
          },
        ],
      }),
    )
    await waitFor(() => expect(screen.getByText('新对话')).toBeInTheDocument())
    // 那串 hex 仍然得在链接里 —— 只是不再当成标题显示给人看。
    expect(screen.getByText('新对话').closest('a')?.getAttribute('href')).toBe(
      '/chat/AAAA1111BBBB2222CCCC3333DDDD4444',
    )
  })
})