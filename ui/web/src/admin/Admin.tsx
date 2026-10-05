import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type FormEvent, type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router-dom'

import { Card, ErrorNotice, PageHeader, StatusBadge } from '../components/Page'
import { MCP_ROUTE, listMcpServers, saveMcpServers, type McpServerConfig } from '../devices/api'
import { CapabilityCatalog, McpForm, type DiscoveredServer } from '../devices/Devices'
import { readMcpForm } from '../devices/mcpConfig'
import { USERS_ROUTE, ADMIN_CONFIG_ROUTE, listUsers } from './api'

const USERS_KEY = ['admin-users'] as const
const MCP_KEY = ['mcp-servers'] as const
const USER_PAGE_SIZE = 50

const JEV_ROUTE = 'POST /api/admin/config/jev/check'

type McpDraft = { servers: McpServerConfig[] }
type AdminMcpServer = McpServerConfig

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
  const users = useQuery({
    queryKey: [...USERS_KEY, offset],
    queryFn: listUsers,
    retry: false,
  })

  return (
    <div className="page-scroll">
      <PageHeader
        eyebrow={t('nav.admin', { defaultValue: '管理' })}
        title={t('admin.users', { defaultValue: '用户' })}
        description={t('admin.usersDescription', { defaultValue: 'quill 的账号来自服务端的令牌配置。' })}
        actions={<AdminTabs />}
      />
      <ErrorNotice error={users.error} />
      <Card
        title={users.data ? t('admin.pageUsers', { count: users.data.length, defaultValue: '本页 {{count}} 个用户' }) : t('admin.users', { defaultValue: '用户' })}
        description={t('admin.usersApiHelp', { defaultValue: '数据来自 {{route}}。', route: USERS_ROUTE })}
      >
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>{t('admin.userColumn', { defaultValue: '用户' })}</th>
                <th>{t('admin.identity', { defaultValue: '身份' })}</th>
                <th>{t('admin.status', { defaultValue: '状态' })}</th>
              </tr>
            </thead>
            <tbody>
              {users.data?.map((user) => (
                <tr key={user.id}>
                  <td><strong>{user.name}</strong><small>{user.email}</small></td>
                  <td>
                    <StatusBadge tone={user.is_admin ? 'success' : 'neutral'}>
                      {user.is_admin ? t('admin.adminRole', { defaultValue: '管理员' }) : t('admin.memberRole', { defaultValue: '成员' })}
                    </StatusBadge>
                  </td>
                  <td>
                    <StatusBadge tone={user.locked ? 'danger' : 'success'}>
                      {user.locked ? t('admin.overQuota', { defaultValue: '超配额' }) : t('common.normal', { defaultValue: '正常' })}
                    </StatusBadge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="form-actions">
          <button className="secondary-button" data-testid="users-previous-page" disabled={offset === 0 || users.isFetching} onClick={() => setOffset((value) => Math.max(0, value - USER_PAGE_SIZE))}>
            {t('admin.previousPage', { defaultValue: '上一页' })}
          </button>
          <span>{t('admin.pageNumber', { page: Math.floor(offset / USER_PAGE_SIZE) + 1, defaultValue: '第 {{page}} 页' })}</span>
          <button className="secondary-button" data-testid="users-next-page" disabled={(users.data?.length ?? 0) < USER_PAGE_SIZE || users.isFetching} onClick={() => setOffset((value) => value + USER_PAGE_SIZE)}>
            {t('admin.nextPage', { defaultValue: '下一页' })}
          </button>
        </div>
        <p className="field-help full-row">
          {t('admin.usersNotWired', {
            defaultValue: '{{route}} 在 quill 里已登记但尚未实现，所以这里没有用户列表，也不会显示任何示例行。下一步：实现 {{route}} 后，本表自动就有数据；删除/改权限请在服务端的 QUILL_TOKENS 里操作。',
            route: USERS_ROUTE,
          })}
        </p>
      </Card>
    </div>
  )
}

/** 共享 MCP 服务页；quill 只有一套实例级 MCP，路由为 `/api/extensions/mcp`。 */
export function AdminMcpPage(): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()
  const config = useQuery({ queryKey: MCP_KEY, queryFn: listMcpServers, retry: false })
  const [draft, setDraft] = useState<McpDraft | null>(null)
  const [editing, setEditing] = useState<{ server: AdminMcpServer; catalog?: DiscoveredServer } | null>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const servers = draft?.servers ?? (config.data?.servers as AdminMcpServer[] | undefined) ?? []

  const save = useMutation({
    mutationFn: (current: McpDraft) => saveMcpServers(current.servers),
    onSuccess: (result) => {
      client.setQueryData(MCP_KEY, result)
      setDraft(null)
      setEditing(null)
    },
  })
  const updateDraft = (nextServers: AdminMcpServer[]): void => {
    setDraft({ servers: nextServers })
  }
  const add = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const existing = editing?.server ?? servers.find((item) => item.name === String(data.get('name'))) ?? null
    const result = readMcpForm(data, existing, true, t)
    if (typeof result === 'string') {
      setFormError(result)
      return
    }
    setFormError(null)
    const created = result as AdminMcpServer
    updateDraft([
      ...servers.filter((item) => item.name !== created.name && item.name !== editing?.server.name),
      created,
    ])
    setEditing(null)
  }

  return (
    <div className="page-scroll">
      <PageHeader
        eyebrow={t('nav.admin', { defaultValue: '管理' })}
        title={t('admin.sharedMcp', { defaultValue: '共享 MCP' })}
        description={t('admin.sharedMcpDescription', { defaultValue: '对整个 quill 实例生效的 MCP 服务。' })}
        actions={(
          <>
            <AdminTabs />
            <button className="secondary-button" data-testid="server-mcp-refresh" onClick={() => void config.refetch()}>
              {t('common.refresh', { defaultValue: '刷新' })}
            </button>
            <button className="primary-button" disabled={draft === null || save.isPending} onClick={() => draft && save.mutate(draft)}>
              {t('admin.saveSharedMcp', { defaultValue: '保存共享 MCP' })}
            </button>
          </>
        )}
      />
      <ErrorNotice error={config.error ?? save.error} />
      <div className="settings-stack">
        {servers.map((server) => {
          const catalog = editing?.server.name === server.name ? editing.catalog : undefined
          return (
            <Card
              key={server.name}
              title={server.name}
              description={server.transport}
              actions={(
                <>
                  <button
                    className="secondary-button"
                    onClick={() => {
                      setEditing({ server, catalog })
                      setFormError(null)
                    }}
                  >
                    {t('common.edit', { defaultValue: '编辑' })}
                  </button>
                  <button className="danger-link" onClick={() => updateDraft(servers.filter((item) => item.name !== server.name))}>
                    {t('common.delete', { defaultValue: '删除' })}
                  </button>
                </>
              )}
            >
              <div className="mcp-summary">
                <code>{server.transport === 'stdio' ? server.command : server.url}</code>
                <span>{t('mcp.maxConcurrency', { defaultValue: '并发上限' })}: {server.max_concurrent_calls}</span>
              </div>
              {catalog ? <CapabilityCatalog catalog={catalog} /> : null}
            </Card>
          )
        })}
        <Card
          title={t('admin.addSharedMcp', { defaultValue: '添加共享 MCP' })}
          description={t('admin.addSharedMcpHelp', { defaultValue: '填好后先点「保存共享 MCP」。' })}
        >
          <p className="field-help">
            {t('admin.mcpRouteNote', {
              defaultValue: '数据来自 {{route}}。quill 只有一套实例级 MCP，没有「服务端覆盖设备」的那层，所以本页与「扩展 · MCP 服务」是同一份配置的两种入口。',
              route: MCP_ROUTE,
            })}
          </p>
          <McpForm
            key={editing?.server.name ?? 'new'}
            onSubmit={add}
            serverSide
            initial={editing?.server}
            catalog={editing?.catalog}
            error={formError}
            onCancel={editing ? () => { setEditing(null); setFormError(null) } : undefined}
          />
        </Card>
      </div>
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
      <Link to="/admin/mcp">{t('admin.sharedMcp', { defaultValue: '共享 MCP' })}</Link>
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
