import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Link, useSearchParams } from 'react-router-dom'

import { PageHeader } from '../components/Page'
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
const TABS = [
  {
    key: 'skills',
    icon: 'K',
    titleKey: 'personalization.skills',
    title: '技能',
    descKey: 'personalization.skillsDesc',
    desc: '给智能体加的手艺：一份带说明的 markdown，装上后变成它能用的技能。',
    to: '/skills',
  },
  {
    key: 'subagents',
    icon: 'B',
    titleKey: 'personalization.subagents',
    title: '子智能体',
    descKey: 'personalization.subagentsDesc',
    desc: '把专家编成团队，派工时它们各自领活。',
    to: '/experts?tab=team',
  },
  {
    key: 'tools',
    icon: 'T',
    titleKey: 'personalization.tools',
    title: '工具',
    descKey: 'personalization.toolsDesc',
    desc: 'MCP 服务。智能体在这里挂的能力开关，决定它能调用什么。',
    to: '/devices',
  },
  {
    key: 'plugins',
    icon: 'U',
    titleKey: 'personalization.plugins',
    title: '插件',
    descKey: 'personalization.pluginsDesc',
    desc: '技能包的形式之一：一个带清单与启用开关的目录。',
    to: '/skills',
  },
  {
    key: 'mbti',
    icon: 'G',
    titleKey: 'personalization.mbti',
    title: '人格',
    descKey: 'personalization.mbtiDesc',
    desc: '28 道题算出一个人格类型，看四维光谱，选一个专家就把这套说话风格写进它的人格正文。',
    to: null,
  },
  {
    key: 'memory',
    icon: 'N',
    titleKey: 'personalization.memory',
    title: '记忆',
    descKey: 'personalization.memoryDesc',
    desc: '智能体的长期资料库。',
    to: '/memory',
  },
  {
    key: 'channels',
    icon: 'H',
    titleKey: 'personalization.channels',
    title: '通道',
    descKey: 'personalization.channelsDesc',
    desc: '把智能体接到浏览器之外。在微信上给它发消息，它在那边回你。',
    to: '/channels',
  },
] as const

type TabKey = (typeof TABS)[number]['key']

const KEYS = TABS.map((t) => t.key) as readonly TabKey[]

export function PersonalizationPage(): ReactNode {
  const { t } = useTranslation()
  const [params, setParams] = useSearchParams()

  // 与专家页同一套做法：当前页签活在 URL 上，`?tab=` 认不出来就落回第一个。
  // 放 URL 而不是内存，是为了让「个性化 / 人格」能直接发给别人。
  const raw = params.get('tab')
  const active: TabKey = KEYS.includes(raw as TabKey) ? (raw as TabKey) : KEYS[0]
  const current = TABS.find((s) => s.key === active) ?? TABS[0]
  const titleOf = (s: (typeof TABS)[number]) => t(s.titleKey, { defaultValue: s.title })

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

      <section
        className="personalization-panel"
        role="tabpanel"
        id={`p-panel-${active}`}
        aria-labelledby={`p-tab-${active}`}
        data-section={active}
      >
        {active === 'mbti' ? (
          <MbtiPage embedded />
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