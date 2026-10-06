import { render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { EChart, useCssVar } from './EChart'
import type { EChartsOption } from './echarts'

/**
 * echarts 画在 canvas 上，`fillStyle` **不认 `var(--x)`**：
 * 解析失败后会回退到一个深色默认值。实测后果是「只用了 3%」的
 * 上下文环被画成一个几乎全黑的圆环，读起来像占满了 ——
 * **一张在说反话的图，比没有图更糟。**
 *
 * 所以这里钉住：交给 echarts 的颜色必须是解析后的真色值。
 */

function Probe({ name, fallback }: { name: string; fallback: string }) {
  const value = useCssVar(name, fallback)
  return <span data-testid="v">{value}</span>
}

describe('useCssVar', () => {
  it('把 CSS 变量读成真实色值，而不是把 var(--x) 原样交给 canvas', async () => {
    // jsdom 的 getComputedStyle 不加载 index.css，所以这里显式打一个变量上去。
    const root = document.documentElement
    root.style.setProperty('--quill-test-ink', '#172033')
    render(<Probe name="--quill-test-ink" fallback="#000000" />)
    await waitFor(() => expect(screen.getByTestId('v').textContent).toBe('#172033'))
    root.style.removeProperty('--quill-test-ink')
  })

  it('变量不存在时退回 fallback，不把空串交给 echarts', async () => {
    render(<Probe name="--quill-does-not-exist" fallback="#0f151e" />)
    await waitFor(() => expect(screen.getByTestId('v').textContent).toBe('#0f151e'))
  })
})

/**
 * ## 为什么下面这一段是「透传 mock」而不是「假 echarts」
 *
 * 这次改动的全部要害就三句：**init 只做一次 / 内容变了才 setOption /
 * 切语言（updateKey）也算内容变了**。这三条只能靠调用次数来证，
 * 而组件对外没有任何观测口 —— `EChart` 是个极薄的容器，不导出实例。
 *
 * 所以这里 mock 掉 `./echarts` 这一个模块，**转发到真的 echarts**：
 * `init` 真的建实例、真的画一遍（`src/test/setup.ts` 已经给 jsdom 的
 * canvas 打了桩），我们只是在旁边记一笔账。
 * 换成返回空壳的假实例，测试会变成「断言一个我们自己写的对象」，
 * init 参数、setOption 传进去的 option 全都是我们自己造的 ——
 * 那种测试通过了也说明不了真组件画得对。
 */

/** init 的真实实例上，组件只会用到这三个方法。 */
interface RealInstance {
  getDom: () => HTMLElement
  getZr: () => unknown
  setOption: (option: unknown) => unknown
  resize: () => void
  dispose: () => void
}

const probe = vi.hoisted(() => ({
  /** 每次 `echarts.init` 的参数（宿主元素 / theme / opts）。 */
  initCalls: [] as unknown[][],
  /** 每次 `setOption` 收到的 option 对象。 */
  setOptionCalls: [] as unknown[],
  disposeCount: 0,
  resizeCount: 0,
  /** 让下一次 init 抛，模拟「这个环境没有 canvas」。 */
  failNextInit: false,
}))

vi.mock('./echarts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./echarts')>()
  const realInit = actual.echarts.init as unknown as (...args: unknown[]) => RealInstance

  return {
    echarts: {
      init: (...args: unknown[]) => {
        probe.initCalls.push(args)
        if (probe.failNextInit) {
          probe.failNextInit = false
          throw new Error('test: 当前环境没有可用的 canvas')
        }
        const instance = realInit(...args)
        return {
          getDom: () => instance.getDom(),
          getZr: () => instance.getZr(),
          setOption: (option: unknown) => {
            probe.setOptionCalls.push(option)
            return instance.setOption(option)
          },
          resize: () => {
            probe.resizeCount += 1
            instance.resize()
          },
          dispose: () => {
            probe.disposeCount += 1
            instance.dispose()
          },
        }
      },
    },
  }
})

/**
 * jsdom 的 ResizeObserver 是 `src/test/setup.ts` 里的空实现，
 * 记不下 disconnect。这里换成一个记账的版本，断言「卸载时不摘挂」——
 * 漏掉 disconnect 会让被观察的 DOM 一直持有回调。
 */
