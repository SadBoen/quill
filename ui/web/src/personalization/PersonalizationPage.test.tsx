import { render } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it } from 'vitest'

import i18n from '../i18n'
import { PersonalizationPage } from './PersonalizationPage'

/**
 * 个性化页的判据。
 *
 * 这一页全是链接，所以它的坏处跟别处不一样：**每张卡都「能点」**，
 * 编译过、渲染得出来，几乎不能证明它是对的。真正会出事的是指错了地方 ——
 * 用户点「工具」跳到技能页，或者跳到 404，而两处都不会报错。
 *
 * 所以这里逐张卡断言 `href`，而不是断言「有六张卡」。
 */

beforeEach(async () => {
  await i18n.changeLanguage('zh')
})

function mount() {
  return render(
    <MemoryRouter>
      <PersonalizationPage />
    </MemoryRouter>,
  )
}

describe('个性化页', () => {
  it('七张卡各自指向真实存在的页面', () => {
    mount()
    const expected: Record<string, string> = {
      skills: '/skills',
      tools: '/devices',
      subagents: '/experts?tab=team',
      plugins: '/skills',
      memory: '/memory',
      channels: '/channels',
      mbti: '/mbti',
    }
    for (const [section, href] of Object.entries(expected)) {
      const link = document.querySelector(`a[data-section="${section}"]`)
      expect(link, `缺少 ${section} 卡片`).not.toBeNull()
      expect(link!.getAttribute('href'), `${section} 指错了地方`).toBe(href)
    }
  })

  it('每张卡都有说明，不是一堆光秃秃的标题', () => {
    mount()
    const cards = document.querySelectorAll('.personalization-card')
    expect(cards.length).toBe(7)
    for (const card of Array.from(cards)) {
      const p = card.querySelector('p')
      expect(p?.textContent?.trim().length ?? 0).toBeGreaterThan(8)
    }
  })

  it('MBTI 已经是真页面：卡片能点，且路由真的存在', async () => {
    mount()
    const card = document.querySelector('[data-section="mbti"]')
    expect(card).not.toBeNull()
    expect(card!.tagName).toBe('A')
    expect(card!.getAttribute('href')).toBe('/mbti')
    // 路由与侧栏都得有，否则点进去是 404 —— 卡片指对了地方不代表真有那一页。
    const app = await import('../app/App.tsx?raw')
    expect(app.default).toContain('path="/mbti"')
    const shell = await import('../layout/AppShell.tsx?raw')
    expect(shell.default).toContain("'/mbti'")
  })

  it('子智能体那张卡必须说清派工不执行', () => {
    // 这张卡是全页唯一一个「承诺会落空」的：teams 页只记账不执行
    // （BACKLOG B2-2，MemberExecutor 只有测试里的实现）。文案若写成
    // 「各自领活」，用户就会以为派出去的活真有人做。
    mount()
    const card = document.querySelector('[data-section="subagents"]')
    const text = card?.textContent ?? ''
    expect(text).toMatch(/不执行|还没做|没有实现/)
  })

  it('说明页与导航里都有入口', async () => {
    mount()
    // 侧栏入口
    const shell = await import('../layout/AppShell.tsx?raw')
    expect(shell.default).toContain('/personalization')
    // 路由
    const app = await import('../app/App.tsx?raw')
    expect(app.default).toContain('/personalization')
  })
})