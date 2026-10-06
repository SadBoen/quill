import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { EChart } from '../charts/EChart'
import type { UsageReport } from '../chat/chatApi'
import { contextSegmentLabel } from './ContextWindowChart'
import { formatWhen } from './formatWhen'

/**
 * 用量统计页的图表，全部画**真实数据**。
 *
 * 每张图都遵守同一条规矩：**只画服务端给了真值的项。**
 * 服务端回 `null` 的指标不进图 —— 在图上画一条 0 高度的柱子，
 * 等于宣称「这一项是 0」，而真实情况是「没测到」。
 * 所以图下方一律配 `—` 表格，让用户看到到底哪几格是真的。
 */

/** 只保留服务端报过值的会话。 */
function withData(report: UsageReport) {
  return report.sessions.filter(
    (s) => s.metrics.input_tokens !== null || s.metrics.output_tokens !== null,
  )
}

/**
 * 轴标签。**必须与下方表格用同一个兜底名** ——
 * 表里显示「新对话」而图上显示 8 位 hex，用户会以为是两个不同的东西。
 */
function shortTitle(title: string, fallback: string): string {
  const t = title.trim()
  if (!t) return fallback
  return t.length > 8 ? `${t.slice(0, 8)}…` : t
}

/**
 * 轴标签的第二行：会话最后活动时间。
 *
 * ## 为什么必须加这一行
 *
 * 侧栏里所有会话都没起标题，兜底名一律是「新对话」。于是图上会出现
 * **八根并排的「新对话」** —— 看得见、却分不出谁是谁，图就废了。
 *
 * 时间取的是 `/api/usage` **本来就返回**的 `last_active_at`（`ORDER BY`
 * 就是按它排的），没有新增字段、没有自己编一个序号称过去。
 * 有了它，图上的每一根都能对回表格里的那一行。
 */
function axisLabel(title: string, fallback: string, lastActiveAt: number): string {
  const name = shortTitle(title, fallback)
  const when = formatWhen(lastActiveAt)
  return when ? `${name}\n${when}` : name
}

/** 入参 / 出参 按会话的堆叠柱状图。 */
function TokensBySession({ report }: { report: UsageReport }): ReactNode | null {
  const { t } = useTranslation()
  const untitled = t('nav.newChat', { defaultValue: '新对话' })
  const rows = withData(report)
  if (rows.length === 0) return null

  // 图表在窄容器里会挤成一团，超过 12 个会话就只画最近 12 个，
  // 并在图例旁写明「还有多少个没画」—— 少画不说明就等于骗人。
  const shown = rows.slice(0, 12)
  const hidden = rows.length - shown.length

  const option = {
    tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
    legend: { bottom: 0, itemWidth: 10, itemHeight: 10, textStyle: { fontSize: 11 } },
    grid: { left: 48, right: 16, top: 16, bottom: 66 },
    xAxis: {
      type: 'category',
      data: shown.map((s) => axisLabel(s.title, untitled, s.last_active_at)),
      axisLabel: { fontSize: 10, interval: 0, lineHeight: 13, rotate: rows.length > 6 ? 30 : 0 },
    },
    yAxis: { type: 'value', axisLabel: { fontSize: 10 } },
    series: [
      {
        name: t('usage.chartInput', { defaultValue: '入参' }),
        type: 'bar',
        stack: 'tokens',
        itemStyle: { color: '#5b5fef' },
        data: shown.map((s) => s.metrics.input_tokens ?? null),
      },
      {
        name: t('usage.chartOutput', { defaultValue: '出参' }),
        type: 'bar',
        stack: 'tokens',
        itemStyle: { color: '#22d3ee' },
        data: shown.map((s) => s.metrics.output_tokens ?? null),
      },
    ],
  }

  return (
    <figure className="usage-chart-wide">
      <figcaption>
        {t('usage.tokensBySession', { defaultValue: '各会话 token 用量' })}
        {hidden > 0 ? (
          <small className="field-help">
            {t('usage.chartTruncated', {
              hidden,
              defaultValue: '只画了最近 {{hidden}} 个会话，更早的在下方表格里',
            })}
          </small>
        ) : null}
      </figcaption>
      <EChart
        option={option}
        height={260}
        ariaLabel={t('usage.tokensBySessionAria', {
          detail: shown
            .map((s) => `${shortTitle(s.title, untitled)} 入参 ${s.metrics.input_tokens ?? '—'}`)
            .join('，'),
          defaultValue: '各会话 token 用量：{{detail}}',
        })}
      />
    </figure>
  )
}