class RecordingResizeObserver {
  static instances: RecordingResizeObserver[] = []
  observed: Element[] = []
  disconnected = 0
  private readonly callback: ResizeObserverCallback

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback
    RecordingResizeObserver.instances.push(this)
  }

  observe(el: Element): void {
    this.observed.push(el)
  }

  unobserve(): void {}

  disconnect(): void {
    this.disconnected += 1
  }

  /** 模拟容器尺寸变化，验证「跟着容器走」而不是跟着 window 走。 */
  emit(): void {
    this.callback([{ target: this.observed[0] } as unknown as ResizeObserverEntry], this)
  }
}

interface PieOption {
  series: Array<{ data: Array<{ value: number; itemStyle: { color: string } }> }>
}

function seriesColor(option: unknown): string {
  return (option as PieOption).series[0].data[0].itemStyle.color
}

function seriesValue(option: unknown): number {
  return (option as PieOption).series[0].data[0].value
}

/**
 * 所有调用方都把 option 写成**每次渲染新建的字面量**，
 * 所以这个 Harness 必须保持这个形状 —— 换成 `useMemo` 的 option
 * 下面一半测试就失去意义了（那正是被修掉之前的写法）。
 *
 * `unit` 故意只出现在 formatter 闭包里：闭包**源码**每次都一样，
 * 值却可能变了。这正是切语言的情形。
 */
function Harness({
  percent,
  color,
  unit,
  updateKey,
}: {
  percent: number
  color: string
  unit: string
  updateKey?: string
}) {
  const option: EChartsOption = {
    tooltip: { trigger: 'item', formatter: () => `已占用 ${percent}% · ${unit}` },
    series: [{ type: 'pie', data: [{ value: percent, itemStyle: { color } }] }],
  }
  return <EChart option={option} ariaLabel="测试上下文环" height={120} updateKey={updateKey} />
}

const BASE = { percent: 40, color: '#0af', unit: 'tokens', updateKey: undefined as string | undefined }

