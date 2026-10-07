import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router-dom'

import { PageHeader } from '../components/Page'
import './personalization.css'

/**
 * 个性化：把「这个智能体是谁、能做什么、在哪能找到它」收在一处。
 *
 * 形态对齐 Octop 的 `/personalization`
 * （`.octop-ref/octop/dashboard/src/pages/Agent/Personalization/index.tsx:30-49`，
 * **只读对齐**）：它同样是七个页签的聚合，每一签都是**复用别处的面板**
 * （`SkillsTabs` / `ToolsTabs` / `SubagentManager` / `MemoryPanel` / `ChannelsPanel`），
 * 不是新写一套。
 *
 * **为什么不整页搬进来**：那样会把六个页面的实现各抄一份，两边从此各改各的。
 * 这里只做「索引」——点哪一签就跳到那个页面，那个页面仍是唯一实现。
 * 抄一份看起来省事，代价是六处漂移。
 *
 * **与 Octop 的差别（本项目的选择）**：
 * - Octop 的页签都在 `/personalization/*` 下面，本项目把它们留在各自的老地址
 *   （`/skills`、`/devices`…），这边只放链接。老书签、老外链因此继续有效，
 *   而这正是本项目反复处理过的「信息架构改了别让旧链接变 404」。
 * - **没有 MBTI 页签**。Octop 有（`components/MBTISelector.tsx`），但那要一张
 *   四维光谱表 + 28 题测试结果的存储，本项目现在没有。画一个点不动的选择器
 *   是不如不画 —— 所以下面明说「还没做」，而不是给个空壳。
 */
const SECTIONS = [
  {
    key: 'skills',
    to: '/skills',
    titleKey: 'personalization.skills',
    title: '技能',
    descKey: 'personalization.skillsDesc',
    desc: '给智能体加的手艺：一份带说明的 markdown，装上后变成它能用的技能。',
  },
  {
    key: 'tools',
    to: '/devices',
    titleKey: 'personalization.tools',
    title: '工具',
    descKey: 'personalization.toolsDesc',
    desc: 'MCP 服务。智能体在这里挂的能力开关，决定它能调用什么。',
  },
  {
    key: 'subagents',
    to: '/experts?tab=team',
    titleKey: 'personalization.subagents',
    title: '子智能体',
    descKey: 'personalization.subagentsDesc',
    desc: '把专家编成团队，派工时它们各自领活。',
  },
  {
    key: 'plugins',
    to: '/skills',
    titleKey: 'personalization.plugins',
    title: '插件',
    descKey: 'personalization.pluginsDesc',
    desc: '技能包的形式之一：一个带清单与启用开关的目录。',
  },
  {
    key: 'memory',
    to: '/memory',
    titleKey: 'personalization.memory',
    title: '记忆',
    descKey: 'personalization.memoryDesc',
    desc: '智能体的长期资料库。',
  },
  {
    key: 'channels',
    to: '/channels',
    titleKey: 'personalization.channels',
    title: '通道',
    descKey: 'personalization.channelsDesc',
    desc: '把智能体接到浏览器之外。在微信上给它发消息，它在那边回你。',
  },
] as const

export function PersonalizationPage(): ReactNode {
  const { t } = useTranslation()

  return (
    <div className="page-scroll personalization-page">
      <PageHeader
        eyebrow={t('personalization.eyebrow', { defaultValue: '个性化' })}
        title={t('personalization.title', { defaultValue: '个性化' })}
        description={t('personalization.description', {
          defaultValue: '配置当前智能体的技能、工具、插件、子智能体、通道与记忆。',
        })}
      />

      <div className="personalization-grid">
        {SECTIONS.map((s) => (
          <Link
            key={s.key}
            to={s.to}
            className="personalization-card"
            data-section={s.key}
          >
            <h2>{t(s.titleKey, { defaultValue: s.title })}</h2>
            <p>{t(s.descKey, { defaultValue: s.desc })}</p>
          </Link>
        ))}
      </div>

      {/* 没有 MBTI 页签这件事要写出来。
          悄悄不列 = 用户以为自己漏看了；画个空的 = 用户点了发现是死的。 */}
      <section className="personalization-missing">
        <h2>{t('personalization.mbtiTitle', { defaultValue: 'MBTI' })}</h2>
        <p className="form-notice">
          {t('personalization.mbtiMissing', {
            defaultValue:
              '还没做。这一页要做的是四维光谱与 28 题测评结果的存储与展示，而本项目现在没有这些数据 —— 画一个点不动的选择器比不画更糟，所以这里只说明它不存在。',
          })}
        </p>
      </section>
    </div>
  )
}