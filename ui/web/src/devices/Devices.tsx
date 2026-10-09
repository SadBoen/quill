import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type FormEvent, type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice, PageHeader } from '../components/Page'
import { CollectionView, ViewToggle, useViewMode } from '../components/collection'
import {
  MCP_ROUTE,
  listMcpServers,
  mcpRowState,
  mcpServersOf,
  mcpStatusOf,
  saveMcpServers,
  type McpRowState,
  type McpServerConfig,
  type McpServerList,
  type McpServerStatus,
} from './api'
import {
  type EditableMcpServer,
  formatMcpSecrets,
  MCP_NAME_PATTERN,
  mcpCapabilityMode,
  mcpSecretMap,
  readMcpForm,
} from './mcpConfig'
import './devices-row.css'

const MCP_KEY = ['mcp-servers'] as const

type McpDraft = { servers: McpServerConfig[] }
type McpEditing = { server: McpServerConfig; catalog?: DiscoveredServer }

type DiscoveredCapability = { raw_name: string; final_name: string; enabled: boolean }
export type DiscoveredServer = {
  tools: DiscoveredCapability[]
  resources: DiscoveredCapability[]
  resource_templates: DiscoveredCapability[]
  prompts: DiscoveredCapability[]
}

function serverAddress(server: McpServerConfig): string {
  // stdio 与 builtin 的「地址」都是 `command` 那一列：前者是可执行文件，
  // 后者是内置服务器的名字（见 `quill_core::builtin`）。
  if (server.transport === 'stdio' || server.transport === 'builtin') return server.command ?? ''
  return server.url ?? ''
}

/**
 * ISSUE-010：草稿曾经与已保存行渲染成**一模一样**的卡片 —— 同样的图标、
 * 同样的传输方式、同样的「编辑 / 删除」。用户以为存好了就关掉标签页，
 * 这条配置就永远丢了（实测：`curl /api/extensions/mcp` 里根本没有它）。
 *
 * 现在草稿行必须挂上「未保存」标记，而且删除按钮的文案要跟着变：
 * 对一条**服务端根本没有**的行说「删除」，文案本身就在骗人 ——
 * 那个按钮只把本地草稿里那一项拿掉。
 */
function unsavedBadge(state: McpRowState, t: (key: string, opt?: Record<string, unknown>) => string): ReactNode {
  if (state === 'saved') return null
  return (
    <span className="draft-badge" data-testid="mcp-unsaved-badge" data-state={state} role="status">
      {state === 'new'
        ? t('mcp.unsavedNew', { defaultValue: '未保存（新增）' })
        : t('mcp.unsavedChanged', { defaultValue: '未保存（有改动）' })}
    </span>
  )
}

/** 把「关掉这个标签页就丢了」说在前面，而不是等用户自己撞上。 */
function draftBanner(count: number, t: (key: string, opt?: Record<string, unknown>) => string): ReactNode {
  if (count <= 0) return null
  return (
    <p className="draft-banner" data-testid="mcp-unsaved-banner" role="status">
      {t('mcp.pendingBanner', {
        count,
        defaultValue: '有 {{count}} 条改动还没保存，关掉这个标签页就丢了。下一步：点右上角「保存 MCP 配置」。',
      })}
    </p>
  )
}

/** 删除按钮的文案必须跟着「这行存没存」变，三种状态三句话。 */
function deleteLabel(
  state: McpRowState,
  name: string,
  t: (key: string, opt?: Record<string, unknown>) => string,
): string {
  if (state === 'new') {
    return t('mcp.discardDraftNamed', { name, defaultValue: '丢弃草稿 {{name}}' })
  }
  if (state === 'changed') {
    return t('mcp.deletePendingNamed', { name, defaultValue: '删除 {{name}}（保存后才生效）' })
  }
  return t('mcp.deleteNamed', { name, defaultValue: '删除 {{name}}' })
}

