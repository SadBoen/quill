import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ?raw 把样式文件当字符串读进来（jsdom 不加载 CSS，算不了样式）。
import skillsCardCss from './skills-card.css?raw'

import i18n from '../i18n'
import { SkillsPage } from './SkillsPage'
import { enabledSummaryChars, type Skill } from './skillsApi'

/**
 * 技能页的两条红线：
 *
 * 1. **`enabled` 与 `model_can_see` 必须分开显示。** 只显示「已启用」是在
 *    说假话 —— 开关开了但磁盘正文被删了，模型这一轮照样调不到，界面上
 *    却看着一切正常，用户没有任何迹象能想到原因。
 * 2. **常驻开销必须摆在眼前。** 开关谁都能点，而每个启用技能的摘要都跟着
 *    每一轮请求走（ISSUE-036 实测 13 个技能就把 8192 的窗口顶爆了）。
 */

function skill(over: Partial<Skill> = {}): Skill {
  return {
    slug: 'alpha',
    tool_name: 'alpha',
    description: '一份做法。',
    version: '0.1.0',
    source: 'local',
    kind: 'workspace',
    enabled: false,
    path: '/tmp/skills/alpha.md',
    tool_allowlist: [],
    content_chars: 3867,
    model_sees_summary: '一份做法。',
    model_can_see: false,
    ...over,
  }
}

