import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Link, useSearchParams } from 'react-router-dom'

import { PageHeader } from '../components/Page'
import { listExperts, EXPERTS_KEY } from '../experts/api'
import { MbtiPage } from '../mbti/MbtiPage'
import './personalization.css'

/**
 * 个性化：技能 / 子智能体 / 工具 / 插件 / 人格 / 记忆 / 通道。
 *
 * 形态对齐 Octop 的 `/personalization`
 * （`.octop-ref/octop/dashboard/src/pages/Agent/Personalization/index.tsx:30-49`，
 * **只读对齐**）：**标题右侧一行横向页签**，内容区只显示当前那一签
 * （`index.tsx:140` 用 `display: none` 切 panel），标题拼成「个性化 / 技能」
 * （`index.tsx:92`）。
 *
 * 那行页签在桌面端落在标题行右侧、移动端全宽放到内容区上方，是
 * `PageShell` 的 `pathTabs` 槽渲染的，而它用的是 **antd 的 `Segmented`**
 * （`layouts/PageShell.tsx:52-73`）—— 一个带边框的容器把各项包在里面、
 * 选中项在容器内高亮，**不是** antd 的 `Tabs`。这个区别是结构性的：
 * 一排各自带边框的独立按钮看着像 segmented，其实不是。
 * 桌面/移动两种摆法见 `layouts/PageShell.module.less` 的 `.pathTabsMobile*`。
 *
 * 2026-10-08 改过一次形态：最早这里是七张大卡片铺成两排。**那是照着想象画的**，
 * 用户看完 Octop 后指出「是一行页签导航，不是大框框」。改法见下面的顺序表 ——
 * 连页签顺序都照 Octop 的来（skills → subagents → tools → plugins → mbti →
 * memory → channels），因为那个顺序不是随手排的：技能在最前是因为它是使用频率
 * 最高的，工具与插件挨着因为它们改的是同一样东西（智能体能调用什么）。
 *
 * ## 这一页跟哪个专家有关
 *
 * Octop 整页是 `agentScoped` 的：挂在**某一个 agent** 下，而它的 agent
 * `kind` 默认就是 `expert`（`infra/db/repos/agents.py:49`）—— 在 Octop 里
 * **agent 就是专家**。
 *
 * 而且七个页签**全部**是那个专家自己的，每个面板都显式收 `agentId`：
 * - `agents.py:37-43`：一行 agent 挂着 `skill_package_ids`、
 *   `knowledge_base_ids`、`mcp_servers`、`persona_mbti` —— 四样都是 per-agent 的；
 * - `ChannelsPanel.tsx:2` 的文件头写的是「Embeddable **per-agent** channels grid」；
 * - `SkillsTabs.tsx:9`：「takes an explicit `agentId` so callers decide
 *   which agent's skills to show」；
 * - `MemoryPanel` 被个性化页与专家抽屉共用，也是 per-agent。
 *
 * **本项目这四样只有人格是真的**（落点 `experts.instructions`）。技能
 * (`skills`)、记忆 (`wiki_index`)、通道 (`channels`) 三张表的归属列都是
 * `user_id`（见 `0001_init.sql` 与 `0009_channels.sql`），也就是说
 * **这一层是我们没做**，不是「事情本来如此」。
 *
 * 所以界面上写的是「本项目目前还是账号级」，而不是「这一项不属于某个专家」
 * —— 后者把没做说成了如此，是这个项目自己记过的毛病（免责声明反向授权）。
 *
 * 真要跟上，得给这三张表各加一层 per-expert 挂载（Octop 的做法是
 * `skill_package_ids` / `knowledge_base_ids` 那两个挂载列表），并改全部读写
 * 路径：数据模型与产品契约的改动，不是能顺手做的。
 *
 * ## 每签里面放什么
 *
 * 上游每一签都是**复用别处的面板**（`SkillsTabs` / `ToolsTabs` /
 * `SubagentManager` / `MBTISelector` / `MemoryPanel` / `ChannelsPanel`）。
 * 本项目的对应面板住在各自的独立页面（`/skills`、`/devices`…），而把六份
 * 实现各抄一份进本页 = 两处从此各改各的。所以：
 *
 * - **人格这一签直接内嵌** `<MbtiPage />`。它是本页签的唯一实现，
 *   抄不了第二份，内嵌反而避免了两个路由各有一份。
 * - 其余六签给说明 + 一个真链接。**画一张「点这里打开」而不是把页面搬进来**，
 *   是因为搬进来就要维护两份；给链接至少不撒谎。
 */

