import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import i18n from '../i18n'
import { HubList } from './HubList'
// 直接读样式表原文。**不用 `node:fs`**：应用 tsconfig 的 `types` 里
// 没有 node（`noUnusedLocals` 之外还有 `types` 白名单），引 fs 会让
// `tsc -b` 直接失败。Vite 的 `?raw` 是现成的，且类型在 `vite/client` 里。
import hubCss from './hub.css?raw'
import type { HubSkill, HubSkillSet } from './hubApi'

/**
 * 市场页要同时说准**四件事**，少一件就是在骗人：
 *
 * 1. **上游地址要显示。** 它不是内置列表，断网时用户得知道去哪儿看。
 * 2. **没连上 ≠ 市场是空的。** 上游挂了就显示挂，绝不返回一张空列表。
 * 3. **装了几个就是几个。** 实测一个包里只有一篇正文，manifest 点名的
 *    6 个下游技能**没有正文、没装** —— 不说清楚，用户会以为装漏了，
 *    或者更糟：以为那 6 个已经能用。
 * 4. **技能包与单技能是两层。** 所以是两个页签：装错层的用户会以为
 *    市场缺货，实际是他把「装一个包 = 拿到一堆技能」当成了理所当然。
 */

function hub(over: Partial<HubSkillSet> = {}): HubSkillSet {
  return {
    id: 1,
    slug: 'tech-test-automation',
    display_name: '自动化测试',
    display_name_en: '',
    summary: '从 TDD 到 E2E 的完整工作流。',
    summary_en: '',
    scene: 'tech',
    sub_scene: 'test-automation',
    content: '# 编排说明',
    skill_slugs: ['superpowers-tdd', 'test-case-generator'],
    skill_count: 2,
    icon_url: '',
    ...over,
  }
}

function skill(over: Partial<HubSkill> = {}): HubSkill {
  return {
    slug: 'pdf-image-text-extractor',
    name: 'PDF和图片文字提取',
    description: 'Extract text from images or PDF documents.',
    description_zh: '从图片或 PDF 文档中识别并提取文字内容。',
    version: '1.0.13',
    category: 'office-efficiency',
    icon_url: '',
    installs: 378,
    downloads: 900,
    ...over,
  }
}

const OK_LIST = {
  host: 'https://api.skillhub.cn',
  items: [hub()],
  total: 56,
  page: 1,
  page_size: 20,
}

const OK_RANK = {
  host: 'https://api.skillhub.cn',
  kind: 'recommended',
  section: 'recommended',
  items: [skill()],
  errors: {},
  kinds: ['all', 'recommended', 'trending', 'hot', 'featured', 'newest', 'paid'],
}

interface Bodies {
  list?: unknown
  listError?: string
  rank?: unknown
  rankError?: string
  search?: unknown
  installPack?: unknown
  installSkill?: unknown
  /** 本机已装的技能。市场卡片靠它标「已安装 / 重新安装」。 */
  installed?: { slug: string; tool_name?: string }[]
}

/** 按 URL 分派。**URL 判断要贴住真实的路径形状** ——
 *  `/skill-hub/skills/pdf-.../install` 也含 `/skill-hub/`，不区分就会走错分支。 */
function renderHub(bodies: Bodies): ReturnType<typeof render> {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      const json = (v: unknown, status = 200) =>
        new Response(JSON.stringify(v), {
          status,
          headers: { 'content-type': 'application/json' },
        })
      const fail = (msg: string) =>
        json(
          {
            error: {
              code: 'upstream_unavailable',
              detail: msg,
              next_step: '确认网络能到上游；已装好的技能不受影响。',
            },
          },
          503,
        )

      if (init?.method === 'POST') {
        return json(url.includes('/skills/') ? bodies.installSkill : bodies.installPack)
      }
      if (url.includes('/rankings')) {
        return bodies.rankError ? fail(bodies.rankError) : json(bodies.rank ?? OK_RANK)
      }
      if (url.includes('/skill-hub/skills?')) {
        return json(
          bodies.search ?? { host: 'https://api.skillhub.cn', query: '', items: [], total: null },
        )
      }
      if (url.includes('/skill-hub?')) {
        return bodies.listError ? fail(bodies.listError) : json(bodies.list ?? OK_LIST)
      }
      // 已装技能目录。**默认 0 个** —— 没有真实安装记录就不该显示「已安装」。
      // 注意键名是 `skills`（见 `skills_api::skillsOf`），不是 `items`。
      if (url.includes('/api/extensions/skills')) {
        return json({ skills: bodies.installed ?? [] })
      }
      return json({})
    }),
  )
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const view = render(
    <QueryClientProvider client={client}>
      <HubList />
    </QueryClientProvider>,
  )
  return view
}

