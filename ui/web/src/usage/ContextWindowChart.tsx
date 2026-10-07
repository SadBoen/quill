import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorNotice } from '../components/Page'
import { EChart, useCssVar } from '../charts/EChart'
import type { SessionContext } from './contextApi'
import { useSessionContext } from './useSessionContext'

/**
 * 上下文窗口图。抄 octop 的 `ContextWindowRing`，但用 echarts 而不是手搓 SVG。
 *
 * 一张图说两件事：
 * - **环形**：上下文占了多少（实测 token）。
 * - **构成条**：这些 token 是被谁吃掉的（按字符数，相对构成）。
 *
 * ## 两条不能越过的线
 *
 * 1. **环的总占用是实测的**：`used_tokens` 来自模型端真实上报的
 *    `input_tokens`。不用「字符数 ÷ 4」这种估算 —— 那样环上的百分比
 *    就是一个凭空的数字。
 * 2. **构成段是估算值，前缀 `~`。** 服务端给的是字符数，quill 没有分词器，
 *    所以它只能是估算。照 octop（`ContextWindowRing.tsx:254`）在每个值前
 *    面加 `~`，靠符号本身声明这件事 —— 而不是用一整句话解释 `~` 是什么，
 *    那等于替用户把符号读一遍。
 *
 * 两者的关系和 octop 一致：环的宽度由实测值决定，构成段只提供**相对比例**。
 */

const SEGMENT_COLOR: Record<string, string> = {
  system_prompt: '#9ca3af',
  tool_definitions: '#c4b5fd',
  skills: '#fbbf24',
  mcp: '#e879f9',
  conversation: '#22d3ee',
}

export function contextSegmentColor(key: string): string {
  return SEGMENT_COLOR[key] ?? '#94a3b8'
}

export function contextSegmentLabel(key: string, t: (k: string, o: Record<string, unknown>) => string): string {
  const map: Record<string, string> = {
    system_prompt: t('usage.segSystem', { defaultValue: '系统提示' }),
    tool_definitions: t('usage.segTools', { defaultValue: '内置工具' }),
    skills: t('usage.segSkills', { defaultValue: '技能' }),
    mcp: t('usage.segMcp', { defaultValue: 'MCP' }),
    conversation: t('usage.segConversation', { defaultValue: '对话历史' }),
  }
  return map[key] ?? key
}

/**
 * 大数压成 `k` / `M`，逐字抄 octop 的 `formatTokenK`
 * （`.octop-ref/octop/dashboard/src/pages/Chat/components/ContextWindowRing.tsx:26-30`）。
 *
 * 不压的话一个 128k 窗口要写成「131072」，五位数在面板里既占地方又要用户
 * 自己在脑子里换算。三位数以下原样显示 —— 压成「0.2k」反而更难读。
 *
 * **导出而不留在本文件**：聊天页的小环面板与用量页的图表要显示同一批数，
 * 两处各抄一份就会出现「环上 3.1k、图上 3100」。
 */
export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1000) return `${Math.round(n / 1000)}k`
  return String(n)
}

