import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ApiError } from '../api/client'
import { Card, ErrorNotice, StatusBadge } from '../components/Page'
import { ViewToggle, useViewMode, viewStorageKey } from '../components/collection'
import { IconRefresh } from './icons'
import {
  EXPERTS_KEY,
  MARKET_KEY,
  MARKET_PAGE_SIZE,
  MARKET_ROUTE,
  type MarketExpert,
  type MarketInstallResult,
  type MarketSkillStatus,
  installMarketExpert,
  listMarketExperts,
  marketLabel,
  marketSummary,
} from './api'

/**
 * 专家市场。
 *
 * ## 这几件事不许含糊
 *
 * 1. **三种空/错状态是三件事**：
 *    - 路由未接通（后端 501）→ 说「没接通」；
 *    - 上游挂了 / 断网（503 等）→ 说「连不上上游」，那是外部服务的问题，
 *      不影响已经装好的专家；
 *    - 上游返回 0 条 → 说「上游说就是 0 个」。
 *    把后两种都画成一张空列表，等于骗用户说「市场里没有」。
 * 2. **上游没给的字段就说没给。** `summary` 空串显示「上游没给说明」，
 *    `total` 为 `null` 显示「共 ? 个」—— 拿本页条数顶替总数是编一个数出来。
 *    但**分页控件在总数未知时也要留着**：藏掉导航是另一种不诚实。
 * 3. **技能装进来一律是停用的。** 这是后端刻意的（实测十几个技能就能把
 *    8192 的上下文顶爆），所以界面上必须写出来，并给出下一步（去技能页启用）。
 *    不写，用户会以为点一下就生效了，然后发现模型根本没用到它。
 * 4. **装了几个就是几个。** `skills[].status` 有三个取值，失败的必须带原因
 *    显示出来 —— 静默吞掉一个 `failed` 会让人以为装全了。
 *
 * i18n key 一律走 `experts.market.*`（与 `i18n/resources.ts` 里的嵌套一致）：
 * 写成 `experts.marketFoo` 会静默 miss，然后落回下面那句中文 defaultValue ——
 * 英文界面于是整段显示中文。
 *
 * ## 视图偏好与「我的专家」共用一个键
 *
 * 两个 tab 是同一个页面的两面，用户在一个里选了列表，切到另一个不该又变回
 * 卡片 —— 那看起来像切换丢了。所以这里传的是 `experts` 的键，不是自己的。
 */

/** 后端这条路由还没接上时是 501，与「上游挂了」的 503 必须分开讲。 */
function isRouteMissing(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 501 || error.code === 'not_implemented')
}

/** 三个技能状态各自的措辞。**新增取值必须在这里显式补一个说法**，
 *  宁可显示原始 status，也不要静默落进「未知」以外的分支。 */
function skillStatusTone(status: MarketSkillStatus): 'success' | 'neutral' | 'danger' {
  if (status === 'installed') return 'success'
  if (status === 'failed') return 'danger'
  return 'neutral'
}

