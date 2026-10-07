import type { CSSProperties, ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { SessionContext } from '../usage/contextApi'
import { contextSegmentColor, contextSegmentLabel, formatTokens } from '../usage/ContextWindowChart'
import './ContextRing.css'

/**
 * 输入框里那个上下文小环。
 *
 * **尺寸和位置照 octop，不是照我们自己的判断**：
 * - 32×32 的圆环：`/octop dashboard/src/pages/Chat/chatInputCore.partial.less:465`
 *   （`.contextRingSvg { width: 32px; height: 32px; }`，r=13、strokeWidth=3，
 *   见 `components/ContextWindowRing.tsx:273-295`）。
 * - 放在**输入框内部**、发送键旁边：`components/ChatInputActionsRow.tsx:962-968` 的
 *   `inputActions` 一栏里，不是贴在输入框下面。
 * - 明细点开才看：同文件 `:348-358` 是个 `trigger="click"` 的 Popover。
 *
 * 我们先前把它做成输入框下面一整块图表（两个 168px 的圆环），既不是 octop 的尺寸
 * 也不是它的位置：一个刚说了一句话的会话占 2%，却让那块图表吃掉近 200px 高，
 * 挤掉的全是正文。
 *
 * **goose 是另一套**：根本没有圆环，`ui/desktop/src/components/ChatInput.tsx:727-739`
 * 只在输入框里放一条细进度条，配 75%/90% 两级告警色和一个「压缩」按钮。
 * 两家都不在输入框下面摆图表，所以那块地方我们不该摆。
 */
export function ContextRing({
  context,
  open,
  onToggle,
  cacheHit = null,
}: {
  context: SessionContext
  open: boolean
  onToggle: () => void
  /** 会话级缓存命中率。null = 上游没上报缓存 token，这时整行不出现。 */
  cacheHit?: number | null
}): ReactNode {
  const { t } = useTranslation()

  // 没量过就不画。画一个 0% 的空环等于说「还有一大半没用」，
  // 而真实情况是「完全没量过」—— 那句提示由调用方负责说。
  if (context.used_tokens === null || context.used_percent === null) return null

  const percent = Math.min(Math.max(context.used_percent, 0), 100)
  const used = Math.min(context.used_tokens, context.max_tokens)
  // 阈值与 octop 一致（ContextWindowRing.tsx:150-154）：≥80% 转危险色，≥50% 转警告色。
  const tone = percent >= 80 ? 'danger' : percent >= 50 ? 'warning' : 'ok'

  // `segments` 在类型里是必填，但服务端回 200 时未必带这个键。
// 缺了就当「没有构成明细」—— 展开面板会明说，而不是整页崩在 .filter 上。
const segments = (context.segments ?? []).filter((s) => s.chars > 0)

  return (
    <div className="chat-context-wrap">
      <button
        type="button"
        className={`chat-context-ring is-${tone}`}
        onClick={onToggle}
        aria-expanded={open}
        aria-label={t('chat.contextRing.label', {
          used: formatTokens(used),
          max: formatTokens(context.max_tokens),
          percent,
          defaultValue: '上下文已占用 {{used}} / {{max}} tokens（{{percent}}%），点开看构成',
        })}
        // 环用 conic-gradient 画：32px 的圈起一个 echarts canvas 实例不划算，
        // 而这里要的只是一个单值的比例环。
        style={{ '--chat-ring-deg': `${percent * 3.6}deg` } as CSSProperties}
      >
        <span className="chat-context-ring-inner">{percent}%</span>
      </button>

      {open ? (
        <div
          className="chat-context-panel"
          role="dialog"
          aria-label={t('chat.contextRing.panel', { defaultValue: '上下文构成' })}
        >
          <p className="chat-context-panel-head">
            {t('chat.contextRing.usedOf', {
              used: formatTokens(used),
              max: formatTokens(context.max_tokens),
              percent,
              defaultValue: '已占用 {{used}} / {{max}} tokens（{{percent}}%）',
            })}
          </p>
          {segments.length === 0 ? (
            <p className="chat-context-panel-empty">
              {t('chat.contextRing.noSegments', { defaultValue: '服务端没给出构成明细。' })}
            </p>
          ) : (
            <ul className="chat-context-panel-list">
              {segments.map((s) => (
                <li key={s.key}>
                  <span className="chat-context-dot" style={{ background: contextSegmentColor(s.key) }} aria-hidden />
                  {/* 名字后面那个空格不是排版用的：读屏软件与复制都靠它分词，
                      flex gap 只管眼睛看到的。 */}
                  <span className="chat-context-panel-name">
                    {contextSegmentLabel(s.key, t)}{' '}
                  </span>
                  {/* 前缀 `~` 跟着 octop（ContextWindowRing.tsx:254）：
                      总占用是模型端实测的 token，分段只是本地量的相对构成。
                      `~` 就是「别把这个当准值」。压成 k/M 同理，逐字抄 formatTokenK。 */}
                  <b>
                    ~{formatTokens(s.chars)}
                  </b>
                </li>
              ))}
            </ul>
          )}
          {/* 缓存命中率放这里，不放聊天页那排统计里：它和上下文占用是同一件事的
              两面 —— 缓存读省掉的是钱，占掉的是窗口。分开放等于让用户
              自己把两个不同页面上的数乘起来才知道省了多少。

              只报百分比，不报绝对值：绝对值在用量统计页的表格里有。
              这里再写一遍「575 tokens 走缓存」是同一个数出现两次。 */}
          {cacheHit !== null ? (
            <p className="chat-context-panel-cache">
              {t('chat.contextRing.cacheHit', {
                percent: Math.round(cacheHit * 100),
                defaultValue: '缓存命中 {{percent}}%',
              })}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}