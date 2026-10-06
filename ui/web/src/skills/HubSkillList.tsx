import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice } from '../components/Page'
import { IconDownload, IconZap } from '../experts/icons'
import {
  hubRankings,
  installHubSkill,
  installsCount,
  needsApiKey,
  searchHubSkills,
  skillIcon,
  skillLabel,
  skillSummary,
  type HubSkill,
} from './hubApi'
import { listSkills, skillsOf } from './skillsApi'
import './hub.css'

/**
 * 市场里的**单技能**列表（卡片网格）。
 *
 * ## 为什么要和技能包分开
 *
 * 实测（2026-10-06）：技能集 `tech-test-automation` 的 zip 只有
 * `identify.md`（一篇编排说明）与一份点名 6 个下游技能的 manifest，
 * 而那 6 个**包里没有正文**。单技能 `pdf-image-text-extractor` 才有
 * 15 KB 的 `SKILL.md`。
 *
 * 两者混在一张列表里，用户点一次安装、只拿到 1 个技能，就会以为
 * 市场缺货 —— 实际是他装错了那一层。
 *
 * ## 卡片形态抄 Octop
 *
 * 结构照 `SkillHubTab.tsx` 的 `hubCard`：图标 + 名称 + 「需要 API Key」
 * 橙标、两行截断的简介、footer 左下载数右安装按钮；
 * 网格用 `repeat(auto-fill, minmax(320px, 1fr))`。
 * **只抄结构**，配色与圆角走 quill 自己的变量 ——
 * `index.css` 是 vendor 移植基准，一个字节都不能动。
 *
 * ## 这一层的三条如实呈现
 *
 * 1. **榜单可能只拉到一部分。**「全部」是并发拉 6 个榜合并的，
 *    少的那几个在 `errors` 里，界面上要说出来。
 * 2. **安装次数上游没给就是「—」。** 拿 0 顶替是编一个数字。
 * 3. **同一个 slug 的多个版本都列出来。** 合并成一行等于替用户挑版本。
 */

const SKILL_KEY = ['skill-hub', 'skills'] as const
const RANK_KEY = ['skill-hub', 'rankings'] as const
const INSTALLED_KEY = ['extensions', 'skills'] as const
const SEARCH_LIMIT = 20

function SkillInstallButton({
  item,
  installed,
}: {
  item: HubSkill
  installed: boolean
}): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()

  const install = useMutation({
    mutationFn: () => installHubSkill(item.slug),
    onSuccess: () => {
      // 装完它就进了「已装」集合，卡片上的「重新安装」与已安装标记要跟上。
      void client.invalidateQueries({ queryKey: RANK_KEY })
      void client.invalidateQueries({ queryKey: INSTALLED_KEY })
    },
  })

  if (install.isSuccess) {
    const r = install.data
    return (
      <div className="hub-installed">
        <p className="hub-ok" role="status">
          {t('hub.skillInstalled', {
            name: skillLabel(item),
            defaultValue: '已装入「{{name}}」。',
          })}
        </p>
        <p className="field-help">
          {t('hub.installedDisabled', {
            defaultValue:
              '装完是停用的：还没挂进对话工具表，模型这一轮看不到它。去「已安装」看一眼再打开开关。',
          })}
        </p>
        {/* 正文取自包里哪个文件要说出来。上游今天用 SKILL.md，
            哪天改了名，用户至少知道装进去的是哪一篇。 */}
        <p className="field-help">
          {t('hub.bodyFile', {
            file: r.body_file,
            defaultValue: '正文取自包里的 {{file}}。',
          })}
        </p>
        {r.skipped_other > 0 ? (
          <p className="field-help">
            {t('hub.skipped', {
              count: r.skipped_other,
              defaultValue: '包里另有 {{count}} 个条目不是技能（图片、脚本之类），没有安装。',
            })}
          </p>
        ) : null}
      </div>
    )
  }

  return (
    <div className="hub-action">
      <button
        type="button"
        className="primary-button"
        data-testid={`hub-skill-install-${item.slug}`}
        disabled={install.isPending}
        onClick={() => install.mutate()}
      >
        {install.isPending
          ? t('hub.installing', { defaultValue: '安装中…' })
          : installed
            ? t('hub.reinstall', { defaultValue: '重新安装' })
            : t('hub.install', { defaultValue: '安装' })}
      </button>
      {/* **把 error 对象原样交给 `ErrorNotice`**，不要先转字符串 ——
          转了就把服务端那句中文说明与「下一步：…」全丢掉。 */}
      {install.isError ? <ErrorNotice error={install.error} /> : null}
    </div>
  )
}

/**
 * 一张技能卡片。
 *
 * `key` 用 `slug@version` 而不是 `slug`：上游的同一份榜单里会出现
 * 同一个 slug 的多个版本（实测 `dev-expert` 同时有 2.0.3 与 1.17.0），
 * 拿 slug 当 key 会撞。
 */