/** 切到「技能包」页签。技能包的那些断言都在这一层。 */
async function openPackTab(): Promise<void> {
  fireEvent.click(await screen.findByTestId('hub-tab-pack'))
}

beforeEach(async () => {
  await i18n.changeLanguage('zh-CN')
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('技能市场 · 技能包', () => {
  it('显示实际连的是哪个上游 —— 它不是内置列表', async () => {
    renderHub({})
    await openPackTab()
    await waitFor(() => expect(screen.getByText('自动化测试')).toBeInTheDocument())
    expect(screen.getByText(/https:\/\/api\.skillhub\.cn/)).toBeInTheDocument()
    expect(screen.getByText(/那不影响已经装好的技能/)).toBeInTheDocument()
  })

  it('上游没连上就说没连上，不显示成「市场是空的」', async () => {
    renderHub({ listError: '技能市场没连上。' })
    await openPackTab()
    // 错误文案是服务端那句 + 「下一步：…」；这里只断言两件事分别在场。
    await waitFor(() => expect(screen.getByText(/技能市场没连上/)).toBeInTheDocument())
    // **关键**：不能说「还没有技能」—— 那会让用户以为是市场没有东西。
    expect(screen.queryByText(/市场这一次返回了 0 个/)).toBeNull()
    // `ErrorNotice` 把 next_step 单独占一行渲染（见 Page.tsx），
    // 所以这里断言的是**服务端那句建议本身**，不是「下一步：」这个前缀。
    expect(screen.getByText(/已装好的技能不受影响/)).toBeInTheDocument()
  })

  it('上游真的返回 0 个时，明说这是上游说的', async () => {
    renderHub({
      list: { host: 'https://api.skillhub.cn', items: [], total: 0, page: 1, page_size: 20 },
    })
    await openPackTab()
    await waitFor(() => expect(screen.getByText(/上游说的/)).toBeInTheDocument())
  })

  it('包点名的下游技能只算「引用」，不算「已装」', async () => {
    renderHub({})
    await openPackTab()
    await waitFor(() => expect(screen.getByText('自动化测试')).toBeInTheDocument())
    const card = screen.getByText('自动化测试').closest('li') as HTMLElement
    // 显示成「引用 N 个下游技能」，而不是「含 N 个技能」——
    // 那些只有 slug 与简介，没有正文，装进来模型也读不到内容。
    expect(within(card).getByText(/点名 2 个下游技能/)).toBeInTheDocument()
    expect(within(card).queryByText(/含 2 个技能/)).toBeNull()
  })

  it('装完明说：装了几个、装完是停用的、哪几个没装成', async () => {
    renderHub({
      installPack: {
        installed: [{ slug: 'tech-test-automation', description: '' }],
        source_slug: 'tech-test-automation',
        display_name: '自动化测试',
        installed_count: 1,
        referenced_not_installed: ['superpowers-tdd', 'test-case-generator'],
        skipped_other: 0,
        compressed_bytes: 6855,
        uncompressed_bytes: 6565,
        enabled: false,
      },
    })
    await openPackTab()
    await waitFor(() => expect(screen.getByText('自动化测试')).toBeInTheDocument())
    screen.getByTestId('hub-install-tech-test-automation').click()

    await waitFor(() => expect(screen.getByText(/共 1 个技能/)).toBeInTheDocument())
    expect(screen.getByText(/装完是停用的/)).toBeInTheDocument()
    // 没装的那两个必须点名写出来。
    expect(screen.getByText(/所以没有装/)).toBeInTheDocument()
    expect(screen.getByText(/superpowers-tdd、test-case-generator/)).toBeInTheDocument()
  })

  it('上游没给总数就说没给，不拿本页条数冒充「共 N 个」', async () => {
    renderHub({
      list: { host: 'https://api.skillhub.cn', items: [hub()], total: null, page: 1, page_size: 20 },
    })
    await openPackTab()
    await waitFor(() => expect(screen.getByText('自动化测试')).toBeInTheDocument())
    expect(screen.getByText(/上游没给总数/)).toBeInTheDocument()
    expect(screen.queryByText(/上游说共/)).toBeNull()
  })

  it('上游没给说明就说没给，不拿 slug 顶替', async () => {
    renderHub({ list: { ...OK_LIST, items: [hub({ summary: '', summary_en: '' })] } })
    await openPackTab()
    await waitFor(() => expect(screen.getByText('自动化测试')).toBeInTheDocument())
    expect(screen.getByText(/不拿 slug 顶替/)).toBeInTheDocument()
  })
})

describe('技能市场 · 单个技能', () => {
  it('默认落在「单个技能」，因为实测能直接用的技能在这一层', async () => {
    renderHub({})
    // 技能包装的是一份编排说明；把单技能放在第一位，
    // 免得多数人点一次安装、拿到 1 个编排说明还以为市场缺货。
    await waitFor(() => expect(screen.getByText('PDF和图片文字提取')).toBeInTheDocument())
    expect(screen.getByTestId('hub-tab-skill')).toHaveAttribute('aria-selected', 'true')
    expect(screen.queryByText('自动化测试')).toBeNull()
  })

  it('两层的页签都在，名字说清各自是什么', async () => {
    renderHub({})
    await waitFor(() => expect(screen.getByTestId('hub-tab-skill')).toBeInTheDocument())
    expect(screen.getByTestId('hub-tab-pack')).toBeInTheDocument()
    expect(screen.getByText(/技能包装的是一份编排说明/)).toBeInTheDocument()
  })

  it('安装次数上游没给就显示「—」，不拿 0 顶替', async () => {
    renderHub({
      rank: { ...OK_RANK, items: [skill({ installs: null })] },
    })
    await waitFor(() => expect(screen.getByText('PDF和图片文字提取')).toBeInTheDocument())
    // 拿 0 顶替是在编一个数字。
    expect(screen.getByText('—')).toBeInTheDocument()
    expect(screen.queryByText(/0\s*次安装/)).toBeNull()
  })

  it('有安装次数就照实显示那个数', async () => {
    renderHub({ rank: { ...OK_RANK, items: [skill({ installs: 378 })] } })
    await waitFor(() => expect(screen.getByText('378')).toBeInTheDocument())
    expect(screen.getByText('次安装')).toBeInTheDocument()
  })

  it('上游标了要 API Key 就在卡片上标出来', async () => {
    // 实测上游给的是字符串 "true"（不是布尔），两种都要认。
    // 少了这个标记，用户装完才发现这个技能要自己的密钥。
    // 分成两条用例：一个容器里只渲染一次，才不会互相干扰。
    const { unmount } = renderHub({
      rank: { ...OK_RANK, items: [skill({ labels: { requires_api_key: 'true' } })] },
    })
    await waitFor(() => expect(screen.getByText('需要 API Key')).toBeInTheDocument())
    unmount()

    renderHub({
      rank: { ...OK_RANK, items: [skill({ labels: { requires_api_key: true } })] },
    })
    await waitFor(() => expect(screen.getByText('需要 API Key')).toBeInTheDocument())
  })

  it('没标 API Key 的技能不被标成需要', async () => {
    renderHub({ rank: { ...OK_RANK, items: [skill({ labels: null })] } })
    await waitFor(() => expect(screen.getByText('PDF和图片文字提取')).toBeInTheDocument())
    expect(screen.queryByText('需要 API Key')).toBeNull()
  })

  it('已经装过的技能标出来，按钮写「重新安装」', async () => {
    // 已装状态**从技能目录查**，不是市场自己说。
    renderHub({ installed: [{ slug: 'pdf-image-text-extractor', tool_name: 'pdf-image-text-extractor' }] })
    await waitFor(() => expect(screen.getByText('已安装')).toBeInTheDocument())
    expect(screen.getByText('重新安装')).toBeInTheDocument()
    // 卡片上要能一眼看出这张是装过的。
    expect(screen.getByTestId('hub-card-pdf-image-text-extractor').className).toContain(
      'is-installed',
    )
  })

  it('名字与标签分行摆，名字不会被标签挤没', async () => {
    // 真机上就是这个：技能名、图标、两个标签挤在一行 flex 里，
    // 「PDF和图片文字提取」被压成了「PDF…」。而那是卡片上唯一
    // 能认出它是什么的字。
    // jsdom 不做布局，**算不出截断**；所以只能钉住「结构上分开」：
    // 标签必须在一个 `.hub-card-tags` 容器里，而不是与名字同级。
    renderHub({
      installed: [{ slug: 'pdf-image-text-extractor' }],
      rank: {
        ...OK_RANK,
        items: [skill({ labels: { requires_api_key: 'true' } })],
      },
    })
    await waitFor(() => expect(screen.getByText('需要 API Key')).toBeInTheDocument())
    const name = screen.getByText('PDF和图片文字提取')
    const tags = document.querySelector('.hub-card-tags')
    expect(tags).not.toBeNull()
    // 名字**不在**标签容器里 —— 两者的行数由 CSS 网格分开。
    expect(tags?.contains(name)).toBe(false)
    expect(name.parentElement).toBe(tags?.parentElement)
  })

  it('列数随宽度自动铺，不写死', async () => {
    // 这条**不能靠渲染后的 DOM 断言**（jsdom 不做布局，算不出列数）。
    // 所以直接读样式表：网格必须是 `auto-fill` + `minmax`，
    // 任何写死列数的写法（`repeat(2, …)`、`grid-template-columns: 2`）
    // 都会让宽屏白白浪费横向空间。
    //
    // `min(100%, 260px)` 里的 `min(100%, …)` 也不能少：
    // 容器比下限还窄时，没有它卡片会**溢出**容器而不是换行。
    const css = hubCss
    expect(css).toMatch(/grid-template-columns:\s*repeat\(auto-fill,\s*minmax\(/)
    expect(css).toMatch(/minmax\(min\(100%,\s*\d+px\),\s*1fr\)/)
    // 下限别大到只剩两列。实测这页 `.card-body` 可用宽约 780px
    // （1527 截图 ÷ DPR 1.25 ≈ 1221 CSS px，减侧栏与内边距）：
    // ```
    // 列数 = floor(容器宽 / (下限 + 间距))
    // 320（Octop 的值）→ 2 列     260 → 2 列     240 → 3 列
    // ```
    // 3 列需要 下限 ≤ (780 − 2×12) / 3 = 252px。
    // 也不能无脑调小：240 − 24 内边距 = 216px 内容宽，
    // 刚好装得下「↓ 378 次安装」与「重新安装」按钮而不换行。
    const floor = Number(/minmax\(min\(100%,\s*(\d+)px\)/.exec(css)?.[1] ?? '9999')
    expect(floor).toBeLessThanOrEqual(250)
    expect(floor).toBeGreaterThanOrEqual(220)
    // 也不许出现写死的列数。
    expect(css).not.toMatch(/repeat\(\s*\d+\s*,/)
  })

  it('已装的技能排前面，省得在一堆卡片里找', async () => {
    // Octop 的 `displaySkills` 就是这么排的（已装的在前）。
    renderHub({
      installed: [{ slug: 'b-skill' }],
      rank: {
        ...OK_RANK,
        items: [
          skill({ slug: 'a-skill', name: 'A' }),
          skill({ slug: 'b-skill', name: 'B' }),
          skill({ slug: 'c-skill', name: 'C' }),
        ],
      },
    })
    await waitFor(() => expect(screen.getByText('A')).toBeInTheDocument())
    const slugs = Array.from(document.querySelectorAll('.hub-card')).map(
      (el) => el.getAttribute('data-slug'),
    )
    expect(slugs[0]).toBe('b-skill')
  })

  it('没有图标时用占位图标，不留破图', async () => {
    // 实测大量 icon_url 是 null 或外部链接，加载不出来是常态。
    renderHub({ rank: { ...OK_RANK, items: [skill({ icon_url: '' })] } })
    await waitFor(() => expect(screen.getByText('PDF和图片文字提取')).toBeInTheDocument())
    expect(document.querySelector('.hub-card-icon.is-fallback')).not.toBeNull()
    expect(document.querySelector('.hub-card-icon[src]')).toBeNull()
  })

  it('图标加载失败时换成占位图标', async () => {
    renderHub({
      rank: { ...OK_RANK, items: [skill({ icon_url: 'https://example.invalid/x.png' })] },
    })
    await waitFor(() => expect(screen.getByText('PDF和图片文字提取')).toBeInTheDocument())
    const img = document.querySelector('.hub-card-icon[src]') as HTMLImageElement
    expect(img).not.toBeNull()
    fireEvent.error(img)
    await waitFor(() =>
      expect(document.querySelector('.hub-card-icon.is-fallback')).not.toBeNull(),
    )
  })

  it('中文简介优先于英文', async () => {
    renderHub({})
    await waitFor(() => expect(screen.getByText(/从图片或 PDF 文档中识别并提取文字内容/)).toBeInTheDocument())
  })

  it('上游没给简介就说没给', async () => {
    renderHub({
      rank: { ...OK_RANK, items: [skill({ description: '', description_zh: '' })] },
    })
    await waitFor(() => expect(screen.getByText('PDF和图片文字提取')).toBeInTheDocument())
    expect(screen.getByText(/不拿 slug 顶替/)).toBeInTheDocument()
  })

  it('榜单没拉到时显示错误 + 下一步，不显示成「没有技能」', async () => {
    renderHub({ rankError: '拉技能市场榜单失败：连不上技能市场。' })
    await waitFor(() => expect(screen.getByText(/连不上技能市场/)).toBeInTheDocument())
    expect(screen.getByText(/已装好的技能不受影响/)).toBeInTheDocument()
    expect(screen.queryByText(/这一份榜单上游返回了 0 条/)).toBeNull()
  })

  it('「全部」这一页只拉到一部分时要说出来，不假装完整', async () => {
    renderHub({
      rank: {
        ...OK_RANK,
        kind: 'all',
        section: null,
        items: [skill()],
        errors: { trending: '上游 503' },
      },
    })
    fireEvent.click(await screen.findByTestId('hub-kind-all'))
    await waitFor(() => expect(screen.getByText(/没拉到/)).toBeInTheDocument())
    expect(screen.getByText(/trending/)).toBeInTheDocument()
    // 关键：说了「不是全部」。
    expect(screen.getByText(/不是全部/)).toBeInTheDocument()
  })

  it('搜索走的是单技能这一层，不是技能包', async () => {
    renderHub({
      search: {
        host: 'https://api.skillhub.cn',
        query: 'pdf',
        items: [skill({ slug: 'document-pdf', name: 'Pdf' })],
        total: null,
      },
    })
    await waitFor(() => expect(screen.getByTestId('hub-skill-search')).toBeInTheDocument())
    fireEvent.change(screen.getByTestId('hub-skill-search'), { target: { value: 'pdf' } })
    fireEvent.click(screen.getByTestId('hub-skill-search-go'))
    await waitFor(() => expect(screen.getByText('document-pdf')).toBeInTheDocument())
  })

  it('搜不到就说搜不到，并说清这是上游回的结果', async () => {
    renderHub({ search: { host: 'h', query: 'zzz', items: [], total: null } })
    await waitFor(() => expect(screen.getByTestId('hub-skill-search')).toBeInTheDocument())
    fireEvent.change(screen.getByTestId('hub-skill-search'), { target: { value: 'zzz' } })
    fireEvent.click(screen.getByTestId('hub-skill-search-go'))
    await waitFor(() => expect(screen.getByText(/没有结果/)).toBeInTheDocument())
    expect(screen.getByText(/不是我们没去取/)).toBeInTheDocument()
  })

  it('装完明说：装了几个、正文取自哪个文件、装完是停用的', async () => {
    renderHub({
      installSkill: {
        installed: [{ slug: 'pdf-image-text-extractor', description: '提取文字' }],
        installed_count: 1,
        source_slug: 'pdf-image-text-extractor',
        body_file: 'SKILL.md',
        skipped_other: 7,
        compressed_bytes: 21929,
        uncompressed_bytes: 52000,
        enabled: false,
      },
    })
    await waitFor(() => expect(screen.getByText('PDF和图片文字提取')).toBeInTheDocument())
    screen.getByTestId('hub-skill-install-pdf-image-text-extractor').click()

    await waitFor(() => expect(screen.getByText(/已装入/)).toBeInTheDocument())
    // 包里那个文件被当成正文要说出来 —— 换了名字也要能追到。
    expect(screen.getByText(/正文取自包里的 SKILL\.md/)).toBeInTheDocument()
    // 实测这个包 8 个条目只装 1 个，剩下的要说出来。
    expect(screen.getByText(/另有 7 个条目不是技能/)).toBeInTheDocument()
    expect(screen.getByText(/装完是停用的/)).toBeInTheDocument()
  })

  it('同一个 slug 的多个版本都列出来，不替用户挑一个', async () => {
    // 真机实测：上游推荐榜里 `dev-expert` 同时有 2.0.3 与 1.17.0。
    // 合并成一行 = 我们替上游做了「哪个版本更好」的判断。
    renderHub({
      rank: {
        ...OK_RANK,
        items: [
          skill({ slug: 'dev-expert', name: '编程专家.Skill', version: '2.0.3' }),
          skill({ slug: 'dev-expert', name: 'dev-expert', version: '1.17.0' }),
        ],
      },
    })
    await waitFor(() => expect(screen.getByText('版本 2.0.3')).toBeInTheDocument())
    // 两个版本都在，各自标了自己的版本号。
    expect(screen.getByText('版本 1.17.0')).toBeInTheDocument()
    // 两张卡片都在（数 slug 标签：第二行的 name 也叫 dev-expert，
    // 用 getAllByText 会把它一起数进来）。
    expect(document.querySelectorAll('.hub-card-meta code')).toHaveLength(2)
  })

  it('装失败时显示服务端的中文说明与下一步，不退化成「请求失败」', async () => {
    // 这条钉的是 `ErrorNotice` 的用法：先转字符串会把说明与下一步全丢掉。
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
        if (init?.method === 'POST') {
          return new Response(
            JSON.stringify({
              error: {
                code: 'entity_not_found',
                detail: '下载技能 nope 失败：技能市场里没有这个东西（上游返回 404），它可能已被下架。',
                next_step: '回到列表里重新选一个。',
              },
            }),
            { status: 404, headers: { 'content-type': 'application/json' } },
          )
        }
        return new Response(JSON.stringify(OK_RANK), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        })
      }),
    )
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={client}>
        <HubList />
      </QueryClientProvider>,
    )
    await waitFor(() => expect(screen.getByText('PDF和图片文字提取')).toBeInTheDocument())
    screen.getByTestId('hub-skill-install-pdf-image-text-extractor').click()

    await waitFor(() => expect(screen.getByText(/可能已被下架/)).toBeInTheDocument())
    expect(screen.getByText(/回到列表里重新选一个/)).toBeInTheDocument()
  })
})