export function MarketTab(): ReactNode {
  const { t } = useTranslation()
  const [page, setPage] = useState(1)
  const { viewMode: view, setViewMode: setView } = useViewMode('experts-market', 'card', viewStorageKey('experts'))
  const query = useQuery({
    queryKey: [...MARKET_KEY, page],
    queryFn: () => listMarketExperts(page),
    // 市场是外部服务代理来的：反复切页不该反复打上游，给 30 秒窗口。
    staleTime: 30_000,
    retry: false,
  })

  const data = query.data
  const items = data?.items ?? []
  // 总数未知时靠「本页是不是装满了」判断还有没有下一页；这是**导航**的判断，
  // 不是拿它当总数显示。
  const hasMore = data ? items.length >= (data.page_size ?? MARKET_PAGE_SIZE) : false

  return (
    <div className="settings-stack">
      <Card
        title={t('experts.marketTitle', { defaultValue: '市场' })}
        description={t('experts.market.description', {
          route: MARKET_ROUTE,
          defaultValue: '数据来自 {{route}}，来源是外部市场的 skillset 目录。这里没有下载量、评分、在线状态 —— 上游没给，我们就不显示。',
        })}
        actions={undefined}
      >
        {/* 控件放正文里而不是卡片的 actions 槽：那个槽只有约 150px 宽，
            看法切换 + 刷新放不下，会折成两行、看起来像没排完。 */}
        <div className="experts-market-controls">
          <ViewToggle
            viewMode={view}
            onChange={setView}
            cardLabel={t('experts.market.viewCard', { defaultValue: '卡片' })}
            listLabel={t('experts.market.viewList', { defaultValue: '列表' })}
            testIdPrefix="experts-market-view"
          />
          <button
            type="button"
            className="experts-toolbar-icon-btn"
            data-testid="experts-market-refresh"
            aria-label={t('common.refresh', { defaultValue: '刷新' })}
            title={t('common.refresh', { defaultValue: '刷新' })}
            disabled={query.isFetching}
            onClick={() => void query.refetch()}
          >
            <IconRefresh />
          </button>
        </div>
        {/* 上游地址必须显示：这**不是内置列表**，断网时用户得知道去哪里看。 */}
        {data?.host ? (
          <p className="field-help experts-market-host">
            {t('experts.market.host', {
              host: data.host,
              defaultValue: '上游是 {{host}}。它挂了或断网时这里连不上 —— 那不影响你已经装好的专家。',
            })}
          </p>
        ) : null}

        {query.isPending ? (
          <p className="empty-card-copy">{t('common.loading', { defaultValue: '加载中…' })}</p>
        ) : null}

        {query.isError ? (
          isRouteMissing(query.error) ? (
            <div className="experts-market-state" role="alert">
              <p className="form-error">
                {t('experts.market.routeMissing', {
                  route: MARKET_ROUTE,
                  defaultValue: '后端这条路由（{{route}}）还没有接通，所以这里没有条目 —— 不是市场是空的。',
                })}
              </p>
              <p className="field-help experts-next-step">
                <strong>{t('experts.nextStepTitle', { defaultValue: '下一步' })}</strong>
                <span>
                  {t('experts.market.routeMissingNext', {
                    defaultValue: '由后端把这条路由接上上游 skillset；接通之前这里不会有任何条目。',
                  })}
                </span>
              </p>
            </div>
          ) : (
            <div className="experts-market-state">
              {/* error 对象原样传进去，别先转字符串：ErrorNotice 自己认 ApiError
                  才会把服务端的中文说明与「下一步：…」分开显示。 */}
              <ErrorNotice error={query.error} />
              <p className="field-help experts-next-step">
                <strong>{t('experts.nextStepTitle', { defaultValue: '下一步' })}</strong>
                <span>
                  {t('experts.market.loadFailedNext', {
                    defaultValue: '确认服务端能连上上游市场；这是外部服务，quill 本地这边不用改。',
                  })}
                </span>
              </p>
            </div>
          )
        ) : null}

        {data && items.length === 0 ? (
          <p className="empty-card-copy">
            {t('experts.market.empty', {
              defaultValue: '上游这一次返回了 0 个专家包 —— 这是上游说的，不是我们没去取，也不是没接通。',
            })}
          </p>
        ) : null}

        {items.length > 0 ? (
          <ul className="experts-market-list" data-view={view}>
            {items.map((item) => (
              <MarketItem key={item.slug} item={item} />
            ))}
          </ul>
        ) : null}

        {/* 分页栏在总数未知时也保留。藏掉它，用户连「还有没有下一页」都不知道。 */}
        {data ? (
          <div className="experts-market-pager">
            <button
              type="button"
              className="secondary-button"
              disabled={page <= 1}
              onClick={() => setPage((current) => Math.max(1, current - 1))}
            >
              {t('experts.market.previous', { defaultValue: '上一页' })}
            </button>
            <span className="field-help">
              {t('experts.market.page', { page, defaultValue: '第 {{page}} 页' })}
              {data.total !== null
                ? t('experts.market.totalKnown', { total: data.total, defaultValue: ' · 共 {{total}} 个' })
                : t('experts.market.totalUnknown', {
                    defaultValue: ' · 共 ? 个（上游没给总数，这里只是本页）',
                  })}
            </span>
            <button
              type="button"
              className="secondary-button"
              disabled={!hasMore}
              onClick={() => setPage((current) => current + 1)}
            >
              {t('experts.market.nextPage', { defaultValue: '下一页' })}
            </button>
          </div>
        ) : null}
      </Card>
    </div>
  )
}

