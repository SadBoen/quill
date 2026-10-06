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