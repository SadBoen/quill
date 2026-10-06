import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice, StatusBadge } from '../components/Page'
import { hubLabel, hubSummary, installFromHub, listHub, type HubSkillSet } from './hubApi'
import { HubSkillList } from './HubSkillList'
import './hub.css'

/**
 * 技能市场。
 *
 * ## 这四件事必须说准
 *
 * 1. **上游是外部服务**（SkillHub），不是内置列表。所以界面上要显示
 *    我们**真的连的是哪个地址** —— 用户得知道断网时是市场没了，不是
 *    「quill 没有技能」。
 * 2. **「没连上」与「市场是空的」是两件事。** 上游挂了就显示挂 + 下一步；
 *    绝不返回一张空列表 —— 那会让用户以为是市场没有东西。
 * 3. **装了几个就是几个。** 实测一个包里只有**一篇**正文，
 *    manifest 里点名的 6 个下游技能**没有正文、没装**。界面上必须分开写，
 *    否则用户会以为装漏了，或者更糟 —— 以为那 6 个已经能用。
 * 4. **技能包与单技能是两层。** 所以这里是两个页签而不是一个列表 ——
 *    原因见 `HubSkillList` 的文件头。
 */

const HUB_KEY = ['skill-hub'] as const
const PAGE_SIZE = 20
type SubTab = 'pack' | 'skill'

function InstallButton({ item }: { item: HubSkillSet }): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()

  const install = useMutation({
    mutationFn: () => installFromHub(item.slug),
    onSuccess: () => void client.invalidateQueries({ queryKey: HUB_KEY }),
  })

  if (install.isSuccess) {
    const r = install.data
    const referenced = r.referenced_not_installed
    return (
      <div className="hub-installed">
        <p className="hub-ok" role="status">
          {t('hub.installed', {
            count: r.installed_count,
            name: r.display_name || item.slug,
            defaultValue: '已装入「{{name}}」，共 {{count}} 个技能。',
          })}
        </p>
        {/* 装完是停用的 —— 明说，用户不该以为点一下就生效了。 */}
        <p className="field-help">
          {t('hub.installedDisabled', {
            defaultValue: '装完是停用的：还没挂进对话工具表，模型这一轮看不到它。去「技能包」看一眼再打开开关。',
          })}
        </p>
        {referenced.length > 0 ? (
          <p className="hub-referenced">
            {t('hub.referencedTitle', {
              count: referenced.length,
              defaultValue: '这个包还点名要用下面 {{count}} 个技能，但包里没有它们的正文，所以**没装**：',
            })}
            <span className="hub-referenced-list">{referenced.join('、')}</span>
          </p>
        ) : null}
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
        data-testid={`hub-install-${item.slug}`}
        disabled={install.isPending}
        onClick={() => install.mutate()}
      >
        {install.isPending
          ? t('hub.installing', { defaultValue: '安装中…' })
          : t('hub.install', { defaultValue: '安装' })}
      </button>
      {/* **直接接 `error` 对象**，不能先过 `chatErrorMessage` 转成字符串：
          转了之后 ApiError 的中文说明与「下一步：…」全丢，退化成一句
          「请求失败」。见下面 skill_hub_list 的同类注释。 */}
      {install.isError ? <ErrorNotice error={install.error} /> : null}
    </div>
  )
}

export function HubList(): ReactNode {
  const { t } = useTranslation()
  const [tab, setTab] = useState<SubTab>('skill')

  return (
    <div className="settings-stack">
      <div className="skills-tabs" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={tab === 'skill'}
          className={tab === 'skill' ? 'skills-tab is-active' : 'skills-tab'}
          data-testid="hub-tab-skill"
          onClick={() => setTab('skill')}
        >
          {t('hub.tabSkill', { defaultValue: '单个技能' })}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === 'pack'}
          className={tab === 'pack' ? 'skills-tab is-active' : 'skills-tab'}
          data-testid="hub-tab-pack"
          onClick={() => setTab('pack')}
        >
          {t('hub.tabPack', { defaultValue: '技能包' })}
        </button>
      </div>
      {/* 默认落在「单个技能」：实测技能包装到的是一份编排说明，
          而**能直接用的技能在这一层**。反过来摆会让多数人装错。 */}
      {tab === 'skill' ? <HubSkillList /> : <HubPackList />}
    </div>
  )
}