/** 没草稿时恒为 0 —— 避免服务端归一化过的字段被误判成「用户改过了」。 */
function countUnsaved(rows: McpServerConfig[], saved: McpServerConfig[], hasDraft: boolean): number {
  if (!hasDraft) return 0
  return rows.filter((row) => mcpRowState(row, saved, hasDraft) !== 'saved').length
}

function mcpServerQuery() {
  return {
    queryKey: MCP_KEY,
    queryFn: listMcpServers,
    refetchIntervalInBackground: false,
    staleTime: 10_000,
  }
}

/**
 * 如实显示这一轮的**实测**连通性。
 *
 * 组件里**不许**出现任何根据 `servers.length` 推断出来的「已连接」「可用」
 * 字样 —— 配了不等于连上了，服务器可能没起、地址可能写错、token 可能过期。
 * 唯一可信的来源是服务端给的 `probed` / `connected` / `note`，原样显示。
 *
 * 特别地：**`probed === 0` 时不报「尚未连接验证」**。那是「没查过」，
 * 与「查了、没连上」是两种完全不同的状态，混成同一句红字就等于替用户
 * 猜了一个不存在的故障。
 */
function McpLinkStatus({ body, route }: { body: McpServerList | undefined; route: string }): ReactNode {
  const { t } = useTranslation()
  const probed = body?.probed ?? 0
  return (
    <p className="field-help">
      {t('devices.apiNote', {
        defaultValue: '配置存在 {{route}}，下面每一行都是服务端返回的原文。',
        route,
      })}
      {probed > 0 && body?.connected === true && (
        <>
          {' '}
          <strong>
            {t('devices.mcpConnected', {
              count: body.connected_count ?? probed,
              probed,
              defaultValue: '本轮 {{probed}} 台全部连上，{{count}} 台报了工具。',
            })}
          </strong>
        </>
      )}
      {probed > 0 && body?.connected === false && (
        <>
          {' '}
          <strong>
            {t('devices.mcpNotConnected', {
              defaultValue: '本轮 {{probed}} 台里 {{failed}} 台没连上。',
              probed,
              failed: body.failed_count ?? 0,
            })}
          </strong>
        </>
      )}
      {body?.note ?? ''}
    </p>
  )
}

/**
 * 一台服务器卡片上的实测状态。
 *
 * 三种状态分开说，因为它们的下一步完全不同：
 * 连上了 / 没连上（附服务端给的原因）/ 没探测（附为什么没探测）。
 * 第三种**不许**显示成「失败」—— 停用的服务器不是故障。
 */