function renderPage(skills: Skill[], onPatch?: (slug: string, body: unknown) => void): void {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (init?.method === 'PATCH') {
        onPatch?.(url.split('/').pop() ?? '', JSON.parse(String(init.body ?? '{}')))
      }
      return new Response(JSON.stringify({ skills }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    }),
  )
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={client}>
      <SkillsPage />
    </QueryClientProvider>,
  )
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
  localStorage.clear()
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('卡片与列表两种看法', () => {
  it('默认是卡片，切换器在', async () => {
    renderPage([skill({ slug: 'one' })])
    await waitFor(() => expect(screen.getByText('one')).toBeInTheDocument())
    expect(screen.getByTestId('skills-view-card')).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByTestId('skills-view-list')).toHaveAttribute('aria-pressed', 'false')
  })

  it('切到列表后同一个技能还在，开关照样能用', async () => {
    const seen: [string, unknown][] = []
    renderPage([skill({ slug: 'beta' })], (slug, body) => seen.push([slug, body]))
    await waitFor(() => expect(screen.getByText('beta')).toBeInTheDocument())

    fireEvent.click(screen.getByTestId('skills-view-list'))
    expect(screen.getByTestId('skills-view-list')).toHaveAttribute('aria-pressed', 'true')

    // 少一种能力就等于告诉用户「切到列表就不能改了」，而那不是真的。
    const box = screen.getByTestId('skills-toggle-beta') as HTMLInputElement
    box.click()
    await waitFor(() => expect(seen.length).toBe(1))
    expect(seen[0]).toEqual(['beta', { enabled: true }])
  })

  /**
   * 卡片形态里，两句警告必须**常显**。
   *
   * 这一条挡过一次真错：第一版把「打开开关也没用」折进了默认收起的
   * `<details>`，于是这句话默认看不见 —— 而它是这句话唯一的出现位置，
   * 等于被删了。能折起来的只有占地方的摘要正文。
   */
  it('卡片形态下，没有摘要的警告常显，不折进展开区', async () => {
    renderPage([
      skill({ slug: 'mute', model_sees_summary: '', content_chars: 0 }),
      skill({ slug: 'ghost', enabled: true, model_can_see: false, not_mounted_reason: '正文不在磁盘上' }),
    ])
    await waitFor(() => expect(screen.getByText('mute')).toBeInTheDocument())

    // 关键不是「文本在不在 DOM 里」—— testing-library 查得到收起的
    // `<details>` 内部内容，但**用户看不见**。要断言的是这两句话的祖先里
    // 没有收起的 details：摘要正文可以折，警告不行。
    for (const warning of [
      within(screen.getByText('mute').closest('.skill-card') as HTMLElement).getByText(/打开开关也不会让模型知道/),
      within(screen.getByText('ghost').closest('.skill-card') as HTMLElement).getByText(/正文不在磁盘上/),
    ]) {
      const foldedAncestor = warning.closest('details:not([open])')
      expect(
        foldedAncestor,
        '警告被折进了默认收起的区域，用户根本看不见',
      ).toBeNull()
    }
  })

  it('卡片形态下有摘要时，那段正文折起来（它占地方但不影响用户做什么）', async () => {
    renderPage([skill({ slug: 'talky', model_sees_summary: '这是一段摘要。' })])
    await waitFor(() => expect(screen.getByText('talky')).toBeInTheDocument())
    const card = screen.getByText('talky').closest('.skill-card') as HTMLElement
    const details = within(card).getByText('模型这一轮看到什么').closest('details') as HTMLDetailsElement
    expect(details).not.toBeNull()
    expect(details.open).toBe(false)
  })

  it('偏好被记住，键归技能包自己名下', async () => {
    renderPage([skill({ slug: 'one' })])
    await waitFor(() => expect(screen.getByText('one')).toBeInTheDocument())
    fireEvent.click(screen.getByTestId('skills-view-list'))
    expect(localStorage.getItem('quill:skills:view')).toBe('list')
  })

  /**
   * 两种看法都必须能改开关。
   *
   * 第一版只测了卡片：把卡片里那份 `SkillActions` 删掉，测试照样全绿 ——
   * 因为当时那张卡就是默认看到的。少一种能力等于告诉用户「切到列表就不能改了」。
   */
  it.each(['card', 'list'] as const)('%s 形态下开关都在且可用', async (view) => {
    const seen: [string, unknown][] = []
    if (view === 'list') localStorage.setItem('quill:skills:view', 'list')
    renderPage([skill({ slug: 'sw', enabled: false })], (slug, body) => seen.push([slug, body]))
    await waitFor(() => expect(screen.getByText('sw')).toBeInTheDocument())

    const box = screen.getByTestId('skills-toggle-sw') as HTMLInputElement
    expect(box).toBeInTheDocument()
    box.click()
    await waitFor(() => expect(seen.length).toBe(1))
    expect(seen[0]).toEqual(['sw', { enabled: true }])
  })

  /**
   * 动作区是两种看法共用的组件，所以它身上不能带任何形态专属的类名。
   *
   * 这一条挡过一次真错：`.skill-card-actions` 里写着 `margin-top: auto` 和
   * `border-top`，那是「贴在卡片底边」的做法。列表行里同一个类名就把一条竖线
   * 画在了「挂进对话工具表」旁边，而那列还被正文挤到只剩几十像素，字折成两行。
   * 界面上看就是：列表视图整体是坏的。jsdom 不加载 CSS，所以这里断言的是
   * 组件结构，样式那条由下一个判据守。
   */
  it.each(['card', 'list'] as const)('%s 形态下动作区都用同一个共用类名', async (view) => {
    if (view === 'list') localStorage.setItem('quill:skills:view', 'list')
    renderPage([skill({ slug: 'act' })])
    await waitFor(() => expect(screen.getByText('act')).toBeInTheDocument())

    const row = screen.getByText('act').closest(view === 'list' ? '.skills-row' : '.skill-card') as HTMLElement
    const actions = within(row).getByTestId('skills-toggle-act').closest('div') as HTMLElement
    expect(actions.className.trim().split(/\s+/)).toContain('skills-actions')
  })

  /**
   * 「贴到卡底、画分隔线」只属于卡片，样式里必须带 `.skill-card` 限定。
   *
   * jsdom 不算样式，所以这条直接读 CSS 源文件：`.skills-actions` 的无条件
   * 规则里不许出现 border-top / margin-top。这正是上面那个 bug 的形状 ——
   * 一条只对一种看法成立、却写成了无条件规则的声明。
   */
  it('卡片专属的贴底样式不会漏到共用的动作区上', async () => {
    const css = skillsCardCss
    // 只看无条件的 `.skills-actions { ... }` 块（后面紧跟 `{` 的那个）。
    const shared = /^\.skills-actions \{([^}]*)\}/m.exec(css)
    expect(shared, '.skills-actions 的无条件规则不见了').not.toBeNull()
    expect(shared![1], '贴底/分隔线是卡片专属的，不能写在共用的动作区上').not.toMatch(
      /border-top|margin-top/,
    )
    // 而卡片自己那份必须有，否则卡片底部会没有那条线。
    expect(css).toMatch(/\.skill-card \.skills-actions \{[^}]*border-top/)
  })
})

