import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router-dom'

import { Card, ErrorNotice, PageHeader } from '../components/Page'
import {
  METRIC_KEYS,
  formatMetric,
  visibleMetrics,
  type SessionMetricKey,
  type SessionMetrics,
} from '../chat/sessionMetrics'
import { loadUsage, type UsageReport } from '../chat/chatApi'
import { UsageCharts } from './UsageCharts'
import { ContextWindowChart } from './ContextWindowChart'
import { formatWhen } from './formatWhen'
import './usage.css'

/**
 * 用量统计页。
 *
 * 指标条本身（`SessionMetricsBar`）是给单个会话用的；这一页是**跨会话**的视角：
 * 一行总计 + 每个会话一行，点进去就是那个会话。
 *
 * 这一页最要紧的一条规矩，和统计条完全一样：
 * **没测到的指标整列显示「—」，不显示 0。**
 * 所以每一格都走 `visibleMetrics()`，服务端给 `null` 的列就是空的。
 */

/** 每一格的表头（中文）与指标键的映射。 */
const COLUMN_LABEL: Record<SessionMetricKey, string> = {
  turns: '轮次',
  steps: '回复',
  llm_duration_ms: '模型耗时',
  tool_duration_ms: '工具耗时',
  ttft_avg_ms: '首字延迟',
  tok_per_s: 'tok/s',
  cache_hit_ratio: '缓存命中',
  input_tokens: '入参',
  output_tokens: '出参',
  cache_read_tokens: '缓存读',
}

function cellText(metrics: SessionMetrics, key: SessionMetricKey): string {
  const found = visibleMetrics(metrics).find((m) => m.key === key)
  // 「—」是 quill 约定的「没记这一项」，不是 0。
  return found ? formatMetric(key, found.value) : '—'
}

function MetricsTable({ report }: { report: UsageReport }): ReactNode {
  const { t } = useTranslation()
  return (
    <div className="usage-table-scroll">
      <table className="usage-table">
        <caption className="field-help">
          {t('usage.dashMeaning', { defaultValue: '「—」表示 quill 没有记录这一项，不是 0。' })}
        </caption>
        <thead>
          <tr>
            <th scope="col">{t('usage.colSession', { defaultValue: '会话' })}</th>
            <th scope="col">{t('usage.colLastActive', { defaultValue: '最后活动' })}</th>
            <th scope="col">{t('usage.colExpert', { defaultValue: '角色' })}</th>
            {METRIC_KEYS.map((key) => (
              <th scope="col" key={key}>
                {COLUMN_LABEL[key]}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          <tr className="usage-row usage-row-total">
            <th scope="row">{t('usage.totalRow', { defaultValue: '合计' })}</th>
            <td>—</td>
            <td>—</td>
            {METRIC_KEYS.map((key) => (
              <td key={key} data-metric={key}>
                {cellText(report.totals, key)}
              </td>
            ))}
          </tr>
          {report.sessions.map((session) => (
            <tr className="usage-row" key={session.id}>
              <th scope="row">
                <Link to={`/chat/${session.id}`}>
                  {/* 没起过标题的会话在侧栏里叫「新对话」。这里跟着叫同一个名字，
                      而不是把 32 位 hex 甩给用户 —— 那串字符对谁都没用。 */}
                  {session.title || t('nav.newChat', { defaultValue: '新对话' })}
                </Link>
              </th>
              {/* 时间取自 /api/usage 本来就返回的 last_active_at（排序用的也是它），
                  这样图上每根柱子都能对回表里这一行 —— 侧栏里这些会话全都叫「新对话」，
                  只看名字分不出谁是谁。 */}
              <td className="usage-when">{formatWhen(session.last_active_at)}</td>
              <td>
                {session.expert_id || (
                  <span className="field-help">{t('usage.noExpert', { defaultValue: '未绑定角色' })}</span>
                )}
              </td>
              {METRIC_KEYS.map((key) => (
                <td key={key} data-metric={key}>
                  {cellText(session.metrics, key)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export function UsagePage(): ReactNode {
  const { t } = useTranslation()
  const usage = useQuery({ queryKey: ['usage'], queryFn: loadUsage, staleTime: 15_000 })

  return (
    <div className="page-scroll">
      <PageHeader
        title={t('usage.title', { defaultValue: '用量统计' })}
        description={t('usage.description', {
          defaultValue:
            '来自 GET /api/usage。每一格都是服务端实测值；quill 没记录的那一项显示「—」，不会拿 0 顶替。',
        })}
      />
      <div className="settings-stack">
        <ErrorNotice error={usage.error} />
        {usage.isPending ? (
          <p className="page-status">{t('common.loading', { defaultValue: '加载中…' })}</p>
        ) : null}
        {usage.data ? (
          <>
            {usage.data.truncated ? (
              <Card tone="danger" title={t('usage.truncatedTitle', { defaultValue: '统计被截断' })}>
                <p className="empty-card-copy">
                  {t('usage.truncated', {
                    count: usage.data.limit,
                    defaultValue:
                      '这里只统计了最近 {{count}} 个会话，更早的没有计入。把「最近 {{count}} 个」当成「全部」是一句凭空而来的话。',
                  })}
                </p>
              </Card>
            ) : null}
            <Card
              title={t('usage.chartsTitle', { defaultValue: '图表' })}
              description={t('usage.chartsDescription', {
                defaultValue: '全部来自服务端实测值；没测到的项不进图，下面表格里显示「—」。',
              })}
            >
              <UsageCharts report={usage.data} />
            </Card>
            {usage.data.sessions[0] ? (
              <Card
                title={t('usage.contextTitle', { defaultValue: '最近一个会话的上下文窗口' })}
                description={t('usage.contextDescription', {
                  defaultValue: '环是实测 token；构成是字符数（quill 没有分词器）。',
                })}
              >
                <ContextWindowChart sessionId={usage.data.sessions[0].id} context={null} />
              </Card>
            ) : null}
            <Card
              title={t('usage.bySession', { defaultValue: '按会话' })}
              description={t('usage.bySessionDescription', {
                count: usage.data.session_count,
                defaultValue: '共 {{count}} 个会话。',
              })}
            >
              {usage.data.sessions.length === 0 ? (
                <p className="empty-card-copy">
                  {t('usage.empty', { defaultValue: '还没有任何会话，先去对话页发一句话。' })}
                </p>
              ) : (
                <MetricsTable report={usage.data} />
              )}
            </Card>
          </>
        ) : null}
      </div>
    </div>
  )
}