/** 输出速度按会话。只画测到耗时的那些。 */
function SpeedBySession({ report }: { report: UsageReport }): ReactNode | null {
  const { t } = useTranslation()
  const untitled = t('nav.newChat', { defaultValue: '新对话' })
  const rows = report.sessions.filter((s) => s.metrics.tok_per_s !== null)
  if (rows.length === 0) return null
  const shown = rows.slice(0, 12)

  const option = {
    // valueFormatter 是必须的：不加的话鼠标一悬停，
    // 气泡里就是 10.561850329222233 这种除法原始值。
    tooltip: { trigger: 'axis', valueFormatter: (v: number) => `${v.toFixed(1)} tok/s` },
    grid: { left: 48, right: 16, top: 16, bottom: 66 },
    xAxis: {
      type: 'category',
      data: shown.map((s) => axisLabel(s.title, untitled, s.last_active_at)),
      axisLabel: { fontSize: 10, interval: 0, lineHeight: 13, rotate: shown.length > 6 ? 30 : 0 },
    },
    yAxis: {
      type: 'value',
      name: 'tok/s',
      nameTextStyle: { fontSize: 10 },
      axisLabel: { fontSize: 10 },
    },
    series: [
      {
        type: 'line',
        smooth: true,
        symbolSize: 6,
        itemStyle: { color: '#e879f9' },
        data: shown.map((s) => s.metrics.tok_per_s),
      },
    ],
  }

  return (
    <figure className="usage-chart-wide">
      <figcaption>
        {t('usage.speedBySession', { defaultValue: '各会话输出速度' })}
      </figcaption>
      <EChart
        option={option}
        height={220}
        ariaLabel={t('usage.speedBySessionAria', {
          // 必须格式化：tok_per_s 是除法结果，直接插值会变成
          // 「10.561850329222233 tok/s」。aria-label 是要念给人听的。
          detail: shown
            .map((s) => `${shortTitle(s.title, untitled)} ${(s.metrics.tok_per_s ?? 0).toFixed(1)} tok/s`)
            .join('，'),
          defaultValue: '各会话输出速度：{{detail}}',
        })}
      />
    </figure>
  )
}

/** 缓存命中率。只有上报过缓存 token 的会话才有值。 */
function CacheHitBySession({ report }: { report: UsageReport }): ReactNode | null {
  const { t } = useTranslation()
  const untitled = t('nav.newChat', { defaultValue: '新对话' })
  const rows = report.sessions.filter((s) => s.metrics.cache_hit_ratio !== null)
  if (rows.length === 0) return null

  const option = {
    tooltip: { trigger: 'axis', valueFormatter: (v: number) => `${Math.round(v * 100)}%` },
    grid: { left: 48, right: 16, top: 16, bottom: 66 },
    xAxis: {
      type: 'category',
      data: rows.map((s) => axisLabel(s.title, untitled, s.last_active_at)),
      axisLabel: { fontSize: 10, interval: 0, lineHeight: 13, rotate: rows.length > 6 ? 30 : 0 },
    },
    yAxis: {
      type: 'value',
      max: 1,
      axisLabel: { fontSize: 10, formatter: (v: number) => `${Math.round(v * 100)}%` },
    },
    series: [
      {
        type: 'bar',
        itemStyle: { color: '#fbbf24' },
        data: rows.map((s) => s.metrics.cache_hit_ratio),
      },
    ],
  }

  return (
    <figure className="usage-chart-wide">
      <figcaption>
        {t('usage.cacheBySession', { defaultValue: '各会话缓存命中率' })}
        <small className="field-help">
          {t('usage.cacheOnlyReported', {
            defaultValue: '只包含模型端上报过缓存 token 的会话',
          })}
        </small>
      </figcaption>
      <EChart
        option={option}
        height={220}
        ariaLabel={t('usage.cacheBySessionAria', {
          detail: rows
            .map((s) => `${shortTitle(s.title, untitled)} ${Math.round((s.metrics.cache_hit_ratio ?? 0) * 100)}%`)
            .join('，'),
          defaultValue: '各会话缓存命中率：{{detail}}',
        })}
      />
    </figure>
  )
}

export { contextSegmentLabel }

export function UsageCharts({ report }: { report: UsageReport }): ReactNode {
  const { t } = useTranslation()
  const anyChart =
    withData(report).length > 0 ||
    report.sessions.some((s) => s.metrics.tok_per_s !== null) ||
    report.sessions.some((s) => s.metrics.cache_hit_ratio !== null)

  if (!anyChart) {
    return (
      <p className="usage-chart-note">
        {t('usage.noCharts', {
          defaultValue: '还没有可画的用量数据 —— 发一句话让模型跑一轮，这里就会有图。',
        })}
      </p>
    )
  }

  return (
    <div className="usage-charts">
      <TokensBySession report={report} />
      <SpeedBySession report={report} />
      <CacheHitBySession report={report} />
    </div>
  )
}