function HubPackList(): ReactNode {
  const { t } = useTranslation()
  const [page, setPage] = useState(1)
  const query = useQuery({
    queryKey: [...HUB_KEY, page],
    queryFn: () => listHub(page, PAGE_SIZE),
    // 市场是外部服务：多按几次 F5 就给它多打几次。放 30 秒。
    staleTime: 30_000,
    retry: false,
  })

  const data = query.data
  const items = data?.items ?? []
  const hasMore = data ? items.length >= (data.page_size ?? PAGE_SIZE) : false

  return (
    <div className="settings-stack">
      <Card
        title={t('hub.title', { defaultValue: '技能市场' })}
        description={t('hub.description', {
          defaultValue: '装进来的技能默认是停用的 —— 要不要让它进每一轮对话，由你在「技能包」里决定。',
        })}
        actions={(
          <button
            className="secondary-button"
            data-testid="hub-refresh"
            onClick={() => void query.refetch()}
          >
            {t('common.refresh', { defaultValue: '刷新' })}
          </button>
        )}
      >
        {/* 上游地址必须显示。**这不是内置列表**，
            断网 / 上游挂了的时候，用户得知道该去哪里看。 */}
        {data?.host ? (
          <p className="field-help hub-host">
            {t('hub.host', {
              host: data.host,
              defaultValue: '数据来自外部市场 {{host}}。它挂了或断网时这里会连不上 —— 那不影响已经装好的技能。',
            })}
          </p>
        ) : null}

        {query.isError ? (
          /* **把 error 对象原样传进去，不要先转成字符串。**
             `ErrorNotice` 自己认 `ApiError` 才会把服务端那句中文与
             「下一步：…」分开显示；先过一遍 `chatErrorMessage` 就变成了
             一个普通字符串，于是界面上只剩一句「请求失败」——
             精心写好的错误说明与下一步全被丢掉了。 */
          <ErrorNotice error={query.error} />
        ) : null}
        {query.isPending ? (
          <p className="empty-state">{t('common.loading', { defaultValue: '加载中…' })}</p>
        ) : null}

        {!query.isPending && !query.isError && items.length === 0 ? (
          <p className="empty-state">
            {t('hub.empty', {
              defaultValue: '市场这一次返回了 0 个技能集 —— 这是上游说的，不是我们没去取。',
            })}
          </p>
        ) : null}

        {items.length > 0 ? (
          <ul className="hub-list">
            {items.map((item) => {
              const summary = hubSummary(item)
              const children = item.skill_slugs ?? []
              return (
                <li className="hub-card" key={item.slug} data-slug={item.slug}>
                  <div className="hub-card-main">
                    <div className="hub-card-head">
                      <strong>{hubLabel(item, t)}</strong>
                      <code className="hub-slug">{item.slug}</code>
                      {children.length > 0 ? (
                        <StatusBadge tone="neutral">
                          {t('hub.references', {
                            count: children.length,
                            defaultValue: '点名 {{count}} 个下游技能',
                          })}
                        </StatusBadge>
                      ) : null}
                    </div>
                    {summary ? (
                      <p className="hub-summary">{summary}</p>
                    ) : (
                      <p className="field-help">
                        {t('hub.noSummary', {
                          defaultValue: '上游没给说明。没有说明就是没有，不拿 slug 顶替。',
                        })}
                      </p>
                    )}
                  </div>
                  <InstallButton item={item} />
                </li>
              )
            })}
          </ul>
        ) : null}

        {/* 分页栏**在总数未知时也要出现**。
            早先写成 `total !== null || hasMore`，于是上游不给总数（本页正好
            装满、看起来像最后一页）时整个分页栏消失 —— 用户连「还有没有
            下一页」都不知道。拿本页条数冒充「共 N 个」是编数据，
            但把「不知道」连同导航一起藏起来是另一种不诚实。 */}
        {data ? (
          <div className="hub-pager">
            <button
              className="secondary-button"
              disabled={page <= 1}
              onClick={() => setPage((p) => Math.max(1, p - 1))}
            >
              {t('hub.previous', { defaultValue: '上一页' })}
            </button>
            <span className="field-help">
              {t('hub.page', { page, defaultValue: '第 {{page}} 页' })}
              {/* 上游没给总数就说没给。拿本页条数冒充「共 N 个」是编一个数。 */}
              {data.total !== null
                ? t('hub.totalKnown', { total: data.total, defaultValue: '（上游说共 {{total}} 个）' })
                : t('hub.totalUnknown', {
                    defaultValue: '（上游没给总数，这里只是本页）',
                  })}
            </span>
            <button
              className="secondary-button"
              disabled={!hasMore}
              onClick={() => setPage((p) => p + 1)}
            >
              {t('hub.next', { defaultValue: '下一页' })}
            </button>
          </div>
        ) : null}
      </Card>
    </div>
  )
}
