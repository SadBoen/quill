import { useEffect, useRef, useState } from 'react'

import { echarts, type EChartsOption } from './echarts'

/**
 * 极薄的 echarts 容器。
 *
 * 只做三件必须做的事，其余交给 echarts：
 * 1. 挂载时 init、卸载时 dispose —— 漏掉 dispose 会泄漏 canvas 与事件监听。
 * 2. option **内容**变化时 setOption（不是重新 init）。
 * 3. 容器尺寸变化时 resize —— 用 ResizeObserver 而不是 window.resize，
 *    因为侧栏折叠这类「窗口没变、容器变了」的情况后者听不到。
 *
 * ## 为什么要算「内容指纹」而不是直接把 option 放进依赖
 *
 * 所有调用方都把 option 写成**每次渲染新建的字面量**。把它直接放进
 * `useEffect` 依赖，等于「父组件每重渲染一次 → dispose + 重新 init」：
 * 聊天页每敲一个字就重建两张上下文图（入场动画重放、hover/zoom 状态丢失、
 * ResizeObserver 被反复摘挂）。
 *
 * 所以这里：init 只在挂载时做一次，实例活到卸载；每次渲染算一遍内容指纹，
 * **只有内容真的变了才 setOption**。指纹里含颜色与数据，所以主题切换
 * （useCssVar 改 state）一定会触发重画；翻译藏在 formatter 闭包里看不出来，
 * 由调用方用 `updateKey` 传当前语言补上。
 *
 * ## 为什么 init 要 try/catch
 *
 * echarts 依赖 canvas。在 jsdom（测试）以及任何禁用了 canvas 的环境里，
 * `init` 会抛。**如果不接住，一个画不出图的角落会连带白屏整页** ——
 * 图表是锦上添花，不该有能力把整页带走。
 *
 * 所以这里退化成「只有 aria-label 的占位」：无障碍文本还在（那正是
 * chart 该提供的可读信息），图形没了。真浏览器里不会走到这个分支，
 * 真走到了说明环境有问题 —— 打个 warn，不装作没事。
 *
 * 刻意不封装更多：再往上包一层，最后总会变成一个谁都不敢动的黑盒。
 */

/**
 * 把 CSS 变量读成**真实色值**。
 *
 * ## 为什么需要它
 *
 * echarts 画在 canvas 上，canvas 的 `fillStyle` 只认色值字符串，
 * **不认 `var(--line)`** —— 交给它的效果是解析失败后回退到一个
 * 深色默认色。实测后果是「上下文只用了 3%」的环被画成一个全黑的圆环，
 * 读起来像占满了。**这是一张在说反话的图**，比没有图更糟。
 *
 * 这里读的是 `documentElement` 上的计算值，所以颜色仍然跟着
 * `html[data-theme="dark"]` 走，不需要在 JS 里再抄一份明暗两套色板
 * —— 抄一份就意味着以后改主题会漏改一处。
 */
export function useCssVar(name: string, fallback: string): string {
  const [value, setValue] = useState(fallback)

  useEffect(() => {
    const read = () => {
      const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
      // 空字符串说明这个变量不存在（比如 jsdom 里没加载 index.css）。
      // 这时候必须退回 fallback，不能把空串交给 echarts。
      if (v) setValue(v)
    }
    read()
    // 主题是改 <html data-theme>，不冒泡也不进任何子元素订阅，
    // 所以 ResizeObserver 之类的都听不到，只能盯这个属性本身。
    const mo = new MutationObserver(read)
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] })
    return () => mo.disconnect()
  }, [name])

  return value
}

/**
 * option 的内容指纹 —— 「该不该重画」的判据。
 *
 * 规则：
 * - 普通值照常序列化。颜色、数据、图例名都在这里，所以主题切换一定被抓到。
 * - 函数按**源码**序列化：tooltip 的 formatter 是闭包，引用每次渲染都不同，
 *   但源码没变时没必要重画。
 * - 环引用兜底成 `[circular]`（同一份 option 里重复引用同一个对象时也会
 *   走到这里，但两次判定结果一致，不影响相等判断）。
 */
function fingerprint(option: EChartsOption): string {
  const seen = new WeakSet<object>()
  return JSON.stringify(option, (_key, value) => {
    if (typeof value === 'function') return `fn:${value.toString()}`
    if (value && typeof value === 'object') {
      if (seen.has(value)) return '[circular]'
      seen.add(value)
    }
    return value
  })
}

export function EChart({
  option,
  height = 220,
  className,
  ariaLabel,
  updateKey,
}: {
  option: EChartsOption
  height?: number
  className?: string
  /** canvas 本身对读屏软件是黑的，所以外层必须给一个可读的标签。 */
  ariaLabel: string
  /**
   * 额外参与「内容是否变了」判断的标识。
   *
   * 只有当 option 里有一处**翻译藏在闭包里**时才有意义 —— 比如 tooltip 的
   * formatter：切语言时闭包源码不变，但里面的文案变了。传当前语言即可。
   */
  updateKey?: string
}) {
  const host = useRef<HTMLDivElement>(null)
  const chart = useRef<ReturnType<typeof echarts.init> | null>(null)
  /** 上一次真正 setOption 过的内容指纹。 */
  const applied = useRef<string | null>(null)

  const stamp =
    updateKey === undefined ? fingerprint(option) : `${updateKey}\u0000${fingerprint(option)}`

  // init / dispose 只跟生命周期走。
  useEffect(() => {
    const el = host.current
    if (!el) return
    let instance: ReturnType<typeof echarts.init>
    try {
      instance = echarts.init(el, undefined, { renderer: 'canvas' })
    } catch (e) {
      console.warn('[charts] 无法初始化 echarts（当前环境没有可用的 canvas），图不显示：', e)
      return
    }
    chart.current = instance
    // 指纹记的是**上一个实例**画过的内容：新实例必须重画一次。
    applied.current = null
    const ro = new ResizeObserver(() => instance.resize())
    ro.observe(el)
    return () => {
      ro.disconnect()
      instance.dispose()
      chart.current = null
      applied.current = null
    }
  }, [])

  // 内容变了才 setOption —— 不重新 init。
  useEffect(() => {
    const instance = chart.current
    // init 失败（无 canvas）时这里是 null：退化行为和以前一样，只剩占位。
    if (!instance || applied.current === stamp) return
    applied.current = stamp
    instance.setOption(option)
  }, [option, stamp])

  return (
    <div
      ref={host}
      className={className}
      style={{ width: '100%', height }}
      role="img"
      aria-label={ariaLabel}
    />
  )
}