import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { PersonalizationPage } from './PersonalizationPage'

/**
 * 个性化页的判据。
 *
 * 2026-10-08 改过一次形态：最早是七张大卡片铺两排，用户对着 Octop 指出
 * 「是一行页签导航」。所以这里的重点不是「有七个东西」，而是
 * **顶部一行、顺序照 Octop、内容区只显示当前那一签**。
 *
 * 另外钉两处容易被悄悄弄坏的：
 * - 页签状态在 URL 上（`?tab=`），这样「个性化 / 人格」能直接发给别人；
 * - 只有一个人格页签的内嵌页面，其余六签给的是**真链接**，不是死内容区。
 */

/** Octop 的页签顺序（index.tsx:33-41）。顺序不是随手排的。 */
const UPSTREAM_ORDER = [
  'skills',
  'subagents',
  'tools',
  'plugins',
  'mbti',
  'memory',
  'channels',
]

/** 每签的出口：只有人格是内嵌的，其余都在自己的页面。 */
const LINKS: Record<string, string> = {
  skills: '/skills',
  subagents: '/experts?tab=team',
  tools: '/devices',
  plugins: '/skills',
  memory: '/memory',
  channels: '/channels',
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

function stubFetch() {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input)
      if (url.includes('/api/mbti/questions')) {
        return json({ questions: [], min_answers: 20 })
      }
      if (url.includes('/api/mbti/history')) return json({ history: [], current: null, keep: 20 })
      if (url.includes('/api/experts')) return json({ experts: [] })
      return json({})
    }),
  )
}

function mount(initial = '/personalization') {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <MemoryRouter initialEntries={[initial]}>
      <QueryClientProvider client={qc}>
        <PersonalizationPage />
      </QueryClientProvider>
    </MemoryRouter>,
  )
}

function tabs(): HTMLElement[] {
  return Array.from(document.querySelectorAll('[role="tab"]')) as HTMLElement[]
}

beforeEach(async () => {
  await i18n.changeLanguage('zh')
  vi.unstubAllGlobals()
})

