import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice, PageHeader, StatusBadge } from '../components/Page'
import {
  USERS_ROUTE,
  ADMIN_CONFIG_ROUTE,
  DEFAULT_PAGE_SIZE,
  listUsers,
  isAdminUserList,
  setUserStatus,
  userFailure,
  userFailureLabel,
} from './api'
import { AdminTabs } from './AdminTabs'

const USERS_KEY = ['admin-users'] as const

/**
 * 实例设置 —— 现在只**说明**这些配置在哪里改，不画表单。
 *
 * 这一页以前有四张卡片（默认人格 / 外部体检服务 / 配额 / 网络抓取），
 * 每一张的输入框都是 `readOnly`、保存按钮都是 `disabled`，占位一律写「（未接通）」。
 * 四个点不动的按钮比没有按钮更糟：它们看起来是设置入口，用户会一个一个去试，
 * 试完发现按不动，只能得出「这功能坏了」的结论 —— 而真相是后端压根没这条路由。
 *
 * 项目自己的纪律是「不画没有后端的假开关/假按钮」。`{{route}}` 未登记时
 * 唯一诚实的做法是**说清楚它没接通、以及真要改该走哪条路**（环境变量 / CLI）。
 *
 * 参照 octop：它的 admin 区每一项（`/admin/backend`、`/admin/security`、
 * `/admin/plugins`、`/admin/advanced`）都是**真能操作的**页面，
 * 没有一个是这样一张只读展示页（`.octop-ref/octop/dashboard/src/routes/index.tsx:229-237`）。
 * 等 quill 真接上这些配置，这一页再把表单加回来。
 */
export function AdminInstancePage(): ReactNode {
  const { t } = useTranslation()
  return (
    <div className="page-scroll">
      <PageHeader
        eyebrow={t('nav.admin', { defaultValue: '管理' })}
        title={t('admin.settings', { defaultValue: '实例设置' })}
        description={t('admin.settingsDescription', { defaultValue: 'quill 的实例级配置由服务端环境变量决定，页面上只做展示。' })}
        actions={<AdminTabs />}
      />
      <div className="settings-stack">
        <Card
          title={t('admin.instanceNotWiredTitle', { defaultValue: '这一页还没有可改的配置' })}
          description={t('admin.instanceNotWiredDescription', {
            defaultValue: '下面这些配置项后端都还没有对应的读写接口，所以这里既不画输入框，也不画保存按钮。',
          })}
        >
          <p className="field-help full-row" role="status">
            {t('admin.instanceNotWired', {
              defaultValue:
                '以前这里有四张卡片：默认人格、外部体检服务、配额、网络抓取。它们的输入框全是只读、保存按钮全是禁用 —— 四个点不动的按钮看起来是设置入口，用户试完只会以为功能坏了，而真相是 {{route}} 在 quill 里没有登记。下一步：实现 {{route}} 的读写之后再把表单加回来。',
              route: ADMIN_CONFIG_ROUTE,
            })}
          </p>
          <dl className="detail-grid">
            <div>
              <dt>{t('admin.defaultSoul', { defaultValue: '默认人格' })}</dt>
              <dd>{notWired(t, 'default_soul')}</dd>
            </div>
            <div>
              <dt>{t('admin.quotas', { defaultValue: '配额' })}</dt>
              <dd>{notWired(t, 'quota_bytes')}</dd>
            </div>
            <div>
              <dt>{t('admin.webFetch', { defaultValue: '网络抓取' })}</dt>
              <dd>{notWired(t, 'web_fetch_denylist')}</dd>
            </div>
            <div>
              <dt>{t('admin.jevTitle', { defaultValue: '外部体检服务' })}</dt>
              <dd>{t('admin.jevNotWiredShort', { defaultValue: '未接入任何外部体检服务' })}</dd>
            </div>
          </dl>
          <p className="field-help full-row">
            {t('admin.instanceWhereToEdit', {
              defaultValue:
                '现在能改实例级配置的地方是服务端的启动环境变量，以及 {{doctor}}（打印当前生效的配置与诊断）。',
              doctor: 'quill doctor',
            })}
          </p>
        </Card>
      </div>
    </div>
  )
}

/** 一格配置的状态。**逐项说清为什么不能改**，而不是统一写「未接通」。 */
function notWired(
  t: (key: string, options?: Record<string, unknown>) => string,
  field: string,
): string {
  return t('admin.fieldNotWiredShort', {
    field,
    defaultValue: '{{field}}：只读，{{route}} 未登记',
    route: ADMIN_CONFIG_ROUTE,
  })
}

