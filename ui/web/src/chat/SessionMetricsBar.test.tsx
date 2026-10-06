import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import i18n from '../i18n'
import SessionMetricsBar from './SessionMetricsBar'
import type { SessionMetrics } from './sessionMetrics'

/**
 * 界面层的红线：没测到的指标不许出现在屏幕上。
 *
 * quill 不记录工具耗时与首字延迟，本地模型大多也不报缓存 token。
 * 那些格子的正确表现是「没有这一格」，而不是「这一格写着 0」。
 */

function metrics(over: Partial<SessionMetrics> = {}): SessionMetrics {
  return {
    turns: 3,
    steps: 3,
    llm_duration_ms: 5000,
    tool_duration_ms: null,
    ttft_avg_ms: null,
    tok_per_s: 12.5,
    cache_hit_ratio: null,
    input_tokens: 1000,
    output_tokens: 250,
    cache_read_tokens: null,
    ...over,
  }
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

describe('SessionMetricsBar', () => {
  it('renders nothing when there are no metrics at all', () => {
    const { container } = render(<SessionMetricsBar metrics={null} />)
    expect(container.firstChild).toBeNull()
  })

  it('omits the chips quill cannot measure', () => {
    render(<SessionMetricsBar metrics={metrics()} />)
    const bar = screen.getByLabelText('会话统计')
    // 能测的都在
    expect(bar.querySelector('[data-metric="turns"]')).not.toBeNull()
    expect(bar.querySelector('[data-metric="input_tokens"]')).not.toBeNull()
    // 测不了的必须整格消失，不能显示成 0
    expect(bar.querySelector('[data-metric="tool_duration_ms"]')).toBeNull()
    expect(bar.querySelector('[data-metric="ttft_avg_ms"]')).toBeNull()
    expect(bar.querySelector('[data-metric="cache_hit_ratio"]')).toBeNull()
    expect(bar.querySelector('[data-metric="cache_read_tokens"]')).toBeNull()
  })

  it('shows a reported zero cache hit rather than hiding it', () => {
    // 真的报了 0 命中，和没报是两回事：前者要显示 0%，后者要消失。
    render(<SessionMetricsBar metrics={metrics({ cache_read_tokens: 0, cache_hit_ratio: 0 })} />)
    const bar = screen.getByLabelText('会话统计')
    expect(bar.querySelector('[data-metric="cache_hit_ratio"]')?.textContent).toContain('0%')
    expect(bar.querySelector('[data-metric="cache_read_tokens"]')?.textContent).toContain('0')
  })

  it('formats the values it does display', () => {
    render(<SessionMetricsBar metrics={metrics()} />)
    const bar = screen.getByLabelText('会话统计')
    expect(bar.querySelector('[data-metric="turns"]')?.textContent).toBe('3 轮次')
    expect(bar.querySelector('[data-metric="llm_duration_ms"]')?.textContent).toBe('模型耗时 5.0s')
    expect(bar.querySelector('[data-metric="input_tokens"]')?.textContent).toBe('入参 1.0k')
    expect(bar.querySelector('[data-metric="tok_per_s"]')?.textContent).toBe('12.5 tok/s')
  })

  it('separates chips with a middot and never leaves a leading one', () => {
    const { container } = render(<SessionMetricsBar metrics={metrics()} />)
    const seps = container.querySelectorAll('.chat-metrics-sep')
    const chips = container.querySelectorAll('.chat-metrics-chip')
    expect(seps).toHaveLength(chips.length - 1)
  })
})