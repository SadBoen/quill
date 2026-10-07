import { describe, expect, it } from 'vitest'

import { ApiError } from '../api/client'
import { pickVisibleError, routeMissing } from './capability'

function apiError(status: number, code: string): ApiError {
  // 参数顺序跟着 ApiError 的构造函数：`(status, code, message, nextStep?)`。
  // 第一次写成 `(message, status, code)`，于是 status 拿到的是一段中文、
  // code 拿到的是数字，断言红了 —— 是夹具写错了，不是判定写错了。
  return new ApiError(status, code, '请求失败')
}

describe('routeMissing：判「这台实例没这个能力」', () => {
  it('404 + not_found → 没登记（自动化页据此不画那个表单）', () => {
    expect(routeMissing(apiError(404, 'not_found'))).toBe(true)
  })

  it('404 但错误码不是 not_found → 不算能力缺失', () => {
    // 那是个案（某个资源不存在），不是整个功能没接。
    // 误判成「没这个能力」会把还在正常工作的功能藏起来。
    expect(routeMissing(apiError(404, 'entity_not_found'))).toBe(false)
  })

  it('别的 4xx/5xx → 不算能力缺失', () => {
    expect(routeMissing(apiError(401, 'unauthorized'))).toBe(false)
    expect(routeMissing(apiError(403, 'forbidden'))).toBe(false)
    expect(routeMissing(apiError(500, 'internal'))).toBe(false)
    expect(routeMissing(apiError(429, 'rate_limited'))).toBe(false)
  })

  it('不是 ApiError（网络断了、解析炸了）→ 不算能力缺失', () => {
    // 拿不准就返回 false：多画一个坏表单比把能用的功能藏起来要好。
    expect(routeMissing(new Error('Failed to fetch'))).toBe(false)
    expect(routeMissing(null)).toBe(false)
    expect(routeMissing(undefined)).toBe(false)
    expect(routeMissing('字符串错误')).toBe(false)
  })

  it('反向钉死：一个恒真的实现会被上面四条抓住', () => {
    // 「永远说没这个能力」会把所有正常页面都清空 —— 包括真的能用的那些。
    const alwaysMissing = () => true
    expect(alwaysMissing()).toBe(true) // 恒真
    expect(routeMissing(apiError(500, 'internal'))).toBe(false) // 但真实现不是
  })
})

describe('pickVisibleError：你刚点的那个操作，优先于一直在报的后台错误', () => {
  it('没有错误 → null（不渲染错误条）', () => {
    expect(pickVisibleError(null, null, null)).toBe(null)
  })

  it('动作错误优先于后台常驻错误', () => {
    // 这就是那个静默失败的机制：`cron.error` 在 /api/cron 没登记时永远非空，
    // `a ?? b` 会把它排在前面，于是用户刚点的「创建」失败被一条无关旧提示挡住。
    const action = apiError(404, 'not_found')
    const background = apiError(500, 'internal')
    expect(pickVisibleError(action, background)).toBe(action)
  })

  it('没有动作错误时，后台错误仍然显示（不是把问题藏起来）', () => {
    const background = apiError(500, 'internal')
    expect(pickVisibleError(null, background)).toBe(background)
  })

  it('跳过的是 null/undefined，不是「所有 falsy」', () => {
    // 用 `a || b` 写会把 0 / '' 这类合法值也吞掉。
    expect(pickVisibleError(0, '后面这条')).toBe(0)
    expect(pickVisibleError(undefined, '后面这条')).toBe('后面这条')
  })
})