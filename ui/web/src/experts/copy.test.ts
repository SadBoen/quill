import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { copyText } from './ExpertsUi'

const realClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard')

function stubClipboard(writeText: unknown): void {
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
}

beforeEach(() => {
  document.execCommand = vi.fn(() => true) as unknown as typeof document.execCommand
})

afterEach(() => {
  if (realClipboard) Object.defineProperty(navigator, 'clipboard', realClipboard)
  vi.useRealTimers()
})

it('剪贴板 API 正常时直接用它', async () => {
  const writeText = vi.fn(() => Promise.resolve())
  stubClipboard(writeText)
  await expect(copyText('ai-coding-coach')).resolves.toBe('clipboard')
  expect(writeText).toHaveBeenCalledWith('ai-coding-coach')
})

it('剪贴板被拒时降级到 execCommand，不静默失败', async () => {
  stubClipboard(vi.fn(() => Promise.reject(new Error('NotAllowedError'))))
  await expect(copyText('x')).resolves.toBe('fallback')
})

// 权限提示既不批准也不拒时 writeText 的 promise 永远挂着。
// 挂起 = 点了没反应，所以必须超时后自己走兜底。
it('剪贴板 promise 永远挂着时超时降级，不让按钮变成死的', async () => {
  vi.useFakeTimers()
  stubClipboard(vi.fn(() => new Promise<void>(() => {})))
  const pending = copyText('x')
  await vi.advanceTimersByTimeAsync(700)
  await expect(pending).resolves.toBe('fallback')
})

it('两条路都不通时必须报 failed，不能假装成功', async () => {
  stubClipboard(undefined)
  document.execCommand = vi.fn(() => false) as unknown as typeof document.execCommand
  await expect(copyText('x')).resolves.toBe('failed')
})

it('没有剪贴板 API 时也能走兜底', async () => {
  stubClipboard(undefined)
  await expect(copyText('y')).resolves.toBe('fallback')
  expect(document.execCommand).toHaveBeenCalledWith('copy')
})
