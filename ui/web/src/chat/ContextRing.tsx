import type { CSSProperties, ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { SessionContext } from '../usage/contextApi'
import { contextSegmentColor, contextSegmentLabel } from '../usage/ContextWindowChart'
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
}: {
  context: SessionContext
  open: boolean
  onToggle: () => void
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
          used: String(used),
          max: String(context.max_tokens),
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
              used: String(used),
              max: String(context.max_tokens),
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
                  {/* 单位必须跟着数走：quill 没有分词器，说成 token 就是凭空造数字。 */}
                  <b>
                    {s.chars} {t('chat.contextRing.charUnit', { defaultValue: '字符' })}
                  </b>
                </li>
              ))}
            </ul>
          )}
          <p className="chat-context-panel-note">
            {t('usage.charsNotTokens', { defaultValue: '按字符数，不是 token 数（quill 没有分词器）' })}
          </p>
        </div>
      ) : null}
    </div>
  )
}