export function AdminUsersPage(): ReactNode {
  const { t } = useTranslation()
  const [offset, setOffset] = useState(0)
  const [pending, setPending] = useState<string | null>(null)
  const qc = useQueryClient()
  const users = useQuery({
    queryKey: [...USERS_KEY, offset],
    queryFn: () => listUsers(offset),
    retry: false,
  })
  const mutation = useMutation({
    mutationFn: ({ id, status }: { id: string; status: 'active' | 'disabled' }) =>
      setUserStatus(id, status),
    onSuccess: () => {
      // 启停之后名册变了。不重取的话，界面上会继续显示改之前的状态 ——
      // 那正是这个项目最不能出的那种错：做了一件事，界面上什么都没发生。
      void qc.invalidateQueries({ queryKey: USERS_KEY })
    },
  })

  const total = users.data?.total ?? 0
  // **逐字段自检后才认这份数据。** 不自检的话，服务端改了字段名或少给一个，
  // 界面上照样渲染出一张「看起来有数据」的表 —— 那正是这一页以前的问题。
  const roster = users.data && isAdminUserList(users.data) ? users.data : undefined
  const unreadable = !!users.data && !roster
  const page = roster?.users ?? []
  // 列表失败与「点停用失败」都要说人话。原来只看 users.error，
  // 结果 409「不能停用自己」点下去之后界面上一个字都不显示 ——
  // 用户只会以为按钮坏了。
  const failure = userFailure(users.error) ?? userFailure(mutation.error)
  const lastPage = offset + DEFAULT_PAGE_SIZE

  return (
    <div className="page-scroll">
      <PageHeader
        eyebrow={t('nav.admin', { defaultValue: '管理' })}
        title={t('admin.users', { defaultValue: '用户' })}
        description={t('admin.usersDescription', { defaultValue: '账号来自服务端的令牌配置与首个 owner 引导。' })}
        actions={<AdminTabs />}
      />
      <ErrorNotice error={users.error} />
      <ErrorNotice error={mutation.error} />
      {unreadable ? (
        <p className="field-help full-row" data-testid="users-unreadable">
          {t('admin.usersUnreadable', {
            defaultValue:
              '服务端回了成功状态，但名册里有字段缺失或类型不对，所以这里不显示任何用户行。这不是一份空名册。',
          })}
        </p>
      ) : null}
      {failure ? (
        <p className="field-help full-row" data-testid="users-failure-help">
          {userFailureLabel(failure, t)}
        </p>
      ) : null}
      <Card
        title={t('admin.totalUsers', { total, defaultValue: '共 {{total}} 个账号' })}
        description={t('admin.usersApiHelp', { defaultValue: '数据来自 {{route}}。', route: USERS_ROUTE })}
      >
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>{t('admin.userColumn', { defaultValue: '用户' })}</th>
                <th>{t('admin.identity', { defaultValue: '身份' })}</th>
                <th>{t('admin.status', { defaultValue: '状态' })}</th>
                <th>{t('admin.actions', { defaultValue: '操作' })}</th>
              </tr>
            </thead>
            <tbody>
              {page.map((user) => (
                <tr key={user.id} data-testid={`user-row-${user.username}`}>
                  <td>
                    <strong>{user.display_name || user.username}</strong>
                    <small>{user.username}</small>
                  </td>
                  <td>
                    <StatusBadge tone={user.role === 'owner' ? 'success' : 'neutral'}>
                      {user.role === 'owner' ? t('admin.adminRole', { defaultValue: '管理员' }) : t('admin.memberRole', { defaultValue: '成员' })}
                    </StatusBadge>
                  </td>
                  <td>
                    <StatusBadge tone={user.status === 'active' ? 'success' : 'danger'}>
                      {user.status === 'active' ? t('admin.normal', { defaultValue: '正常' }) : t('admin.disabled', { defaultValue: '已停用' })}
                    </StatusBadge>
                    {/* 登出收不回环境变量令牌 —— 不说出来，管理员会以为登出就够了。 */}
                    {user.has_env_token ? (
                      <small data-testid={`user-env-token-${user.username}`}>
                        {t('admin.stillHasEnvToken', {
                          defaultValue: '凭据是 QUILL_TOKENS 令牌，登出收不回（停用可以挡住）',
                        })}
                      </small>
                    ) : null}
                  </td>
                  <td>
                    <button
                      className="secondary-button"
                      data-testid={`user-toggle-${user.username}`}
                      disabled={pending === user.id}
                      onClick={() => {
                        setPending(user.id)
                        mutation.mutate(
                          { id: user.id, status: user.status === 'active' ? 'disabled' : 'active' },
                          { onSettled: () => setPending(null) },
                        )
                      }}
                    >
                      {user.status === 'active'
                        ? t('admin.userDisable', { defaultValue: '停用' })
                        : t('admin.userEnable', { defaultValue: '启用' })}
                    </button>
                  </td>
                </tr>
              ))}
              {page.length === 0 && !users.isFetching ? (
                <tr>
                  <td colSpan={4}>{t('admin.noUsers', { defaultValue: '名册是空的。' })}</td>
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
        <div className="form-actions">
          <button className="secondary-button" data-testid="users-previous-page" disabled={offset === 0 || users.isFetching} onClick={() => setOffset(Math.max(0, offset - DEFAULT_PAGE_SIZE))}>
            {t('admin.previousPage', { defaultValue: '上一页' })}
          </button>
          <span>{t('admin.pageNumber', { page: Math.floor(offset / DEFAULT_PAGE_SIZE) + 1, defaultValue: '第 {{page}} 页' })}</span>
          <button className="secondary-button" data-testid="users-next-page" disabled={lastPage >= total || users.isFetching} onClick={() => setOffset(offset + DEFAULT_PAGE_SIZE)}>
            {t('admin.nextPage', { defaultValue: '下一页' })}
          </button>
        </div>
        <p className="field-help full-row" data-testid="users-not-allowed">
          {t('admin.usersCannotCreate', {
            defaultValue:
              '这台实例刻意不提供建号与删号：账号只来自部署配置（QUILL_TOKENS / QUILL_PASSWORD_USERS），软删除也还没落地。停用会挡住口令登录并让已登录的会话失效，账号与它的数据都还在；要加账号请改环境变量后重启。',
          })}
        </p>
      </Card>
    </div>
  )
}