beforeEach(() => {
  probe.initCalls = []
  probe.setOptionCalls = []
  probe.disposeCount = 0
  probe.resizeCount = 0
  probe.failNextInit = false
  RecordingResizeObserver.instances = []
  vi.stubGlobal('ResizeObserver', RecordingResizeObserver)
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('EChart 生命周期与内容指纹', () => {
  it('父组件反复重渲染（option 每次都是新字面量）时，chart 只 init 一次', () => {
    // 这条是整次改动的第一红线。以前 option 直接进依赖，等于「每敲一个字
    // 重画两张图」：入场动画重放、hover/zoom 状态丢失、RO 反复摘挂。
    const { rerender } = render(<Harness {...BASE} />)
    for (let i = 0; i < 5; i += 1) rerender(<Harness {...BASE} />)

    expect(probe.initCalls).toHaveLength(1)
    // 指纹没变 → 连 setOption 都不该发生。
    expect(probe.setOptionCalls).toHaveLength(1)
  })

  it('option 内容真的变了才 setOption，且始终不重新 init', () => {
    const { rerender } = render(<Harness {...BASE} />)
    rerender(<Harness {...BASE} percent={80} />)
    rerender(<Harness {...BASE} percent={80} />)

    expect(probe.initCalls).toHaveLength(1)
    expect(probe.setOptionCalls).toHaveLength(2)
    // 传进去的必须是**最新**那份 option，不是第一次那份。
    expect(seriesValue(probe.setOptionCalls[1])).toBe(80)
  })

  it('主题色变了一定重画，且不重新 init', () => {
    const { rerender } = render(<Harness {...BASE} />)
    rerender(<Harness {...BASE} color="#f43f5e" />)

    expect(probe.initCalls).toHaveLength(1)
    expect(probe.setOptionCalls).toHaveLength(2)
    expect(seriesColor(probe.setOptionCalls[1])).toBe('#f43f5e')
  })

  it('只改 updateKey（切语言）也重画 —— 源码指纹看不见的那一类', () => {
    // tooltip 的 formatter 是闭包：换语言时**源码一个字符都没变**，
    // 里面的文案变了。任何基于 option 源码的指纹都不可能发现这件事，
    // 所以调用方必须把 i18n.language 当作 updateKey 传进来。
    // 下面的对照用例证明这条不是装饰品。
    const { rerender } = render(<Harness {...BASE} updateKey="zh-CN" />)
    rerender(<Harness {...BASE} updateKey="en" />)

    expect(probe.initCalls).toHaveLength(1)
    expect(probe.setOptionCalls).toHaveLength(2)
  })

  it('反过来：不传 updateKey 时，闭包里的文案变了也重画不出来（这正是需要 updateKey 的原因）', () => {
    const { rerender } = render(<Harness {...BASE} />)
    rerender(<Harness {...BASE} unit="字符" />)

    expect(probe.initCalls).toHaveLength(1)
    // 闭包源码没变 → 指纹相同 → 不重画。这不是 bug，是内容指纹的边界，
    // 边界之外靠调用方的 updateKey 兜。
    expect(probe.setOptionCalls).toHaveLength(1)
  })

  it('把 updateKey 改成同一个值不算内容变化（否则每次渲染都会重画）', () => {
    const { rerender } = render(<Harness {...BASE} updateKey="zh-CN" />)
    rerender(<Harness {...BASE} updateKey="zh-CN" />)

    expect(probe.setOptionCalls).toHaveLength(1)
  })

  it('容器尺寸变化时 resize 的是同一个实例', () => {
    render(<Harness {...BASE} />)
    const ro = RecordingResizeObserver.instances[0]
    expect(ro.observed).toHaveLength(1)

    ro.emit()
    ro.emit()
    expect(probe.resizeCount).toBe(2)
    expect(probe.initCalls).toHaveLength(1)
  })

  it('卸载时 dispose 掉实例并摘掉 ResizeObserver，不泄漏', () => {
    const { unmount } = render(<Harness {...BASE} />)
    const ro = RecordingResizeObserver.instances[0]

    unmount()

    expect(probe.disposeCount).toBe(1)
    expect(ro.disconnected).toBe(1)
  })

  it('没有 canvas 时退化成只有 aria-label 的占位，不把整页带崩', () => {
    // init 抛异常如果不接住，图表这个角落能把整页白掉。
    probe.failNextInit = true
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    render(<Harness {...BASE} />)

    // 无障碍文本还在 —— 那正是图表该提供的可读信息。
    expect(screen.getByRole('img', { name: '测试上下文环' })).toBeInTheDocument()
    expect(probe.setOptionCalls).toHaveLength(0)
    expect(warn).toHaveBeenCalled()
    warn.mockRestore()
  })

  it('init 失败后再恢复：重新挂载仍能画出来', () => {
    probe.failNextInit = true
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const first = render(<Harness {...BASE} />)
    first.unmount()
    warn.mockRestore()

    render(<Harness {...BASE} />)

    expect(probe.initCalls).toHaveLength(2)
    expect(probe.setOptionCalls).toHaveLength(1)
  })
})

/**
 * 主题切换走的是真实链路：`data-theme` 属性变更 → useCssVar 的
 * MutationObserver → state 变化 → option 里换颜色 → 指纹变化 → 重画。
 * jsdom 的 getComputedStyle 不认 CSS 自定义属性，所以只有「读变量」
 * 这一步打桩；属性变更与重渲染都是真的。
 */
function ThemeHarness() {
  const ink = useCssVar('--quill-theme-ink', '#f8fafc')
  const option: EChartsOption = {
    series: [{ type: 'pie', data: [{ value: 40, itemStyle: { color: ink } }] }],
  }
  return <EChart option={option} ariaLabel="主题色环" height={120} />
}

describe('EChart 与主题切换', () => {
  const computed = window.getComputedStyle.bind(window)

  function stubThemeVariable(): void {
    vi.spyOn(window, 'getComputedStyle').mockImplementation((el, pseudo) => {
      const real = computed(el, pseudo)
      const ink =
        document.documentElement.getAttribute('data-theme') === 'dark' ? '#0b1220' : '#f8fafc'
      return new Proxy(real, {
        get(target, prop, receiver) {
          if (prop === 'getPropertyValue') {
            return (name: string) =>
              name === '--quill-theme-ink' ? ink : target.getPropertyValue(name)
          }
          return Reflect.get(target, prop, receiver)
        },
      })
    })
  }

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('切主题时颜色进到 option 里并触发重画，全程只 init 一次', async () => {
    stubThemeVariable()
    render(<ThemeHarness />)
    await waitFor(() => expect(probe.setOptionCalls).toHaveLength(1))
    expect(seriesColor(probe.setOptionCalls[0])).toBe('#f8fafc')

    document.documentElement.setAttribute('data-theme', 'dark')

    await waitFor(() => expect(probe.setOptionCalls).toHaveLength(2))
    expect(seriesColor(probe.setOptionCalls[1])).toBe('#0b1220')
    expect(probe.initCalls).toHaveLength(1)
  })
})
