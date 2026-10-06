import { describe, expect, it } from 'vitest'
import {
  formatDurationMs,
  formatMetric,
  formatTokenCount,
  visibleMetrics,
  type SessionMetrics,
} from './sessionMetrics'

/**
 * 这组测试盯的是一条红线：**算不出来的东西不许变成 0 显示给用户。**
 *
 * quill 不记录工具耗时、不记录首 token 时刻，本地模型大多也不报缓存 token。
 * 所以服务端回 null 是常态，前端必须把它整格跳过。哪天有人图省事写成
 * `value ?? 0`，这里就会红。
 */

function full(over: Partial<SessionMetrics> = {}): SessionMetrics {
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

describe('visibleMetrics', () => {
  it('skips null metrics instead of showing them as zero', () => {
    const keys = visibleMetrics(full()).map((m) => m.key)
    expect(keys).not.toContain('tool_duration_ms')
    expect(keys).not.toContain('ttft_avg_ms')
    expect(keys).not.toContain('cache_hit_ratio')
    expect(keys).not.toContain('cache_read_tokens')
  })

  it('keeps a reported zero — that is a real measurement', () => {
    // 上游报了 cached_tokens=0，和「没报」是两回事，必须都显示出来。
    const keys = visibleMetrics(full({ cache_read_tokens: 0, cache_hit_ratio: 0 })).map((m) => m.key)
    expect(keys).toContain('cache_read_tokens')
    expect(keys).toContain('cache_hit_ratio')
  })

  it('an empty session still shows its zero turn count', () => {
    const keys = visibleMetrics(full({ turns: 0, steps: 0 })).map((m) => m.key)
    expect(keys).toContain('turns')
    expect(keys).toContain('steps')
  })

  it('returns nothing for a session where every metric is unknown', () => {
    const m = visibleMetrics({
      turns: 0,
      steps: 0,
      llm_duration_ms: null,
      tool_duration_ms: null,
      ttft_avg_ms: null,
      tok_per_s: null,
      cache_hit_ratio: null,
      input_tokens: null,
      output_tokens: null,
      cache_read_tokens: null,
    })
    expect(m).toHaveLength(2)
  })

  it('preserves the declared display order', () => {
    const keys = visibleMetrics(
      full({ cache_read_tokens: 10, cache_hit_ratio: 0.1, ttft_avg_ms: 120, tool_duration_ms: 800 }),
    ).map((m) => m.key)
    expect(keys).toEqual([
      'turns',
      'steps',
      'llm_duration_ms',
      'tool_duration_ms',
      'ttft_avg_ms',
      'tok_per_s',
      'cache_hit_ratio',
      'input_tokens',
      'output_tokens',
      'cache_read_tokens',
    ])
  })

  it('treats undefined the same as null', () => {
    // 老服务端可能压根没有这个字段。undefined 同样不能被显示成 0。
    const m = visibleMetrics({ turns: 1, steps: 1 } as SessionMetrics)
    expect(m.map((x) => x.key)).toEqual(['turns', 'steps'])
  })
})

describe('formatDurationMs', () => {
  it('formats the three magnitude bands', () => {
    expect(formatDurationMs(0)).toBe('0ms')
    expect(formatDurationMs(999)).toBe('999ms')
    expect(formatDurationMs(1500)).toBe('1.5s')
    expect(formatDurationMs(42_000)).toBe('42s')
    expect(formatDurationMs(125_000)).toBe('2m05s')
  })

  it('refuses to render nonsense', () => {
    expect(formatDurationMs(-1)).toBe('—')
    expect(formatDurationMs(Number.NaN)).toBe('—')
    expect(formatDurationMs(Number.POSITIVE_INFINITY)).toBe('—')
  })
})

describe('formatTokenCount', () => {
  it('keeps small counts exact and abbreviates big ones', () => {
    expect(formatTokenCount(0)).toBe('0')
    expect(formatTokenCount(999)).toBe('999')
    expect(formatTokenCount(1000)).toBe('1.0k')
    expect(formatTokenCount(1500)).toBe('1.5k')
    expect(formatTokenCount(10_000)).toBe('10k')
    expect(formatTokenCount(12_500)).toBe('13k')
  })
})

describe('formatMetric', () => {
  it('renders a cache hit ratio as a percentage', () => {
    expect(formatMetric('cache_hit_ratio', 0.9)).toBe('90%')
    expect(formatMetric('cache_hit_ratio', 0)).toBe('0%')
  })

  it('renders token counts compactly', () => {
    expect(formatMetric('input_tokens', 2500)).toBe('2.5k')
    expect(formatMetric('output_tokens', 300)).toBe('300')
    expect(formatMetric('cache_read_tokens', 0)).toBe('0')
  })

  it('renders durations through formatDurationMs', () => {
    expect(formatMetric('llm_duration_ms', 2500)).toBe('2.5s')
    expect(formatMetric('ttft_avg_ms', 800)).toBe('800ms')
  })

  it('keeps tok/s at one decimal only when needed', () => {
    expect(formatMetric('tok_per_s', 12)).toBe('12')
    expect(formatMetric('tok_per_s', 12.53)).toBe('12.5')
  })
})