function SkillCard({
  item,
  installed,
}: {
  item: HubSkill
  installed: boolean
}): ReactNode {
  const { t } = useTranslation()
  const summary = skillSummary(item)
  const icon = skillIcon(item)
  const [iconBroken, setIconBroken] = useState(false)
  const installs = installsCount(item)
  const apiKey = needsApiKey(item)

  return (
    <li
      className={installed ? 'hub-card is-installed' : 'hub-card'}
      data-slug={item.slug}
      data-version={item.version}
      data-testid={`hub-card-${item.slug}`}
    >
      <div className="hub-card-head">
        {icon && !iconBroken ? (
          <img
            className="hub-card-icon"
            src={icon}
            alt=""
            loading="lazy"
            // 外链图标挂掉是常态（实测大量 `icon_url` 指向外部存储），
            // 留一个破图标比换个占位难看得多。
            onError={() => setIconBroken(true)}
          />
        ) : (
          <span className="hub-card-icon is-fallback" aria-hidden="true">
            <IconZap size={16} />
          </span>
        )}
        <strong className="hub-card-name">{skillLabel(item)}</strong>
        {/*
          标签**另起一行**，不跟名字抢同一行。
          真机上就是抢出来的后果：「PDF和图片文字提取」被两个标签
          挤成了「PDF…」—— 那是这张卡片上唯一能认出它是什么的字。
        */}
        {apiKey || installed ? (
          <span className="hub-card-tags">
            {apiKey ? (
              <span className="hub-card-flag">
                {t('hub.needsApiKey', { defaultValue: '需要 API Key' })}
              </span>
            ) : null}
            {installed ? (
              <span className="hub-card-flag is-done">
                {t('hub.alreadyInstalled', { defaultValue: '已安装' })}
              </span>
            ) : null}
          </span>
        ) : null}
      </div>

      <p className={summary ? 'hub-summary' : 'hub-summary is-empty'}>
        {summary ||
          t('hub.noSummary', {
            defaultValue: '上游没给说明。没有说明就是没有，不拿 slug 顶替。',
          })}
      </p>

      {/* slug 与版本：**不抄 Octop 的省略**。
          Octop 的卡片只显示 `name`，可实测上游有一批技能的 `name`
          是个占位串（`martin-pdf` 的 name 字面就是 `pdf`），
          去掉 slug 之后三张卡片全都叫「pdf」，用户无从分辨。
          版本同理：同一 slug 会同时有多个版本，不写出来就看不出差在哪。 */}
      <p className="hub-card-meta">
        <code>{item.slug}</code>
        {item.version ? (
          <span className="hub-card-version">
            {t('hub.version', {
              version: item.version,
              defaultValue: '版本 {{version}}',
            })}
          </span>
        ) : null}
      </p>

      <div className="hub-card-foot">
        <span className="hub-card-stat">
          <IconDownload size={13} />
          {/* 上游没给安装次数就显示「—」。拿 0 顶替是在编一个数字。 */}
          {installs === null
            ? t('hub.installsUnknown', { defaultValue: '—' })
            : installs.toLocaleString()}
          {installs !== null ? (
            <span className="hub-card-stat-unit">
              {t('hub.installsUnit', { defaultValue: '次安装' })}
            </span>
          ) : null}
        </span>
        <SkillInstallButton item={item} installed={installed} />
      </div>
    </li>
  )
}

function SkillCards({
  items,
  installedSlugs,
}: {
  items: HubSkill[]
  installedSlugs: ReadonlySet<string>
}): ReactNode {
  // **已装的排前面。** 抄 Octop 的 `displaySkills`：
  // 用户刚装完一个技能，刷新市场页第一眼就该看到它，而不是要在一堆
  // 卡片里按名字找。**顺序只动位置，不动内容** ——
  // 列表里仍然是上游给的那些条目，一条没多一条没少。
  const ordered = [...items].sort(
    (a, b) => Number(installedSlugs.has(b.slug)) - Number(installedSlugs.has(a.slug)),
  )
  return (
    <ul className="hub-grid">
      {ordered.map((item) => (
        <SkillCard
          key={`${item.slug}@${item.version}`}
          item={item}
          installed={installedSlugs.has(item.slug)}
        />
      ))}
    </ul>
  )
}

/** 榜单页签。上游支持的类型由**服务端**给出，界面不自己编一份。 */
const KIND_LABELS: Record<string, { zh: string; en: string }> = {
  all: { zh: '全部', en: 'All' },
  recommended: { zh: '推荐', en: 'Recommended' },
  trending: { zh: '趋势', en: 'Trending' },
  hot: { zh: '热门', en: 'Hot' },
  featured: { zh: '精选', en: 'Featured' },
  newest: { zh: '最新', en: 'Newest' },
  paid: { zh: '付费', en: 'Paid' },
}

