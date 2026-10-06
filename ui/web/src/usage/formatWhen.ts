/**
 * 会话时间的显示格式，图表轴标签与表格列**共用同一个函数**。
 *
 * 为什么不各写一份：一旦两处各有一份格式化，图上的 `10-06 10:35` 和
 * 表里的 `10月6日 10:35` 就会对不上，用户会以为是两个不同的时刻。
 * 这类「同一个事实有两种写法」的漂移，比写错更隐蔽。
 *
 * 时间值全部来自 `/api/usage` 返回的 `last_active_at`（排序用的也是它），
 * **没有任何自己编的时刻**。
 */

import i18n, { normalizeLanguage } from '../i18n'

/**
 * `2026-10-06T10:35:00Z` → `10-06 10:35`。时间戳非法时返回空串。
 *
 * 取值全部交给 `Intl.DateTimeFormat`，不再用 `pad()` 手工拼 `MM-DD HH:mm` ——
 * 手写拼接既拿不到当前语言，又得自己保证补零。这里从 `formatToParts` 里取回
 * 四段数值再定宽拼接，是为了让**轴标签仍然定宽**（图表上等宽才排得齐），
 * 而不是让每个语言各写一种分隔符：`Intl` 的 `zh-CN` 会给 `10/06`。
 * 语言取自 i18n 单例，因为这个模块是普通 `.ts`，用不了 `useTranslation`。
 */
export function formatWhen(ts: number): string {
  const d = new Date(ts)
  if (Number.isNaN(d.getTime())) return ''
  const parts = new Intl.DateTimeFormat(normalizeLanguage(i18n.language), {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    // 写死 h23 而不是 `hour12: false`。后者按 ECMA-402 允许实现选 h23 **或** h24，
    // 选到 h24 的引擎会把午夜印成 `24:35`；原来手写的 `getHours()` 永远是 0-23。
    // 这里显式钉死，免得同一份数据换个浏览器就换个写法。
    hourCycle: 'h23',
  }).formatToParts(d)
  const field = (type: Intl.DateTimeFormatPartTypes): string => parts.find((part) => part.type === type)?.value ?? ''
  return `${field('month')}-${field('day')} ${field('hour')}:${field('minute')}`
}
