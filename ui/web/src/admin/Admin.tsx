import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router-dom'

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

const USERS_KEY = ['admin-users'] as const

const JEV_ROUTE = 'POST /api/admin/config/jev/check'

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
        <Card title={t('admin.defaultSoul', { defaultValue: '默认人格' })} description={t('admin.defaultSoulDescription', { defaultValue: '每个用户的第一份 SOUL 模板。' })}>
          <form className="form-grid" onSubmit={(event) => event.preventDefault()}>
            <label className="full-row">
              {t('admin.defaultSoul', { defaultValue: '默认人格' })}
              <textarea name="default_soul" rows={6} maxLength={32000} readOnly placeholder={t('admin.notAvailable', { defaultValue: '（未接通）' })} />
            </label>
            <div className="form-actions full-row">
              <button className="primary-button" disabled>{t('admin.saveDefaultSoul', { defaultValue: '保存默认人格' })}</button>
            </div>
          </form>
          <p className="field-help full-row">{settingsNotice(t, ADMIN_CONFIG_ROUTE, 'default_soul')}</p>
        </Card>
        <Card title={t('admin.jevTitle', { defaultValue: '外部体检服务' })} description={t('admin.jevDescription', { defaultValue: '上游用来校验 provider 凭据的外部服务。' })}>
          <p className="field-help full-row">
            {t('admin.jevNotWired', {
              defaultValue: 'quill 没有接入任何外部体检服务：{{route}} 未登记，因此这里不显示任何可用性状态。下一步：若仍需要这条能力，再实现 {{route}}。',
              route: JEV_ROUTE,
            })}
          </p>
        </Card>
        <Card title={t('admin.quotas', { defaultValue: '配额' })} description={t('admin.quotasDescription', { defaultValue: '个人与共享空间的容量上限。' })}>
          <form className="form-grid" onSubmit={(event) => event.preventDefault()}>
            <label>
              {t('admin.personalQuota', { defaultValue: '个人配额 (MiB)' })}
              <input name="quota_mib" type="number" min="0" step="any" readOnly placeholder={t('admin.notAvailable', { defaultValue: '（未接通）' })} />
            </label>
            <label>
              {t('admin.sharedQuota', { defaultValue: '共享配额 (MiB)' })}
              <input name="shared_quota_mib" type="number" min="0" step="any" readOnly placeholder={t('admin.notAvailable', { defaultValue: '（未接通）' })} />
            </label>
            <div className="form-actions full-row">
              <button className="primary-button" disabled>{t('admin.saveQuotas', { defaultValue: '保存配额' })}</button>
            </div>
          </form>
          <p className="field-help full-row">{settingsNotice(t, ADMIN_CONFIG_ROUTE, 'quota_bytes')}</p>
        </Card>
        <Card title={t('admin.webFetch', { defaultValue: '网络抓取' })} description={t('admin.webFetchDescription', { defaultValue: '抓取外部网页时的屏蔽名单。' })}>
          <form className="form-grid" onSubmit={(event) => event.preventDefault()}>
            <label className="full-row">
              {t('admin.denylist', { defaultValue: '屏蔽名单' })}
              <textarea name="web_fetch_denylist" rows={7} readOnly placeholder={t('admin.notAvailable', { defaultValue: '（未接通）' })} />
            </label>
            <div className="form-actions full-row">
              <button className="primary-button" disabled>{t('admin.saveNetwork', { defaultValue: '保存网络策略' })}</button>
            </div>
          </form>
          <p className="field-help full-row">{settingsNotice(t, ADMIN_CONFIG_ROUTE, 'web_fetch_denylist')}</p>
        </Card>
      </div>
    </div>
  )
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

function AdminTabs(): ReactNode {
  const { t } = useTranslation()
  return (
    <nav className="inline-tabs" aria-label={t('admin.adminPages', { defaultValue: '管理页面' })}>
      <Link to="/admin/models">{t('nav.models', { defaultValue: '模型' })}</Link>
      <Link to="/admin/instance">{t('admin.settings', { defaultValue: '实例设置' })}</Link>
      <Link to="/admin/users">{t('admin.users', { defaultValue: '用户' })}</Link>
      {/* 原来这里链到 /admin/mcp「共享 MCP」。它与左侧的「MCP 服务」是同一份数据、
          同一个接口，只是另一套表单 —— 而 quill 只有实例级一套，「共享」暗示的
          分用户分享并不存在。两处改一边另一边不跟着变，最终必然漂移，所以指向同一处。 */}
      <Link to="/devices">{t('nav.mcpServers', { defaultValue: 'MCP 服务' })}</Link>
    </nav>
  )
}

function settingsNotice(t: (key: string, options?: Record<string, unknown>) => string, route: string, field: string): string {
  return t('admin.settingsNotWired', {
    defaultValue: '字段 {{field}} 拿不到值：{{route}} 在 quill 里未登记，因此表单保持只读，也不显示任何占位数据。下一步：实现 {{route}} 后再开放保存。',
    field,
    route,
  })
}