function McpProbeStatus({ status }: { status: McpServerStatus | undefined }): ReactNode {
  const { t } = useTranslation()
  if (!status) {
    return (
      <p className="field-help">
        {t('devices.mcpNoStatus', { defaultValue: '本轮没有这一台的探测记录。' })}
      </p>
    )
  }
  if (!status.probed) {
    return (
      <p className="field-help">
        {t('devices.mcpNotProbed', { defaultValue: '未探测：' })}
        {status.error ?? ''}
      </p>
    )
  }
  if (!status.connected) {
    return (
      <p className="field-help">
        <strong>{t('devices.mcpProbeFailed', { defaultValue: '没连上：' })}</strong>
        {status.error ?? ''}
      </p>
    )
  }
  return (
    <>
      <p className="field-help">
        {t('devices.mcpProbeOk', {
          defaultValue: '已连上（协议 {{protocol}}，{{server}}），服务器报了 {{count}} 个工具。',
          protocol: status.protocol_version ?? '—',
          server: status.server_info ?? '—',
          count: status.tool_count,
        })}
      </p>
      {/*
       * 「连上了」与「模型调得到」分开说。`mounted` 是服务端真的挂进工具表的条数，
       * 与 `tool_count` 不一定相等 —— 能力被关掉、挂载名与已有工具撞上、
       * schema 不是 object，这三种都会让条数变少，而它们的症状在模型那边一模一样。
       * 组件里**不许**由 `tool_count` 推断 `mounted`。
       */}
      <p className="field-help">
        <strong>
          {t('devices.mcpMounted', {
            defaultValue: '这一轮真的挂进对话工具表的有 {{count}} 个，模型调得到。',
            count: status.mounted ?? 0,
          })}
        </strong>
      </p>
      {status.mounted_tools?.length ? (
        <ul className="mcp-tool-list">
          {status.mounted_tools.map(([mounted, remote]) => (
            <li key={mounted} className="mcp-tool-item">
              <code>{mounted}</code>
              <span className="mcp-tool-remote">
                {t('devices.mcpToolRemote', {
                  defaultValue: '服务器里的原名：{{remote}}',
                  remote,
                })}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
      {/*
       * **连上了但工具数是 0** 时也要显示原因。这不是连接失败，所以上面两个
       * 分支都走不到；但原因（本地能力开关没开 tools / 服务器自报没有 tools）
       * 是用户唯一能看到的线索 —— 少了它，界面上就是一句「已连上，0 个工具」，
       * 用户无从判断该改自己的配置还是该换服务器。
       */}
      {status.connected && status.error ? (
        <p className="field-help">{status.error}</p>
      ) : null}
      {status.not_mounted?.length ? (
        <div className="mcp-tool-missing">
          <p className="field-help">
            {t('devices.mcpNotMountedTitle', {
              defaultValue: '这些工具没挂上（模型这一轮调不到）：',
            })}
          </p>
          <ul className="mcp-tool-list">
            {status.not_mounted.map(([remote, why]) => (
              <li key={remote} className="mcp-tool-item">
                <code>{remote}</code>
                <span className="mcp-tool-remote">{why}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </>
  )
}

/**
 * quill 没有设备（客户端）名册，设备页改为承载 MCP 扩展服务列表：
 * 数据来自 `GET /api/extensions/mcp`，写入走同名的 `POST`。
 */
export function DeviceListPage(): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()
  const config = useQuery(mcpServerQuery())
  const [draft, setDraft] = useState<McpDraft | null>(null)
  const [editing, setEditing] = useState<McpEditing | null>(null)
  const [formError, setFormError] = useState<string | null>(null)
  const servers = draft?.servers ?? mcpServersOf(config.data)
  // **服务端真正存着的那些**。草稿和它们拼在同一个数组里渲染，
  // 界面上要能分清哪一行还没落库 —— 否则 ISSUE-010 那种「以为存好了」就会发生。
  const savedServers = mcpServersOf(config.data)
  const hasDraft = draft !== null
  const stateOf = (server: McpServerConfig): McpRowState => mcpRowState(server, savedServers, hasDraft)
  const unsavedCount = countUnsaved(servers, savedServers, hasDraft)
  const { viewMode, setViewMode } = useViewMode('devices')

  const save = useMutation({
    mutationFn: (current: McpDraft) => saveMcpServers(current.servers),
    onSuccess: (result) => {
      client.setQueryData(MCP_KEY, result)
      setDraft(null)
      setEditing(null)
    },
  })
  const updateDraft = (nextServers: McpServerConfig[]): void => {
    setDraft({ servers: nextServers })
  }
  const add = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    const existing = editing?.server ?? servers.find((item) => item.name === String(data.get('name'))) ?? null
    const result = readMcpForm(data, existing, false, t)
    if (typeof result === 'string') {
      setFormError(result)
      return
    }
    setFormError(null)
    const created = result as McpServerConfig
    updateDraft([
      ...servers.filter((item) => item.name !== created.name && item.name !== editing?.server.name),
      created,
    ])
    setEditing(null)
  }

  return (
    <div className="page-scroll">
      <PageHeader
        eyebrow={t('devices.eyebrow', { defaultValue: '扩展' })}
        title={t('devices.title', { defaultValue: 'MCP 服务' })}
        description={t('devices.description', { defaultValue: 'quill 通过 MCP 扩展接入外部工具。' })}
        actions={(
          <>
            <button className="secondary-button" data-testid="devices-refresh" onClick={() => void config.refetch()}>
              {t('common.refresh', { defaultValue: '刷新' })}
            </button>
            <button
              className="primary-button"
              disabled={!draft || save.isPending}
              onClick={() => draft && save.mutate(draft)}
            >
              {t('mcp.saveDevice', { defaultValue: '保存 MCP 配置' })}
            </button>
          </>
        )}
      />
      <div className="settings-stack">
        <ErrorNotice error={config.error ?? save.error} />
        {draftBanner(unsavedCount, t)}
        <McpLinkStatus body={config.data} route={MCP_ROUTE} />
        <ViewToggle viewMode={viewMode} onChange={setViewMode} testIdPrefix="devices-view" />
        <CollectionView
          viewMode={viewMode}
          items={servers}
          cardKey={(server) => server.name}
          renderCard={(server) => (
            <McpCard
              server={server}
              state={stateOf(server)}
              address={serverAddress(server)}
              status={mcpStatusOf(config.data, server.name)}
              onEdit={() => {
                setEditing({ server })
                setFormError(null)
              }}
              onDelete={() => updateDraft(servers.filter((item) => item.name !== server.name))}
            />
          )}
          renderList={(server) => (
            <McpRow
              server={server}
              state={stateOf(server)}
              address={serverAddress(server)}
              status={mcpStatusOf(config.data, server.name)}
              onEdit={() => {
                setEditing({ server })
                setFormError(null)
              }}
              onDelete={() => updateDraft(servers.filter((item) => item.name !== server.name))}
            />
          )}
        />
        {config.isPending ? <p className="empty-state">{t('devices.loading', { defaultValue: '加载中…' })}</p> : null}
        {!config.isPending && !servers.length ? <p className="empty-state">{t('devices.empty', { defaultValue: '还没有配置 MCP 服务。' })}</p> : null}
        <Card
          title={editing
            ? t('mcp.editNamed', { name: editing.server.name, defaultValue: '编辑 {{name}}' })
            : t('mcp.addOrReplace', { defaultValue: '添加或替换 MCP 服务' })}
          description={t('mcp.replaceHelp', { defaultValue: '先在表单里改好，再点右上角「保存 MCP 配置」。' })}
        >
          <McpForm
            key={editing?.server.name ?? 'new'}
            onSubmit={add}
            initial={editing?.server}
            catalog={editing?.catalog}
            builtinServers={config.data?.builtin_servers ?? []}
            error={formError}
            onCancel={editing ? () => { setEditing(null); setFormError(null) } : undefined}
          />
        </Card>
      </div>
    </div>
  )
}


export interface McpEntryProps {
  server: McpServerConfig
  state: McpRowState
  address: string
  status: ReturnType<typeof mcpStatusOf>
  onEdit: () => void
  onDelete: () => void
}

/**
 * 一个 MCP 服务的公共部分。两种视图都画同样的四行内容
 * （名称、地址、传输/并发/能力数、探测结果），只是排布不同 ——
 * 内容一样、排版不一样，才叫两种看法而不是两个页面。
 */
function McpFacts({ server }: { server: McpServerConfig }): ReactNode {
  const { t } = useTranslation()
  return (
    <dl className="compact-stats">
      <div>
        <dt>{t('mcp.transport', { defaultValue: '传输方式' })}</dt>
        <dd>{server.transport}</dd>
      </div>
      <div>
        <dt>{t('mcp.maxConcurrency', { defaultValue: '并发上限' })}</dt>
        <dd>{server.max_concurrent_calls ?? '—'}</dd>
      </div>
      <div>
        <dt>{t('mcp.exactNames', { defaultValue: '能力名' })}</dt>
        <dd>{server.enabled_capabilities?.length ?? '—'}</dd>
      </div>
    </dl>
  )
}

/** 编辑 / 删除。两个视图共用，按钮的行为不该因为看法不同而不同。 */
function McpActions({ server, state, onEdit, onDelete }: {
  server: McpServerConfig
  state: McpRowState
  onEdit: () => void
  onDelete: () => void
}): ReactNode {
  const { t } = useTranslation()
  return (
    <div className="form-actions">
      <button
        className="secondary-button"
        aria-label={t('mcp.editNamed', { name: server.name, defaultValue: '编辑 {{name}}' })}
        onClick={onEdit}
      >
        {t('common.edit', { defaultValue: '编辑' })}
      </button>
      <button
        className="danger-link"
        aria-label={deleteLabel(state, server.name, t)}
        onClick={onDelete}
      >
        {state === 'new'
          ? t('mcp.discardDraft', { defaultValue: '丢弃草稿' })
          : t('common.delete', { defaultValue: '删除' })}
      </button>
    </div>
  )
}

function McpCard({ server, state, address, status, onEdit, onDelete }: McpEntryProps): ReactNode {
  const { t } = useTranslation()
  return (
    <article className="device-card" data-testid="mcp-card" data-row-state={state}>
      <div className="device-card-top">
        <span className="device-glyph" aria-hidden="true">{transportGlyph(server.transport)}</span>
        <span className="mcp-summary">
          <span>{formatCapabilityMode(server.enabled_capabilities, t)}</span>
        </span>
      </div>
      <h2>{server.name} {unsavedBadge(state, t)}</h2>
      <p><code>{address}</code></p>
      <McpProbeStatus status={status} />
      <McpFacts server={server} />
      <McpActions server={server} state={state} onEdit={onEdit} onDelete={onDelete} />
    </article>
  )
}

function McpRow({ server, state, address, status, onEdit, onDelete }: McpEntryProps): ReactNode {
  const { t } = useTranslation()
  return (
    <article className="mcp-row" data-testid="mcp-row" data-row-state={state}>
      <div className="mcp-row-main">
        <strong>
          <span className="device-glyph" aria-hidden="true">{transportGlyph(server.transport)}</span>
          {server.name} {unsavedBadge(state, t)}
        </strong>
        <code className="field-help">{address}</code>
        <span className="field-help">{formatCapabilityMode(server.enabled_capabilities, t)}</span>
      </div>
      <McpFacts server={server} />
      <McpProbeStatus status={status} />
      <McpActions server={server} state={state} onEdit={onEdit} onDelete={onDelete} />
    </article>
  )
}

function McpForm({
  onSubmit,
  serverSide = false,
  initial,
  catalog,
  builtinServers = [],
  error,
  onCancel,
}: {
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
  serverSide?: boolean
  initial?: EditableMcpServer
  catalog?: DiscoveredServer
  /** 内置服务器的名字清单（服务端 `builtin_servers`）。空数组 = 还没拿到。 */
  builtinServers?: string[]
  error?: string | null
  onCancel?: () => void
}): ReactNode {
  const { t } = useTranslation()
  const [transport, setTransport] = useState<EditableMcpServer['transport']>(initial?.transport ?? 'streamable_http')
  const [capabilityModeValue, setCapabilityModeValue] = useState(mcpCapabilityMode(initial?.enabled_capabilities))
  const initialSecrets = formatMcpSecrets(mcpSecretMap(initial ?? null))
  const secrets = transport === initial?.transport ? initialSecrets : ''
  const initialConcurrency = initial?.max_concurrent_calls ?? (transport === 'stdio' ? 1 : 8)
  return (
    <form className="form-grid" onSubmit={onSubmit}>
      {initial ? <input type="hidden" name="editing" value="true" /> : null}
      <label>
        {t('mcp.serviceName', { defaultValue: '服务名' })}
        <input name="name" autoComplete="off" spellCheck={false} required pattern={MCP_NAME_PATTERN} placeholder="company-search" defaultValue={initial?.name ?? ''} />
      </label>
      <label>
        {t('mcp.transport', { defaultValue: '传输方式' })}
        <select name="transport" value={transport} onChange={(event) => setTransport(event.target.value as typeof transport)}>
          <option value="streamable_http">Streamable HTTP</option>
          <option value="sse">SSE</option>
          <option value="stdio">stdio</option>
          <option value="builtin">{t('mcp.builtinTransport', { defaultValue: '内置（builtin）' })}</option>
        </select>
      </label>
      {transport === 'stdio' ? (
        <>
          <label>
            {t('mcp.executable', { defaultValue: '可执行文件' })}
            <input name="command" required placeholder="python" defaultValue={initial?.transport === 'stdio' ? initial.command : ''} />
          </label>
          <label>
            {t('mcp.args', { defaultValue: '参数' })}
            <textarea name="args" rows={3} placeholder={'-m\ncalculator_mcp'} defaultValue={initial?.transport === 'stdio' ? (initial.args ?? []).join('\n') : ''} />
          </label>
          <label>
            {t('mcp.workingDirectory', { defaultValue: '工作目录' })}
            <input name="cwd" placeholder={t('mcp.optional', { defaultValue: '可选' })} defaultValue={initial?.transport === 'stdio' ? initial.cwd ?? '' : ''} />
          </label>
        </>
      ) : transport === 'builtin' ? (
        <label className="full-row">
          {t('mcp.builtinServer', { defaultValue: '内置服务器' })}
          {builtinServers.length ? (
            // 名字清单来自服务端（`builtin_servers`）—— 前端不自己维护一份会漂的名单。
            // 已保存的名字不在清单里（服务端换过名字）时把它**留在选项里**：
            // 直接落到第一项等于替用户改配置，而他什么都没点。
            <select
              name="command"
              required
              defaultValue={initial?.transport === 'builtin' && initial.command ? initial.command : builtinServers[0]}
            >
              {(initial?.transport === 'builtin' && initial.command && !builtinServers.includes(initial.command)
                ? [initial.command, ...builtinServers]
                : builtinServers
              ).map((item) => <option key={item} value={item}>{item}</option>)}
            </select>
          ) : (
            // 清单还没拿到（列表没加载出来）：退回自由输入，服务端认不认识它说了算。
            <input name="command" required autoComplete="off" spellCheck={false} placeholder="memory" defaultValue={initial?.transport === 'builtin' ? initial.command : ''} />
          )}
        </label>
      ) : (
        <label className="full-row">
          {t('mcp.url', { defaultValue: '地址' })}
          <input name="url" type="url" autoComplete="off" required placeholder="https://mcp.example.com/mcp" defaultValue={initial && initial.transport !== 'stdio' ? initial.url : ''} />
        </label>
      )}
      {transport !== 'builtin' ? (
        <label className="full-row">
          {transport === 'stdio' ? t('mcp.environment', { defaultValue: '环境变量' }) : t('mcp.headers', { defaultValue: '请求头' })}{' '}
          {t('mcp.keyValueHelp', { defaultValue: '每行一条 KEY=VALUE' })}
          <textarea key={transport} name="secrets" autoComplete="off" rows={3} defaultValue={secrets} />
        </label>
      ) : null}
      <fieldset className="choice-field full-row">
        <legend>{t('mcp.capabilities', { defaultValue: '能力开关' })}</legend>
        <label>
          <input type="radio" name="capability_mode" value="all" checked={capabilityModeValue === 'all'} onChange={(event) => setCapabilityModeValue(event.target.value as typeof capabilityModeValue)} />
          {t('mcp.enableAll', { defaultValue: '全部启用' })}
        </label>
        <label>
          <input type="radio" name="capability_mode" value="none" checked={capabilityModeValue === 'none'} onChange={(event) => setCapabilityModeValue(event.target.value as typeof capabilityModeValue)} />
          {t('mcp.disableAll', { defaultValue: '全部禁用' })}
        </label>
        <label>
          <input type="radio" name="capability_mode" value="exact" checked={capabilityModeValue === 'exact'} onChange={(event) => setCapabilityModeValue(event.target.value as typeof capabilityModeValue)} />
          {t('mcp.exact', { defaultValue: '按名单' })}
        </label>
      </fieldset>
      {capabilityModeValue === 'exact' ? (
        catalog
          ? <CapabilityChoices catalog={catalog} selected={initial?.enabled_capabilities ?? []} />
          : (
            <label className="full-row">
              {t('mcp.exactNames', { defaultValue: '能力名' })}
              <textarea name="enabled_capabilities" rows={2} placeholder="mcp_company_search_search" />
            </label>
          )
      ) : null}
      {serverSide ? (
        <label>
          {t('mcp.maxConcurrency', { defaultValue: '并发上限' })}
          <input name="max_concurrent_calls" type="number" min="1" max="32" defaultValue={initialConcurrency} required />
        </label>
      ) : null}
      {error ? <p className="form-error full-row" role="alert">{error}</p> : null}
      <div className="form-actions full-row">
        {onCancel ? <button type="button" className="secondary-button" onClick={onCancel}>{t('common.cancel', { defaultValue: '取消' })}</button> : null}
        <button className="secondary-button">
          {initial ? t('common.save', { defaultValue: '保存' }) : t('mcp.addDraft', { defaultValue: '加入草稿' })}
        </button>
      </div>
    </form>
  )
}

function transportGlyph(transport: McpServerConfig['transport']): string {
  if (transport === 'stdio') return 'CLI'
  if (transport === 'builtin') return '内置'
  return transport === 'sse' ? 'SSE' : 'HTTP'
}

function capabilityGroups(catalog: DiscoveredServer): Array<[string, DiscoveredCapability[]]> {
  return [
    ['mcp.tools', catalog.tools],
    ['mcp.resources', catalog.resources],
    ['mcp.resourceTemplates', catalog.resource_templates],
    ['mcp.prompts', catalog.prompts],
  ]
}

export function CapabilityCatalog({ catalog }: { catalog: DiscoveredServer }): ReactNode {
  const { t } = useTranslation()
  return (
    <div>
      {capabilityGroups(catalog).map(([label, capabilities]) => capabilities.length ? (
        <div key={label}>
          <strong>{t(label, { defaultValue: label })}</strong>
          <ul>{capabilities.map((capability) => <li key={capability.final_name}><code>{capability.final_name}</code></li>)}</ul>
        </div>
      ) : null)}
    </div>
  )
}

function CapabilityChoices({ catalog, selected }: { catalog: DiscoveredServer; selected: string[] }): ReactNode {
  const { t } = useTranslation()
  return (
    <fieldset className="choice-field full-row">
      <legend>{t('mcp.exactNames', { defaultValue: '能力名' })}</legend>
      {capabilityGroups(catalog).map(([label, capabilities]) => capabilities.length ? (
        <div key={label}>
          <strong>{t(label, { defaultValue: label })}</strong>
          {capabilities.map((capability) => (
            <label key={capability.final_name}>
              <input name="enabled_capabilities" type="checkbox" value={capability.final_name} defaultChecked={selected.includes(capability.final_name)} />
              {capability.final_name}
            </label>
          ))}
        </div>
      ) : null)}
    </fieldset>
  )
}

function formatCapabilityMode(
  value: string[] | null | undefined,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  if (value == null) return t('mcp.disableAll', { defaultValue: '全部禁用' })
  if (value.length === 0) return t('mcp.enableAll', { defaultValue: '全部启用' })
  return t('mcp.exactCount', { count: value.length, defaultValue: '按名单启用 {{count}} 个' })
}