export function ContextWindowChart({
  sessionId,
  context,
}: {
  sessionId: string
  context: SessionContext | null
}): ReactNode {
  const { t, i18n } = useTranslation()

  // canvas 不认 CSS 变量，必须读成真色值（见 useCssVar 的说明）。
  const cInk = useCssVar('--ink', '#172033')
  const cLine = useCssVar('--line', '#dce5f0')
  const cSuccess = useCssVar('--success', '#087562')
  const cWarning = useCssVar('--warning', '#805a08')
  const cDanger = useCssVar('--danger', '#a43f48')

  const query = useSessionContext(sessionId)
  const data = context ?? query.data

  // 「没测到」和「没拉到」是两件不同的事，**两件都得说出来**。
  // 之前这里只有 `if (!data) return null`，于是 400/500/404 与「还没量过」
  // 在界面上长得一模一样 —— 用 `/usage` 的人以为这一格是空的，
  // 用 `/chat` 的人以为环「自己消失了」。所以失败要把错误原样说出来。
  if (!data) {
    return query.isError ? <ErrorNotice error={query.error} /> : null
  }

  // 没跟模型说过话 → 没有实测值。**这里必须显式说明，不能画一个 0% 的空环**：
  // 「0%」看着像「还有一大半没用」，实际是「完全没量过」。
  if (data.used_tokens === null) {
    return (
      <p className="usage-chart-note">
        {t('usage.contextUnmeasured', {
          defaultValue: '上下文占用：还没跟模型通过话，没有实测值。',
        })}
      </p>
    )
  }

  const used = data.used_tokens
  const percent = data.used_percent ?? 0
  const ringColor = percent >= 80 ? cDanger : percent >= 50 ? cWarning : cSuccess

  const ringOption = {
    tooltip: {
      trigger: 'item',
      formatter: () =>
        `${t('usage.contextRingTip', {
          used: formatTokens(used),
          max: formatTokens(data.max_tokens),
          percent,
          defaultValue: '已占用 {{used}} / {{max}} tokens（{{percent}}%）',
        })}`,
    },
    series: [
      {
        type: 'pie',
        radius: ['72%', '92%'],
        center: ['50%', '50%'],
        avoidLabelOverlap: false,
        // 百分比只挂在「已占用」那一段上。两段都配 center label 的话，
        // 剩余那段会拿同一个百分比再画一遍，文字叠在文字上。
        label: {
          show: true,
          position: 'center',
          formatter: `${percent}%`,
          fontSize: 15,
          fontWeight: 600,
          color: cInk,
        },
        labelLine: { show: false },
        emphasis: { label: { show: true } },
        data: [
          {
            value: Math.min(used, data.max_tokens),
            itemStyle: { color: ringColor },
            label: { show: true },
          },
          {
            value: Math.max(data.max_tokens - used, 0),
            itemStyle: { color: cLine },
            label: { show: false },
          },
        ],
      },
    ],
  }

  // 构成段按字符数。相对比例来自字符占比，总宽度由上面的实测环决定。
  const segData = data.segments
    .filter((s) => s.chars > 0)
    .map((s) => ({
      name: contextSegmentLabel(s.key, t),
      value: s.chars,
      itemStyle: { color: contextSegmentColor(s.key) },
    }))

  const compositionOption = {
    tooltip: {
      trigger: 'item',
      formatter: (p: { name: string; value: number; percent: number }) =>
        t('usage.segTip', {
          name: p.name,
          value: formatTokens(p.value),
          percent: p.percent,
          defaultValue: '{{name}}：~{{value}}（{{percent}}%）',
        }),
    },
    legend: { bottom: 0, itemWidth: 10, itemHeight: 10, textStyle: { fontSize: 11 } },
    series: [
      {
        type: 'pie',
        radius: ['38%', '62%'],
        center: ['50%', '44%'],
        label: { show: false },
        data: segData,
      },
    ],
  }

  // 单位跟 tooltip 走同一个 key：这一页唯一说「估算」的地方不允许出现两种
  // 写法，更不允许在英文界面里蹦出中文单位。
  // 提前算好：`.i18n-check.mjs` 靠括号配平抓参数，嵌在外层 t() 里的 t() 会被
  // 误认成「多传了 name / value」。
  const segDetail = segData
    .map((s) =>
      t('usage.segItem', {
        name: s.name,
        value: s.value,
        defaultValue: '{{name}} ~{{value}}',
      }),
    )
    .join('，')

  const ringAria = t('usage.contextRingAria', {
    used: formatTokens(used),
    max: formatTokens(data.max_tokens),
    percent,
    defaultValue: '上下文已占用 {{used}} / {{max}} tokens，占 {{percent}}%',
  })

  return (
    <div className="usage-chart-pair">
      <figure className="usage-chart">
        <figcaption>{t('usage.contextRing', { defaultValue: '上下文占用' })}</figcaption>
        <EChart
          option={ringOption}
          height={168}
          // 环的文案藏在 formatter 闭包里，语言变了源码没变 —— 必须把语言交给
          // EChart，否则切语言后 tooltip 还是旧文案。
          updateKey={i18n.language}
          ariaLabel={ringAria}
        />
      </figure>
      <figure className="usage-chart">
        <figcaption>
          {t('usage.contextComposition', { defaultValue: '上下文构成' })}
          {/* 原来这里挂着一句「按字符数，不是 token 数（quill 没有分词器）」。
              现在每个分段值前面带 `~`（照 octop），那才是「这是估算」的声明；
              再用一整句解释 `~` 是什么意思，等于替用户把符号读一遍。 */}
          <small className="field-help">
            {t('usage.contextCompositionEstimate', {
              defaultValue: '分段为估算值（~），总占用是模型端实测',
            })}
          </small>
        </figcaption>
        <EChart
          option={compositionOption}
          height={168}
          updateKey={i18n.language}
          ariaLabel={t('usage.contextCompositionAria', {
            detail: segDetail,
            defaultValue: '上下文构成：{{detail}}',
          })}
        />
      </figure>
    </div>
  )
}