/** 每一签的归属。这是这一页最要紧的一件事，见文件头。 */
type Scope = 'expert' | 'account'

const TABS = [
  {
    key: 'skills',
    icon: 'K',
    titleKey: 'personalization.skills',
    title: '技能',
    descKey: 'personalization.skillsDesc',
    desc: '给智能体加的手艺：一份带说明的 markdown，装上后变成它能用的技能。',
    scope: 'account' as Scope,
    to: '/skills',
  },
  {
    key: 'subagents',
    icon: 'B',
    titleKey: 'personalization.subagents',
    title: '子智能体',
    descKey: 'personalization.subagentsDesc',
    desc: '把专家编成团队，派工时它们各自领活。',
    // 专家本身就是这一签的内容，所以它是专家级的 —— 但也只是「列出并编队」，
    // 派工仍然不执行（B2-2）。
    scope: 'expert' as Scope,
    to: '/experts?tab=team',
  },
  {
    key: 'tools',
    icon: 'T',
    titleKey: 'personalization.tools',
    title: '工具',
    descKey: 'personalization.toolsDesc',
    desc: 'MCP 服务。智能体在这里挂的能力开关，决定它能调用什么。',
    scope: 'account' as Scope,
    to: '/devices',
  },
  {
    key: 'plugins',
    icon: 'U',
    titleKey: 'personalization.plugins',
    title: '插件',
    descKey: 'personalization.pluginsDesc',
    desc: '技能包的形式之一：一个带清单与启用开关的目录。',
    scope: 'account' as Scope,
    to: '/skills',
  },
  {
    key: 'mbti',
    icon: 'G',
    titleKey: 'personalization.mbti',
    title: '人格',
    descKey: 'personalization.mbtiDesc',
    desc: '28 道题算出一个人格类型，看四维光谱，写进下面选中的那个专家的人格正文。',
    // 全项目唯一真·专家级的一项：`experts.instructions` 就是它的落点。
    scope: 'expert' as Scope,
    to: null,
  },
  {
    key: 'memory',
    icon: 'N',
    titleKey: 'personalization.memory',
    title: '记忆',
    descKey: 'personalization.memoryDesc',
    desc: '智能体的长期资料库。',
    scope: 'account' as Scope,
    to: '/memory',
  },
  {
    key: 'channels',
    icon: 'H',
    titleKey: 'personalization.channels',
    title: '通道',
    descKey: 'personalization.channelsDesc',
    desc: '把智能体接到浏览器之外。在微信上给它发消息，它在那边回你。',
    scope: 'account' as Scope,
    to: '/channels',
  },
] as const

type TabKey = (typeof TABS)[number]['key']

const KEYS = TABS.map((t) => t.key) as readonly TabKey[]

