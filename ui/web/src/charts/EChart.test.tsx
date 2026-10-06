import { render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { useCssVar } from './EChart'

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