describe('个性化页', () => {
  it('顶部是一行七个页签，不是七张大卡片', () => {
    stubFetch()
    mount()
    expect(tabs()).toHaveLength(7)
    // 一行 = 横向排布，且没有卡片网格。判据盯容器而不是猜视觉。
    const row = document.querySelector('.personalization-tabs')
    expect(row, '页签行容器必须在').not.toBeNull()
    expect(row!.querySelectorAll('a'), '页签里不能有链接').toHaveLength(0)
    // 七项必须都是容器的**直接子元素** —— 一旦中间又套一层，
    // 「一个容器包住七项」的结构就没了。
    expect(row!.children).toHaveLength(7)
    // 旧形态的卡片网格必须已经不在了
    expect(document.querySelector('.personalization-grid')).toBeNull()
    expect(document.querySelectorAll('.personalization-card')).toHaveLength(0)
  })

  it('页签是一个整体容器（Octop 用的是 antd Segmented，不是 Tabs）', async () => {
    // 上游 layouts/PageShell.tsx:52-73 渲染的是 Segmented：一个带边框的容器
    // 把各项包住，选中项在容器内高亮。第一版写成「七项各自一个带边框的按钮」，
    // 看着像 segmented，其实结构不是 —— 这条就是钉那个差别。
    //
    // vitest 不注入 CSS（css: false），所以 computed style 拿不到真值。
    // 于是读样式表原文断言关键声明 —— 至少能抓住「容器没边框」
    // 与「每项又各自带边框」这两种回退。
    stubFetch()
    mount()
    const row = document.querySelector('.personalization-tabs')
    expect(row?.getAttribute('data-segmented')).toBe('true')

    const css = await import('./personalization.css?raw')
    const block = (sel: string): string => {
      const i = css.default.indexOf(sel + ' {')
      if (i < 0) return ''
      return css.default.slice(i, css.default.indexOf('}', i))
    }
    const container = block('.personalization-page .personalization-tabs')
    expect(container, '找不到页签容器的样式块').not.toBe('')
    expect(container).toMatch(/border:\s*1px solid/)
    expect(container).toMatch(/border-radius:/)

    const item = block('.personalization-page .personalization-tab')
    expect(item, '找不到单个页签的样式块').not.toBe('')
    expect(item, '单个页签不该再自带边框 —— 边框属于容器').not.toMatch(/(^|[\s;])border:\s*1px/)
    expect(item).toMatch(/font-size:\s*13px/)
  })

  it('页签顺序与 Octop 一致', () => {
    stubFetch()
    mount()
    // 逐个点过去看 aria-selected，顺序错一位这里就露馅。
    for (const key of UPSTREAM_ORDER) {
      const tab = tabs().find((b) => b.getAttribute('id') === `p-tab-${key}`)
      expect(tab, `缺少 ${key} 页签`).toBeTruthy()
    }
    const order = tabs().map((b) => (b.id as string).replace('p-tab-', ''))
    expect(order).toEqual(UPSTREAM_ORDER)
  })

  it('默认落在技能，且标题跟着页签走', () => {
    stubFetch()
    mount()
    expect(tabs()[0].getAttribute('aria-selected')).toBe('true')
    // 上游把标题拼成「个性化 / 技能」（index.tsx:92）
    expect(document.querySelector('.page-header h1')?.textContent ?? '').toContain('技能')
    expect(document.querySelector('[role="tabpanel"]')?.getAttribute('data-section')).toBe(
      'skills',
    )
  })

  it('点页签就切内容区，且只有当前签被选中', () => {
    stubFetch()
    mount()
    // 按 id 找，不按下标 —— 下标会随页签顺序调整而错位，
    // 那种错位会让判据在「顺序改对了」时反而报红。
    const memory = tabs().find((b) => b.id === 'p-tab-memory')!
    fireEvent.click(memory)
    const panel = document.querySelector('[role="tabpanel"]')
    expect(panel?.getAttribute('data-section')).toBe('memory')
    expect(tabs().filter((b) => b.getAttribute('aria-selected') === 'true')).toHaveLength(1)
    expect(memory.getAttribute('aria-selected')).toBe('true')
    // 非当前签不该同时铺出来
    expect(document.querySelector('[data-section="skills"]')).toBeNull()
  })

  it('页签状态活在 URL 上，可以直接分享', async () => {
    stubFetch()
    mount('/personalization?tab=channels')
    await waitFor(() =>
      expect(document.querySelector('[role="tabpanel"]')?.getAttribute('data-section')).toBe(
        'channels',
      ),
    )
    expect(
      tabs().find((b) => b.id === 'p-tab-channels')!.getAttribute('aria-selected'),
    ).toBe('true')
  })

  it('认不出来的 tab 参数落回第一个，而不是渲染一个空白页', () => {
    stubFetch()
    mount('/personalization?tab=no-such-tab')
    expect(tabs()[0].getAttribute('aria-selected')).toBe('true')
    expect(document.querySelector('[role="tabpanel"]')?.getAttribute('data-section')).toBe(
      'skills',
    )
  })

  it('每个非人格页签都给出真链接，没有指向空处的按钮', () => {
    stubFetch()
    for (const [key, href] of Object.entries(LINKS)) {
      document.body.innerHTML = ''
      mount(`/personalization?tab=${key}`)
      const link = document.querySelector(`[data-section-link="${key}"]`)
      expect(link, `${key} 没有给出口`).not.toBeNull()
      expect(link!.getAttribute('href'), `${key} 指错了地方`).toBe(href)
    }
  })

  it('人格页签内嵌真的测评页，不是文字占位', async () => {
    stubFetch()
    mount('/personalization?tab=mbti')
    // 内嵌的是真页面：它会去取题库和历史
    await waitFor(() =>
      expect(
        Array.from(document.querySelectorAll('[role="tabpanel"]')).some((p) =>
          p.textContent?.includes('开始测评'),
        ),
      ).toBe(true),
    )
    // 也不能同时再画一份「打开人格」链接 —— 那一签就是它自己
    expect(document.querySelector('[data-section-link="mbti"]')).toBeNull()
  })

  it('子智能体那一签必须说清派工不执行', () => {
    // 这一签是全页唯一一个「承诺会落空」的：teams 页只记账不执行
    // （BACKLOG B2-2）。文案若写成「各自领活」，用户就会以为派出去的活真有人做。
    stubFetch()
    mount('/personalization?tab=subagents')
    const panel = document.querySelector('[role="tabpanel"]')
    expect(panel?.textContent ?? '').toMatch(/不执行|还没做|没有实现/)
  })

  it('路由与侧栏里都有入口', async () => {
    stubFetch()
    mount()
    const app = await import('../app/App.tsx?raw')
    expect(app.default).toContain('path="/personalization"')
    // 人格仍然有自己的独立地址：内嵌之外还要能直接进
    expect(app.default).toContain('path="/mbti"')
    const shell = await import('../layout/AppShell.tsx?raw')
    expect(shell.default).toContain("'/personalization'")
    expect(shell.default).toContain("'/mbti'")
  })
})