function MarketItem({ item }: { item: MarketExpert }): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()
  const label = marketLabel(item)
  const summary = marketSummary(item)

  const install = useMutation({
    mutationFn: () => installMarketExpert(item.slug),
    onSuccess: async () => {
      // 两边都要失效：市场项的 installed 标记会变，「我的专家」里也多了这一个。
      await Promise.all([
        client.invalidateQueries({ queryKey: MARKET_KEY }),
        client.invalidateQueries({ queryKey: EXPERTS_KEY }),
      ])
    },
  })

  return (
    <li
      className="experts-market-card"
      data-slug={item.slug}
      // 装完那张卡要横跨整行：结果块挤在 1/3 宽的一格里根本读不了。
      data-expanded={install.isSuccess ? 'true' : undefined}
    >
      <div className="experts-market-main">
        <div className="experts-market-head">
          <strong>{label || t('experts.market.noName', { defaultValue: '（上游没给名称）' })}</strong>
          <code className="experts-market-slug">{item.slug}</code>
          {item.installed ? (
            <StatusBadge tone="success">{t('experts.market.installed', { defaultValue: '已安装' })}</StatusBadge>
          ) : null}
          {item.scene ? (
            <StatusBadge tone="neutral">
              {t('experts.market.scene', { scene: item.scene, defaultValue: '场景 {{scene}}' })}
            </StatusBadge>
          ) : null}
        </div>
        {summary ? (
          <p className="experts-market-summary">{summary}</p>
        ) : (
          /* 空说明就说没有。拿 slug 顶替不是说明，是占位符。 */
          <p className="field-help">
            {t('experts.market.noSummary', { defaultValue: '上游没给说明。没有说明就是没有，不拿 slug 顶替。' })}
          </p>
        )}
        <p className="experts-market-meta">
          <span className="experts-card-label">
            {t('experts.market.skills', { count: item.skill_count, defaultValue: '含 {{count}} 个技能' })}
          </span>
          {item.skill_slugs.length > 0 ? (
            <span className="experts-market-skills">{item.skill_slugs.join('、')}</span>
          ) : (
            <span className="experts-market-skills">
              {t('experts.market.noSkills', { defaultValue: '上游没列技能' })}
            </span>
          )}
        </p>
      </div>
      <MarketAction item={item} install={install} />
    </li>
  )
}

function MarketAction({
  item,
  install,
}: {
  item: MarketExpert
  install: ReturnType<typeof useMutation<MarketInstallResult, Error, void>>
}): ReactNode {
  const { t } = useTranslation()

  // **成功结果优先于「已安装」按钮**：装完会失效市场列表重取，
  // 重取回来的项 `installed` 已是 true —— 若先判 installed，
  // 刚渲染出来的「哪几个技能装上了、哪几个没装上」会被一个禁用按钮顶掉，
  // 用户根本来不及看。装完的报告要留在屏幕上。
  if (install.isSuccess) {
    return (
      <div className="experts-market-installed">
        <InstallResult result={install.data} slug={item.slug} />
      </div>
    )
  }

  // 已安装的项按钮直接禁用并改文案：再点一次只会拿到 already_installed，
  // 让用户以为安装又失败了一遍。
  if (item.installed) {
    return (
      <div className="experts-market-action">
        <button type="button" className="secondary-button" disabled>
          {t('experts.market.installed', { defaultValue: '已安装' })}
        </button>
      </div>
    )
  }

  return (
    <div className="experts-market-action">
      <button
        type="button"
        className="primary-button"
        data-testid={`experts-market-install-${item.slug}`}
        disabled={install.isPending}
        onClick={() => install.mutate()}
      >
        {install.isPending
          ? t('experts.market.installing', { defaultValue: '安装中…' })
          : t('experts.market.install', { defaultValue: '安装为我的专家' })}
      </button>
      {/* mutation 的错误必须显示。不显示的话用户点了没反应也不知道为什么。 */}
      {install.isError ? <ErrorNotice error={install.error} /> : null}
    </div>
  )
}