export function HubSkillList(): ReactNode {
  const { t } = useTranslation()
  const [kind, setKind] = useState('recommended')
  const [term, setTerm] = useState('')
  const [submitted, setSubmitted] = useState('')
  // 搜索框里没提交就空着：**空查询不该发请求**，
  // 上游对空 `q` 会回一整页随机内容（Octop 兜成 `q=a`）。
  const searching = submitted.trim().length > 0

  const search = useQuery({
    queryKey: [...SKILL_KEY, submitted],
    queryFn: () => searchHubSkills(submitted, SEARCH_LIMIT),
    enabled: searching,
    staleTime: 30_000,
    retry: false,
  })

  const board = useQuery({
    queryKey: [...RANK_KEY, kind],
    queryFn: () => hubRankings(kind),
    // 搜着的时候不同时拉榜单 —— 否则每次点「搜索」都白打一次榜单。
    enabled: !searching,
    staleTime: 30_000,
    retry: false,
  })

  // 已装集合。**从技能目录查，而不是市场自己说「这个装过了」** ——
  // 市场那边没有这个信息，猜一个「应该装过」就是在编。
  const installedQuery = useQuery({
    queryKey: INSTALLED_KEY,
    queryFn: listSkills,
    staleTime: 30_000,
    retry: false,
  })
  const installedSlugs = new Set(
    skillsOf(installedQuery.data)
      .map((s) => s.slug)
      .filter((s): s is string => typeof s === 'string' && s.length > 0),
  )

  const active = searching ? search : board
  const items = active.data?.items ?? []
  const host = active.data?.host
  const failedSections = Object.keys(board.data?.errors ?? {})

  return (
    <div className="settings-stack">
      <Card
        title={t('hub.skillTitle', { defaultValue: '市场里的单个技能' })}
        description={t('hub.skillDescription', {
          defaultValue:
            '和上面的「技能包」不是一回事：技能包装的是一份编排说明，这里才是带正文的技能。',
        })}
        actions={(
          <button
            className="secondary-button"
            data-testid="hub-skill-refresh"
            onClick={() => void active.refetch()}
          >
            {t('common.refresh', { defaultValue: '刷新' })}
          </button>
        )}
      >
        {host ? (
          <p className="field-help hub-host">
            {t('hub.host', {
              host,
              defaultValue:
                '数据来自外部市场 {{host}}。它挂了或断网时这里会连不上 —— 那不影响已经装好的技能。',
            })}
          </p>
        ) : null}

        <form
          className="hub-search"
          onSubmit={(e) => {
            e.preventDefault()
            setSubmitted(term)
          }}
        >
          <input
            type="search"
            value={term}
            aria-label={t('hub.searchLabel', { defaultValue: '搜技能' })}
            placeholder={t('hub.searchPlaceholder', {
              defaultValue: '按名字或用途搜，例如 pdf、翻译',
            })}
            data-testid="hub-skill-search"
            onChange={(e) => setTerm(e.target.value)}
          />
          <button
            type="submit"
            className="secondary-button"
            data-testid="hub-skill-search-go"
          >
            {t('hub.search', { defaultValue: '搜索' })}
          </button>
          {searching ? (
            <button
              type="button"
              className="secondary-button"
              data-testid="hub-skill-clear"
              onClick={() => {
                setTerm('')
                setSubmitted('')
              }}
            >
              {t('hub.clearSearch', { defaultValue: '回到榜单' })}
            </button>
          ) : null}
        </form>

        {!searching ? (
          <div className="hub-kinds" role="tablist">
            {(board.data?.kinds ?? ['recommended']).map((k) => (
              <button
                key={k}
                type="button"
                role="tab"
                aria-selected={k === kind}
                className={k === kind ? 'skills-tab is-active' : 'skills-tab'}
                data-testid={`hub-kind-${k}`}
                onClick={() => setKind(k)}
              >
                {t(`hub.kind.${k}`, { defaultValue: KIND_LABELS[k]?.zh ?? k })}
              </button>
            ))}
          </div>
        ) : null}

        {active.isError ? <ErrorNotice error={active.error} /> : null}
        {active.isPending ? (
          <p className="empty-state">{t('common.loading', { defaultValue: '加载中…' })}</p>
        ) : null}

        {!active.isPending && !active.isError && items.length === 0 ? (
          <p className="empty-state">
            {searching
              ? t('hub.searchEmpty', {
                  term: submitted,
                  defaultValue: '搜「{{term}}」没有结果 —— 这是上游回的结果，不是我们没去取。',
                })
              : t('hub.boardEmpty', {
                  defaultValue: '这一份榜单上游返回了 0 条 —— 这是上游说的，不是我们没去取。',
                })}
          </p>
        ) : null}

        {/* 「全部」是并发拉 6 个榜合并的。少拉到的要说出来 ——
            吞掉的话，这个页签会假装自己是完整的。 */}
        {failedSections.length > 0 ? (
          <p className="hub-partial" role="status">
            {t('hub.boardPartial', {
              names: failedSections.join('、'),
              defaultValue: '这几个榜单这一轮没拉到：{{names}}。下面这些是成功的那部分，不是全部。',
            })}
          </p>
        ) : null}

        {items.length > 0 ? (
          <SkillCards items={items} installedSlugs={installedSlugs} />
        ) : null}
      </Card>
    </div>
  )
}
