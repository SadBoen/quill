import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import type { SessionContext } from '../usage/contextApi'
import { ContextRing } from './ContextRing'

/**
 * 输入框里那个上下文小环。
 *
 * 尺寸与位置是**照 octop**，不是照我们自己的判断：
 * 32px、放在输入框里发送键旁边、点开才看明细
 * （`.octop-ref/octop/dashboard/src/pages/Chat/chatInputCore.partial.less:465`、
 * `components/ChatInputActionsRow.tsx:962-968`、`components/ContextWindowRing.tsx:348-358`）。
 *
 * 这组测试盯的是两件不能退化的：
 * 1. **没量过就不画环。** 画一个 0% 的空环等于说「还有一大半没用」，
 *    而真实情况是「完全没量过」。
 * 2. **构成明细的单位是字符。** quill 没有分词器，说成 token 就是凭空造数字。
 */

function ctx(over: Partial<SessionContext> = {}): SessionContext {
  return {
    max_tokens: 32768,
    used_tokens: 3042,
    used_percent: 9,
    segments: [
      { key: 'system_prompt', chars: 242 },
      { key: 'tool_definitions', chars: 3100 },
      { key: 'skills', chars: 0 },
      { key: 'mcp', chars: 0 },
      { key: 'conversation', chars: 1800 },
    ],
    segment_unit: 'chars',
    ...over,
  }
}

function renderRing(
  context: SessionContext,
  open = false,
  cache?: { cacheHit?: number | null },
) {
  const onToggle = vi.fn()
  render(
    <ContextRing
      context={context}
      open={open}
      onToggle={onToggle}
      cacheHit={cache?.cacheHit ?? null}
    />,
  )
  return { onToggle }
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

describe('上下文小环', () => {
  it('环上写的是实测占用，并带上上下限', () => {
    renderRing(ctx())
    const ring = screen.getByRole('button')
    expect(ring).toHaveTextContent('9%')
    // 3042 / 32768 压成 3k / 33k，不写成四位数。
    // 取整不是四舍五入到一位小数 —— 逐字抄 octop 的 `Math.round(n / 1000)`。
    expect(ring.getAttribute('aria-label')).toContain('3k')
    expect(ring.getAttribute('aria-label')).toContain('33k')
    expect(ring.getAttribute('aria-label')).not.toContain('3042')
  })

  it('没实测值时一个环都不画', () => {
    renderRing(ctx({ used_tokens: null, used_percent: null }))
    expect(screen.queryByRole('button')).toBeNull()
  })

  it('默认不展开：点开之前页面上没有构成明细', () => {
    renderRing(ctx())
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('展开后逐项带 `~`，一个 0 字符的段不出现', () => {
    renderRing(ctx(), true)
    const panel = screen.getByRole('dialog')
    const items = Array.from(panel.querySelectorAll('li')).map((li) => li.textContent?.replace(/\s+/g, ' ').trim())
    // 名字与数字是两个元素，靠 flex gap 分开，所以按「一行一项」来对。
    // `~` 是「这是估算」的声明（照 octop），k/M 是紧凑写法（照 formatTokenK）。
    expect(items).toEqual([
      '系统提示 ~242',
      '内置工具 ~3k',
      '对话历史 ~2k',
    ])
    // 0 字符的段（技能 / MCP）不该占一行。
    expect(panel).not.toHaveTextContent('技能')
  })

  it('不再显示「quill 没有分词器」那句解释', () => {
    // 声明「这是估算」的是 `~` 符号本身。用一整句话解释符号，
    // 等于替用户把 `~` 读一遍 —— 而那句话描述的是实现细节，不是这个数能干什么。
    //
    // 断言要能同时抓住中英两种写法：那句废话里一定带「字符」或「分词器」，
    // 两种都在 i18n 资源里。上一版这里只断言了中文，换成英文 defaultValue 就漏了。
    renderRing(ctx(), true)
    const panel = screen.getByRole('dialog')
    expect(panel).not.toHaveTextContent('分词器')
    expect(panel).not.toHaveTextContent('tokenizer')
    expect(panel).not.toHaveTextContent('字符')
  })

  it('服务端没给构成时明说没有，而不是画一个空环', () => {
    renderRing(ctx({ segments: [] }), true)
    expect(screen.getByRole('dialog')).toHaveTextContent('服务端没给出构成明细')
  })

  it('阈值变色与 octop 一致：≥80% 危险色、≥50% 警告色', () => {
    const { unmount } = render(<ContextRing context={ctx({ used_percent: 85 })} open={false} onToggle={() => {}} />)
    expect(screen.getByRole('button').className).toContain('is-danger')
    unmount()

    renderRing(ctx({ used_percent: 60 }))
    expect(screen.getByRole('button').className).toContain('is-warning')
    cleanup()
    renderRing(ctx({ used_percent: 10 }))
    expect(screen.getByRole('button').className).toContain('is-ok')
  })

  it('点一下就切换展开状态', () => {
    const onToggle = vi.fn()
    render(<ContextRing context={ctx()} open={false} onToggle={onToggle} />)
    fireEvent.click(screen.getByRole('button'))
    expect(onToggle).toHaveBeenCalledTimes(1)
  })

  /**
   * 缓存命中率搬到这里，而不是留在聊天页那排统计里。
   *
   * 判据是「没上报就不出现」：本地模型常常不报缓存 token，
   * 那种情况下这一行必须整行消失，不能显示成「缓存命中 0%」——
   * 0% 是一个断言，「没测到」不是。
   */
  describe('缓存命中率', () => {
    it('报了就显示，报的是 0% 也照样显示', () => {
      renderRing(ctx(), true, { cacheHit: 0 })
      expect(screen.getByRole('dialog')).toHaveTextContent('缓存命中 0%')
    })

    it('只报百分比，不重复报缓存读的绝对值', () => {
      // 绝对值在聊天页底部那排统计的「缓存读」格子里已经有。
      // 这里再写一遍「575 tokens 走缓存」，是同一个数在一个面板里出现两次。
      renderRing(ctx(), true, { cacheHit: 0.96 })
      const panel = screen.getByRole('dialog')
      expect(panel).toHaveTextContent('缓存命中 96%')
      expect(panel).not.toHaveTextContent('走缓存')
      expect(panel).not.toHaveTextContent('575')
    })

    it('上游没上报时整行不出现', () => {
      renderRing(ctx(), true, { cacheHit: null })
      expect(screen.getByRole('dialog')).not.toHaveTextContent('缓存命中')
    })

    it('不传这个 prop 时也不出现', () => {
      // 默认值必须是「没有」而不是 0：调用方忘了传不该凭空多出一行断言。
      render(<ContextRing context={ctx()} open onToggle={() => {}} />)
      expect(screen.getByRole('dialog')).not.toHaveTextContent('缓存命中')
    })

    it('收起来的时候看不见，得点开才有', () => {
      renderRing(ctx(), false, { cacheHit: 0.96 })
      expect(screen.queryByRole('dialog')).toBeNull()
      expect(screen.getByRole('button')).not.toHaveTextContent('缓存命中')
    })
  })
})