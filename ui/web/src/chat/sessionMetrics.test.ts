import { describe, expect, it } from 'vitest'

import i18n from '../i18n'
import { en, zhCN } from '../i18n/resources'
import {
  METRIC_KEYS,
  formatDurationMs,
  formatMetric,
  formatTokenCount,
  metricLabel,
  visibleMetrics,
  type SessionMetricKey,
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

/**
 * `metricLabel` 是**单一口径**：聊天页统计条和用量页表头展示同一批指标，
 * 名字必须来自同一张表。抄一份的后果是同一个指标在两个页面显示不同名字。
 *
 * 这里钉的是那张表的**穷尽性**：
 * - `METRIC_LABELS` 的类型是 `Record<SessionMetricKey, …>`，漏一项 tsc 就红；
 * - 但「key 存在」不等于「中英两种语言都写了文案」—— 缺一个语言时
 *   i18next 会静默退回 `defaultValue`（中文），于是**英文界面里蹦出中文单位**。
 *   这一层类型系统管不到，只能靠测试。
 */

/**
 * ## 穷尽性钉在哪里
 *
 * `METRIC_LABELS` 声明为 `Record<SessionMetricKey, …>`，所以「少一个 key」
 * 这一层是 tsc 管的（给 `SessionMetricKey` 加一项而表里没跟上，`tsc -b` 直接红）。
 *
 * 但 `METRIC_KEYS` 上有 `readonly SessionMetricKey[]` 的**标注** ——
 * 标注只约束元素类型，编译器看不见数组里少了哪一项。所以
 * 「这一项到底在不在」只能在运行时钉死，落在下面这个对象字面量上：
 * 它的类型是 `Record<SessionMetricKey, true>`，`SessionMetricKey` 加一项，
 * 这里就编译不过（少一个属性）；运行时再拿它的键和 `METRIC_KEYS` 对一遍。
 */
const ALL_METRIC_KEYS: Record<SessionMetricKey, true> = {
  turns: true,
  steps: true,
  llm_duration_ms: true,
  tool_duration_ms: true,
  ttft_avg_ms: true,
  tok_per_s: true,
  cache_hit_ratio: true,
  input_tokens: true,
  output_tokens: true,
  cache_read_tokens: true,
}

interface CapturedLabel {
  metric: SessionMetricKey
  /** `metricLabel` 实际拿去查 i18n 的 key。 */
  i18nKey: string
  /** `defaultValue` 兜底文案。 */
  fallback: string
}

/**
 * `METRIC_LABELS` 没有导出，所以从 `metricLabel` 的行为里把它读出来：
 * 传一个只记账不翻译的 `t`，就能看到每个指标实际用的 key 与兜底文案。
 */
function captureLabels(): CapturedLabel[] {
  let last = { i18nKey: '', fallback: '' }
  return METRIC_KEYS.map((key) => {
    metricLabel(key, (i18nKey, options) => {
      last = { i18nKey, fallback: String(options?.defaultValue ?? '') }
      return last.fallback
    })
    return { metric: key, ...last }
  })
}

/** 按 `a.b.c` 取值，取不到就是 undefined —— 用来证明「语言包里真的有这一项」。 */
function resolve(tree: unknown, path: string): unknown {
  return path.split('.').reduce<unknown>((node, part) => {
    if (node && typeof node === 'object') return (node as Record<string, unknown>)[part]
    return undefined
  }, tree)
}

describe('metricLabel 的单一口径', () => {
  it('METRIC_KEYS 一项不多一项不少地覆盖了 SessionMetricKey', () => {
    const declared = Object.keys(ALL_METRIC_KEYS).sort()
    expect([...METRIC_KEYS].sort()).toEqual(declared)
    expect(new Set(METRIC_KEYS).size).toBe(declared.length)
  })

  it('每个指标的 i18n key 在中英两种语言里都真的存在（缺一个就会露出 key 本身）', () => {
    const labels = captureLabels()
    expect(labels).toHaveLength(METRIC_KEYS.length)

    for (const { metric, i18nKey, fallback } of labels) {
      const zh = resolve(zhCN.translation, i18nKey)
      const english = resolve(en.translation, i18nKey)
      expect(typeof zh, `${metric} 在中文语言包里缺失`).toBe('string')
      expect(typeof english, `${metric} 在英文语言包里缺失`).toBe('string')
      expect(String(zh).length, `${metric} 的中文文案内是空的`).toBeGreaterThan(0)
      expect(String(english).length, `${metric} 的英文文案内是空的`).toBeGreaterThan(0)
      expect(fallback.length, `${metric} 没有兜底文案`).toBeGreaterThan(0)
    }
  })

  it('十个指标互不撞 key（撞了就等于有两个指标共用一个名字）', () => {
    const keys = captureLabels().map((l) => l.i18nKey)
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('走真实 i18n：中英文各自取到语言包里的文案，不退回兜底', async () => {
    const labels = captureLabels()
    const t = (k: string, o?: Record<string, unknown>): string => i18n.t(k, o)

    await i18n.changeLanguage('en')
    for (const { metric, i18nKey } of labels) {
      const label = metricLabel(metric, t)
      expect(label, `${metric} 的英文名露了 key`).not.toContain('chat.metrics')
      expect(label).toBe(resolve(en.translation, i18nKey))
    }

    await i18n.changeLanguage('zh-CN')
    for (const { metric, i18nKey } of labels) {
      expect(metricLabel(metric, t)).toBe(resolve(zhCN.translation, i18nKey))
    }
  })
})