export function PersonalizationPage(): ReactNode {
  const { t } = useTranslation()
  const [params, setParams] = useSearchParams()
  const experts = useQuery({ queryKey: EXPERTS_KEY, queryFn: listExperts })

  // 与专家页同一套做法：当前页签活在 URL 上，`?tab=` 认不出来就落回第一个。
  // 放 URL 而不是内存，是为了让「个性化 / 人格」能直接发给别人。
  const raw = params.get('tab')
  const active: TabKey = KEYS.includes(raw as TabKey) ? (raw as TabKey) : KEYS[0]
  const current = TABS.find((s) => s.key === active) ?? TABS[0]
  const titleOf = (s: (typeof TABS)[number]) => t(s.titleKey, { defaultValue: s.title })

  // 当前专家也活在 URL 上：这一页是「围绕某个专家」的，换个专家就该换个
  // 地址，而不是靠一个看不见、也发不出去的内存状态。
  const all = experts.data ?? []
  const picked = all.find((e) => e.id === params.get('expert')) ?? all[0]
  const setExpert = (id: string): void => {
    const next = new URLSearchParams(params)
    if (!id) next.delete('expert')
    else next.set('expert', id)
    setParams(next)
  }

  return (
    <div className="page-scroll personalization-page">
      <PageHeader
        eyebrow={t('personalization.eyebrow', { defaultValue: '个性化' })}
        // 标题跟着页签走，跟上游一样拼成「个性化 / 技能」。
        title={`${t('personalization.title', { defaultValue: '个性化' })} / ${titleOf(current)}`}
        description={t('personalization.description', {
          defaultValue: '配置当前智能体的技能、工具、插件、子智能体、通道与记忆。',
        })}
        actions={
          <div className="personalization-tabs" role="tablist" data-segmented="true">
            {TABS.map((s) => (
              <button
                key={s.key}
                type="button"
                role="tab"
                id={`p-tab-${s.key}`}
                aria-selected={s.key === active}
                aria-controls={`p-panel-${s.key}`}
                className="personalization-tab"
                onClick={() => {
                  const next = new URLSearchParams(params)
                  // 第一个页签不带参数：`/personalization` 本身就要能用。
                  if (s.key === KEYS[0]) next.delete('tab')
                  else next.set('tab', s.key)
                  setParams(next)
                }}
              >
                <span className="personalization-tab-icon" aria-hidden="true">
                  {s.icon}
                </span>
                {titleOf(s)}
              </button>
            ))}
          </div>
        }
      />

      {/* 当前专家 + 归属说明。放在页签**外面**：换专家对七签都有效，
          做成某一签的内容会让人以为只影响那一签。 */}
      <section className="personalization-scope" data-scope-box="true">
        <label className="personalization-scope-label" htmlFor="p-expert">
          {t('personalization.whichExpert', { defaultValue: '配置哪个专家' })}
        </label>
        {experts.isPending ? (
          <span className="personalization-scope-hint">
            {t('common.loading', { defaultValue: '加载中…' })}
          </span>
        ) : null}
        {experts.isSuccess && all.length > 0 ? (
          <>
            <select
              id="p-expert"
              className="input"
              data-expert-picker="true"
              value={picked?.id ?? ''}
              onChange={(e) => setExpert(e.target.value)}
            >
              {all.map((e) => (
                <option key={e.id} value={e.id}>
                  {e.display_name}（{e.id}）
                </option>
              ))}
            </select>
            <span className="personalization-scope-hint" data-scope-note={current.scope}>
              {current.scope === 'expert'
                ? t('personalization.scopeExpert', {
                    name: picked?.display_name ?? '',
                    defaultValue: '这一项是「{{name}}」自己的：改动只落在它身上。',
                  })
                : t('personalization.scopeAccount', {
                    defaultValue:
                      '这一项在 Octop 上是按专家分开的。本项目目前还是账号级 —— 你装一次，所有专家共用同一份。',
                  })}
            </span>
          </>
        ) : null}
        {experts.isSuccess && all.length === 0 ? (
          <span className="personalization-scope-hint">
            {t('personalization.noExpert', {
              defaultValue:
                '你还没有任何专家。这一页的一切都挂在某个专家身上，先到「专家」页建一个。',
            })}
          </span>
        ) : null}
      </section>

      <section
        className="personalization-panel"
        role="tabpanel"
        id={`p-panel-${active}`}
        aria-labelledby={`p-tab-${active}`}
        data-section={active}
      >
        {active === 'mbti' ? (
          <MbtiPage embedded expertId={picked?.id ?? ''} />
        ) : (
          <div className="personalization-brief">
            <p className="personalization-brief-desc">
              {t(current.descKey, { defaultValue: current.desc })}
            </p>
            {current.to ? (
              <Link to={current.to} className="btn" data-section-link={current.key}>
                {t('personalization.openHere', {
                  name: titleOf(current),
                  defaultValue: '打开「{{name}}」',
                })}
              </Link>
            ) : null}
            <p className="personalization-brief-note">
              {t('personalization.elsewhereNote', {
                defaultValue:
                  '这一签的实现住在独立页面，不在索引页里再抄一份 —— 抄一份就会两边各改各的。',
              })}
            </p>
          </div>
        )}
      </section>
    </div>
  )
}