function InstallResult({ result, slug }: { result: MarketInstallResult; slug: string }): ReactNode {
  const { t } = useTranslation()
  const failed = result.skills.filter((skill) => skill.status === 'failed')

  return (
    <div className="experts-market-result" role="status">
      <p className="experts-market-ok">
        {result.already_installed
          ? t('experts.market.alreadyInstalled', {
              name: result.expert.display_name,
              defaultValue: '「{{name}}」已经装过了，这次没有覆盖它。',
            })
          : t('experts.market.installedOk', {
              name: result.expert.display_name,
              id: result.expert.id,
              defaultValue: '已装入专家「{{name}}」（{{id}}）。',
            })}
      </p>
      <p className="experts-market-persona">
        <span className="experts-card-label">
          {t('experts.market.persona', { defaultValue: '人格' })}
        </span>
        <span>
          {/* source_file 为 null = 这次没下载包，人格取自库里那份。
              拿空串当文件名显示，用户会以为文件丢了。 */}
          {result.persona.source_file
            ? t('experts.market.personaBody', {
                file: result.persona.source_file,
                chars: result.persona.chars,
                defaultValue: '{{file}} · {{chars}} 字',
              })
            : t('experts.market.personaFromStore', {
                chars: result.persona.chars,
                defaultValue: '库里那份（{{chars}} 字），这次没有下载包',
              })}
        </span>
      </p>
      <ul className="experts-market-skill-status">
        {result.skills.map((skill) => (
          <li key={skill.slug} data-slug={skill.slug}>
            <code>{skill.slug}</code>
            <StatusBadge tone={skillStatusTone(skill.status)}>
              {t(`experts.market.skill_${skill.status}`, {
                defaultValue: skill.status === 'installed'
                  ? '已装入'
                  : skill.status === 'already_present'
                    ? '本来就有'
                    : '没装上',
              })}
            </StatusBadge>
            {skill.status === 'failed' && skill.reason ? (
              <span className="experts-market-skill-reason">{skill.reason}</span>
            ) : null}
          </li>
        ))}
      </ul>
      {failed.length > 0 ? (
        <p className="field-help">
          {t('experts.market.skillFailedHint', {
            count: failed.length,
            slug,
            defaultValue: '「{{slug}}」里有 {{count}} 个技能没装上 —— 上面写了原因，专家本身已经装好了。',
          })}
        </p>
      ) : null}
      {/* 停用这件事是后端刻意的选择，不写出来用户会以为点一下就生效了。 */}
      <p className="field-help experts-market-disabled-note">
        {t('experts.market.skillsDisabled', {
          defaultValue: '装进来的技能一律是停用的：十几个技能就能把上下文顶爆，所以后端不替你决定要不要全开。',
        })}
      </p>
      <p className="field-help experts-next-step">
        <strong>{t('experts.nextStepTitle', { defaultValue: '下一步' })}</strong>
        <span>
          {t('experts.market.skillsDisabledNext', {
            defaultValue: '去「技能包」页把它需要的那几个打开，模型这一轮才看得到。',
          })}
        </span>
      </p>
    </div>
  )
}