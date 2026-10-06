import { useTranslation } from 'react-i18next'
import './SessionMetricsBar.css'
import {
  formatMetric,
  metricLabel,
  visibleMetrics,
  type SessionMetricKey,
  type SessionMetrics,
} from './sessionMetrics'

/**
 * 会话级 Token 统计条。
 *
 * 界面照抄 octop 的 `TrajectoryMetricsBar.tsx`：一排 `标签 值` 的 chip，
 * 用 `·` 分隔，贴在会话底部。
 *
 * 两处刻意的不同：
 * 1. octop 那个组件自己带 `agentId`/`threadId` 两个没用上的 prop，
 *    这里不抄 —— 抄进来就是两个误导性的死参数。
 * 2. `metrics` 为 null 时不渲染。octop 渲染空 div，这里直接不出现：
 *    一排什么都没有的统计条比没有统计条更让人困惑。
 */
export default function SessionMetricsBar({ metrics }: { metrics: SessionMetrics | null }) {
  const { t } = useTranslation()
  const entries = metrics ? visibleMetrics(metrics) : []

  if (entries.length === 0) return null

  return (
    <div
      className="chat-metrics-bar"
      role="group"
      aria-label={t('chat.metricsBar.label', { defaultValue: '会话统计' })}
    >
      {entries.map((entry, index) => (
        <span className="chat-metrics-chip-group" key={entry.key}>
          {index > 0 ? (
            <span className="chat-metrics-sep" aria-hidden>
              ·
            </span>
          ) : null}
          <span className="chat-metrics-chip" data-metric={entry.key}>
            {chipText(entry.key, entry.value, metricLabel(entry.key, t))}
          </span>
        </span>
      ))}
    </div>
  )
}

/**
 * 每个 chip 的文案模板。
 *
 * 抄 octop 的 `chipText()`：轮次类「3 轮」把数放前面，tok/s 把单位放后面，
 * 其余一律「标签 值」。中文里轮次不加量词复数，所以直接复用这套排布。
 */
function chipText(key: SessionMetricKey, value: number, label: string): string {
  const text = formatMetric(key, value)
  // 轮次与速率把数字放前面：「3 轮次」「12.5 tok/s」读起来比反过来顺，
  // 这是照抄 octop 的 chipText()。
  if (key === 'turns' || key === 'steps' || key === 'tok_per_s') {
    return `${text} ${label}`
  }
  return `${label} ${text}`
}