import { render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import i18n from '../i18n'
import SessionMetricsBar from './SessionMetricsBar'
import { CHAT_BAR_KEYS, type SessionMetrics } from './sessionMetrics'

/**
 * 界面层的两条红线：
 *
 * 1. **没测到的指标不许出现在屏幕上。** quill 不记录工具耗时与首字延迟，
 *    本地模型大多也不报缓存 token。那些格子的正确表现是「没有这一格」，
 *    而不是「这一格写着 0」。
 * 2. **搬走的指标不许偷偷回来。** 这一条是下面 `movedMetricsAreGone`
 *    存在的唯一理由 —— 删掉一行显示很容易，下次「照抄上游」时
 *    把它连同 METRIC_KEYS 一起带回来更自然，而那时没人会想到要拦。
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
    // 能测且该留的都在
    expect(bar.querySelector('[data-metric="input_tokens"]')).not.toBeNull()
    expect(bar.querySelector('[data-metric="output_tokens"]')).not.toBeNull()
    // 测不了的必须整格消失，不能显示成 0
    expect(bar.querySelector('[data-metric="tool_duration_ms"]')).toBeNull()
    expect(bar.querySelector('[data-metric="ttft_avg_ms"]')).toBeNull()
    expect(bar.querySelector('[data-metric="cache_read_tokens"]')).toBeNull()
  })

  it('shows a reported zero cache read rather than hiding it', () => {
    // 真的报了 0，和没报是两回事：前者要显示 0，后者要消失。
    // （命中率本身已经搬去上下文小环的面板，这里只留缓存读的绝对值。）
    render(<SessionMetricsBar metrics={metrics({ cache_read_tokens: 0 })} />)
    const bar = screen.getByLabelText('会话统计')
    expect(bar.querySelector('[data-metric="cache_read_tokens"]')?.textContent).toContain('0')
  })

  it('formats the values it does display', () => {
    render(<SessionMetricsBar metrics={metrics()} />)
    const bar = screen.getByLabelText('会话统计')
    expect(bar.querySelector('[data-metric="input_tokens"]')?.textContent).toBe('入参 1.0k')
    expect(bar.querySelector('[data-metric="output_tokens"]')?.textContent).toBe('出参 250')
  })

  /**
   * 搬走的那五项，一个都不许回到这一行。
   *
   * `metrics()` 里每一项都有真值（3 / 3 / 5000 / 12.5 / 0.96），
   * 所以只要代码改回全量渲染，这里立刻红。
   */
  it('movedMetricsAreGone', () => {
    render(
      <SessionMetricsBar
        metrics={metrics({ cache_hit_ratio: 0.96, cache_read_tokens: 900 })}
      />,
    )
    const bar = screen.getByLabelText('会话统计')
    for (const key of ['turns', 'steps', 'llm_duration_ms', 'tok_per_s', 'cache_hit_ratio']) {
      expect(
        bar.querySelector(`[data-metric="${key}"]`),
        `${key} 已经搬走了，不该再出现在聊天页这排统计里`,
      ).toBeNull()
    }
  })

  it('only ever renders the three metrics CHAT_BAR_KEYS names', () => {
    // 反向断言：界面上出现过的每一格都必须是我们点名要的那三个。
    // 上面几条只点了「不许出现的名字」，这里防的是「加了个新的没想过」。
    render(
      <SessionMetricsBar
        metrics={metrics({ cache_hit_ratio: 0.96, cache_read_tokens: 900 })}
      />,
    )
    const bar = screen.getByLabelText('会话统计')
    const shown = Array.from(bar.querySelectorAll('.chat-metrics-chip')).map(
      (el) => el.getAttribute('data-metric'),
    )
    expect(shown).toEqual([...CHAT_BAR_KEYS])
  })

  it('separates chips with a middot and never leaves a leading one', () => {
    const { container } = render(<SessionMetricsBar metrics={metrics()} />)
    const seps = container.querySelectorAll('.chat-metrics-sep')
    const chips = container.querySelectorAll('.chat-metrics-chip')
    expect(seps).toHaveLength(chips.length - 1)
  })
})