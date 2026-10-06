import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach, vi } from 'vitest'

import i18n from '../i18n'

Object.defineProperty(window, 'matchMedia', {
  configurable: true,
  value: vi.fn().mockReturnValue({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }),
})

/**
 * jsdom 没有 canvas，而 echarts/zrender 完全建立在 2D context 上。
 *
 * 不打这个桩的话，图表组件一挂载就抛 `Cannot read properties of null
 * (reading 'clearRect')`，整棵测试树被带走 —— 结果不是「图表测不了」，
 * 而是**连图旁边的中文文案都测不了**。
 *
 * 这里给一个最小可用的 2D context：方法全是空实现，`measureText` 返回
 * 宽度 0。**它只够让 echarts 跑通绘制流程，不做任何像素断言** ——
 * 图表长什么样仍然只能靠真机看，这里测的是「数据有没有正确进到 option」。
 */
function stubCanvas(): void {
  const ctx = {
    canvas: null as unknown,
    save() {},
    restore() {},
    scale() {},
    translate() {},
    rotate() {},
    transform() {},
    setTransform() {},
    resetTransform() {},
    beginPath() {},
    closePath() {},
    moveTo() {},
    lineTo() {},
    arc() {},
    arcTo() {},
    ellipse() {},
    rect() {},
    roundRect() {},
    fill() {},
    stroke() {},
    clip() {},
    isPointInPath: () => false,
    isPointInStroke: () => false,
    fillRect() {},
    strokeRect() {},
    clearRect() {},
    fillText() {},
    strokeText() {},
    drawImage() {},
    putImageData() {},
    getImageData: () => ({ data: [] }),
    measureText: () => ({ width: 0, actualBoundingBoxAscent: 0, actualBoundingBoxDescent: 0 }),
    createLinearGradient: () => ({ addColorStop() {} }),
    createRadialGradient: () => ({ addColorStop() {} }),
    createConicGradient: () => ({ addColorStop() {} }),
    createPattern: () => null,
    setLineDash() {},
    getLineDash: () => [],
  }
  HTMLCanvasElement.prototype.getContext = vi.fn(() => ctx) as never
  HTMLCanvasElement.prototype.toDataURL = () => 'data:image/png;base64,'
}

/** jsdom 同样没有 ResizeObserver，而图表靠它跟随容器尺寸。 */
function stubResizeObserver(): void {
  class NoopResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  globalThis.ResizeObserver = NoopResizeObserver as never
}

stubCanvas()
stubResizeObserver()

afterEach(async () => {
  cleanup()
  window.localStorage.clear()
  document.documentElement.removeAttribute('data-theme')
  await i18n.changeLanguage('en')
})