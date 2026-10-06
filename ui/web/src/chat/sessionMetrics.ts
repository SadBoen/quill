/**
 * 会话级 Token 统计的纯函数部分。
 *
 * 抄自 octop `pages/Chat/utils/trajectoryModel.ts` 的 `visibleMetrics()` /
 * `formatDurationMs()`，以及 `TrajectoryMetricsBar.tsx` 里的
 * `formatTokenCount()` / `formatMetric()`。
 *
 * 抄的是**格式**，不是语义。与 octop 的关键差别只有一条，但很要命：
 * `null` 在这里表示「quill 没记这一项」，**必须在展示前被过滤掉**。
 * 直接显示成 0，就等于给用户报了一个从没被测量的数字。
 */

/** 指标键。与服务端 `SessionMetrics` 的字段同名。 */
export type SessionMetricKey =
  | 'turns'
  | 'steps'
  | 'llm_duration_ms'
  | 'tool_duration_ms'
  | 'ttft_avg_ms'
  | 'tok_per_s'
  | 'cache_hit_ratio'
  | 'input_tokens'
  | 'output_tokens'
  | 'cache_read_tokens'

export interface SessionMetrics {
  turns: number
  steps: number
  llm_duration_ms: number | null
  tool_duration_ms: number | null
  ttft_avg_ms: number | null
  tok_per_s: number | null
  cache_hit_ratio: number | null
  input_tokens: number | null
  output_tokens: number | null
  cache_read_tokens: number | null
}

/** 展示顺序即此数组顺序。与 octop 的 METRIC_KEYS 一致。 */
export const METRIC_KEYS: readonly SessionMetricKey[] = [
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
]

export interface VisibleMetric {
  key: SessionMetricKey
  value: number
}

/**
 * 只留下真正有值的指标。
 *
 * `value != null` 这一句是本文件的关键：服务端回 `null` 的项直接消失，
 * 而不是变成 `0`。quill 不记录工具耗时和首 token 时刻，所以那两格
 * 永远不显示 —— 这是诚实，不是缺功能。
 */
export function visibleMetrics(metrics: SessionMetrics): VisibleMetric[] {
  const out: VisibleMetric[] = []
  for (const key of METRIC_KEYS) {
    const value = metrics[key]
    if (value !== null && value !== undefined) {
      out.push({ key, value })
    }
  }
  return out
}

/**
 * 指标名 → 展示文案。
 *
 * 聊天页统计条（`SessionMetricsBar`）和用量页表头（`UsagePage`）展示的是**同一批
 * 指标**，所以名字必须来自同一张表：抄一份的结果就是同一个指标在两个页面显示不同名字。
 *
 * `Record<SessionMetricKey, ...>` 是刻意的：`SessionMetricKey` 新增一项而这里漏了，
 * TypeScript 直接报错，而不是让那一格默默显示 i18n 的原始 key。
 *
 * `defaultValue` 只是兜底 —— 项目规矩是文案必须写进 `i18n/resources.ts` 的中英两处，
 * 否则 `.i18n-check.mjs` 门禁会红。
 */
const METRIC_LABELS: Record<SessionMetricKey, { key: string; fallback: string }> = {
  turns: { key: 'chat.metrics.turns', fallback: '轮次' },
  steps: { key: 'chat.metrics.steps', fallback: '回复' },
  llm_duration_ms: { key: 'chat.metrics.llmMs', fallback: '模型耗时' },
  tool_duration_ms: { key: 'chat.metrics.toolMs', fallback: '工具耗时' },
  ttft_avg_ms: { key: 'chat.metrics.ttft', fallback: '首字延迟' },
  tok_per_s: { key: 'chat.metrics.tokPerS', fallback: 'tok/s' },
  cache_hit_ratio: { key: 'chat.metrics.cacheHit', fallback: '缓存命中' },
  input_tokens: { key: 'chat.metrics.inputTokens', fallback: '入参' },
  output_tokens: { key: 'chat.metrics.outputTokens', fallback: '出参' },
  cache_read_tokens: { key: 'chat.metrics.cacheRead', fallback: '缓存读' },
}

export function metricLabel(
  key: SessionMetricKey,
  t: (k: string, o?: Record<string, unknown>) => string,
): string {
  const entry = METRIC_LABELS[key]
  return t(entry.key, { defaultValue: entry.fallback })
}

/** 毫秒 → 人读的时长。逐字抄 octop 的 `formatDurationMs`。 */
export function formatDurationMs(milliseconds: number): string {
  if (!Number.isFinite(milliseconds) || milliseconds < 0) return '—'
  if (milliseconds < 1000) return `${Math.round(milliseconds)}ms`
  if (milliseconds < 60_000) {
    const seconds = milliseconds / 1000
    return `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s`
  }
  const minutes = Math.floor(milliseconds / 60_000)
  const seconds = Math.round((milliseconds % 60_000) / 1000)
  return `${minutes}m${String(seconds).padStart(2, '0')}s`
}

/** token 计数 → 紧凑写法。≥1000 用 k，≥10k 取整。抄自 octop。 */
export function formatTokenCount(value: number): string {
  if (value >= 1000) {
    const kilo = value / 1000
    return `${kilo >= 10 ? Math.round(kilo) : kilo.toFixed(1)}k`
  }
  return String(Math.round(value))
}

export function formatMetric(key: SessionMetricKey, value: number): string {
  if (key === 'cache_hit_ratio') {
    return `${Math.round(value * 100)}%`
  }
  if (key.endsWith('_ms')) {
    return formatDurationMs(value)
  }
  if (key === 'input_tokens' || key === 'output_tokens' || key === 'cache_read_tokens') {
    return formatTokenCount(value)
  }
  if (key === 'tok_per_s') {
    return Number.isInteger(value) ? String(value) : value.toFixed(1)
  }
  if (Number.isInteger(value)) {
    return String(value)
  }
  return value.toFixed(1)
}