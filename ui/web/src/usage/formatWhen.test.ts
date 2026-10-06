import { describe, expect, it } from 'vitest'

import { formatWhen } from './formatWhen'

/**
 * 图表轴标签与表格列共用这一个函数。
 * 两处各写一份格式化，同一个时刻就会在两处显示成两个样子 ——
 * 那类漂移比写错更隐蔽，因为它不报错。
 */

describe('formatWhen', () => {
  it('输出 MM-DD HH:mm（补零，月/日/时/分都不会挤在一起）', () => {
    const ts = new Date(2026, 9, 6, 9, 5).getTime()
    expect(formatWhen(ts)).toBe('10-06 09:05')
  })

  it('时间戳非法时返回空串，不显示 NaN-NaN', () => {
    expect(formatWhen(Number.NaN)).toBe('')
  })
})
