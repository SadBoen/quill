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

const pad = (n: number): string => String(n).padStart(2, '0')

/** `2026-10-06T10:35:00Z` → `10-06 10:35`。时间戳非法时返回空串。 */
export function formatWhen(ts: number): string {
  const d = new Date(ts)
  if (Number.isNaN(d.getTime())) return ''
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}