describe('技能包页', () => {
  it('空列表说清是空，不显示「全部正常」的假象', async () => {
    renderPage([])
    await waitFor(() => expect(screen.getByText('还没有安装任何技能包。')).toBeInTheDocument())
    expect(screen.queryByRole('list')).toBeNull()
  })

  it('启用的技能要同时说明「模型这一轮看得见」', async () => {
    renderPage([skill({ slug: 'on', enabled: true, model_can_see: true })])
    await waitFor(() => expect(screen.getByText('on')).toBeInTheDocument())
    const row = screen.getByText('on').closest('li') as HTMLElement
    expect(within(row).getByText('已启用')).toBeInTheDocument()
    expect(within(row).getByText('模型这一轮看得见')).toBeInTheDocument()
  })

  it('开关开着但模型看不见时，必须说清原因而不是只显示「已启用」', async () => {
    // 这是这一页最要紧的一条：正文文件被删了，工具表里就没有它，
    // 而界面上若只显示「已启用」，用户会一直以为模型能用。
    renderPage([
      skill({ slug: 'ghost', enabled: true, model_can_see: false, content_missing: true }),
    ])
    await waitFor(() => expect(screen.getByText('ghost')).toBeInTheDocument())
    const row = screen.getByText('ghost').closest('li') as HTMLElement
    expect(within(row).getByText('已启用 · 模型看不见')).toBeInTheDocument()
    expect(within(row).getByText('正文文件不在了')).toBeInTheDocument()
    expect(within(row).getByText(/磁盘上找不到正文文件/)).toBeInTheDocument()
    // 关键：不能出现任何声称它可用的说法
    expect(within(row).queryByText('模型这一轮看得见')).toBeNull()
  })

  it('被否决的技能把服务端给的原因原样显示', async () => {
    renderPage([
      skill({
        slug: 'clash',
        enabled: true,
        model_can_see: false,
        not_mounted_reason: '与已有工具同名，挂进去会顶掉它',
      }),
    ])
    await waitFor(() => expect(screen.getByText('clash')).toBeInTheDocument())
    const row = screen.getByText('clash').closest('li') as HTMLElement
    expect(within(row).getByText(/与已有工具同名，挂进去会顶掉它/)).toBeInTheDocument()
  })

  it('常驻开销摆在列表上方，让开开关之前就知道代价', async () => {
    renderPage([
      skill({ slug: 'a', enabled: true, model_can_see: true, model_sees_summary: '一二三四五' }),
      skill({ slug: 'b', enabled: false, model_sees_summary: '不计入' }),
    ])
    await waitFor(() => expect(screen.getByText('a')).toBeInTheDocument())
    // 只数启用中的：停用的不占常驻开销。
    expect(screen.getByText(/模型每一轮都会看到的说明合计 5 字符/)).toBeInTheDocument()
  })

  it('开销数的是模型真正看到的那段字，不是库里那个空 description 列', () => {
    // 真机上 116 个技能的 description 列**全是空的**，而 `skill_summary`
    // 会退回正文开头。拿空列去算会得到「116 个技能 · 常驻开销 0 字符」——
    // 一个既好看又危险的说法：让人以为可以随便开，而模型每轮都在吃这些字。
    const skills = [
      skill({ slug: 'a', enabled: true, description: '', model_sees_summary: '一二三' }),
      skill({ slug: 'b', enabled: false, description: '', model_sees_summary: '停用不计入' }),
    ]
    expect(enabledSummaryChars(skills)).toBe(3)
  })

  it('description 为空时显示模型真正看到的摘要，而不是说「没有摘要」', async () => {
    renderPage([
      skill({
        slug: 'fallback',
        description: '',
        model_sees_summary: '# 标题\n\n这是从正文开头退回来的摘要。',
        enabled: true,
        model_can_see: true,
      }),
    ])
    await waitFor(() => expect(screen.getByText('fallback')).toBeInTheDocument())
    const row = screen.getByText('fallback').closest('li') as HTMLElement
    expect(within(row).getByText(/这是从正文开头退回来的摘要/)).toBeInTheDocument()
    expect(within(row).queryByText(/没有摘要/)).toBeNull()
  })

  it('连模型能看到的说明都没有时，明说打开开关也没用', async () => {
    renderPage([skill({ slug: 'mute', description: '', model_sees_summary: '', content_chars: 0 })])
    await waitFor(() => expect(screen.getByText('mute')).toBeInTheDocument())
    const row = screen.getByText('mute').closest('li') as HTMLElement
    expect(within(row).getByText(/打开开关也不会让模型知道它是干什么的/)).toBeInTheDocument()
  })

  it('切换开关走 PATCH，且只带 enabled 这一个字段', async () => {
    const seen: [string, unknown][] = []
    renderPage([skill({ slug: 'toggle-me', enabled: false })], (slug, body) => seen.push([slug, body]))
    await waitFor(() => expect(screen.getByText('toggle-me')).toBeInTheDocument())

    const box = screen.getByTestId('skills-toggle-toggle-me') as HTMLInputElement
    expect(box.checked).toBe(false)
    box.click()
    await waitFor(() => expect(seen.length).toBe(1))
    expect(seen[0][0]).toBe('toggle-me')
    // 只准带 enabled。多带一个字段都可能被后端当成人想改别的。
    expect(seen[0][1]).toEqual({ enabled: true })
  })

  it('删除要先二次确认，不点确认就不发请求', async () => {
    const calls: string[] = []
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        if (init?.method === 'DELETE') calls.push(String(input))
        return new Response(JSON.stringify({ skills: [skill({ slug: 'doomed' })] }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        })
      }),
    )
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={client}>
        <SkillsPage />
      </QueryClientProvider>,
    )
    await waitFor(() => expect(screen.getByText('doomed')).toBeInTheDocument())

    // 第一下只是亮出确认，不该发任何请求。
    const del = screen.getByTestId('skills-delete-doomed')
    del.click()
    await waitFor(() => expect(screen.getByText(/这一步没法撤销/)).toBeInTheDocument())
    expect(calls).toHaveLength(0)

    const confirm = screen.getByText('确认删除')
    confirm.click()
    await waitFor(() => expect(calls.length).toBe(1))
    expect(calls[0]).toContain('/api/extensions/skills/doomed')
  })
})
