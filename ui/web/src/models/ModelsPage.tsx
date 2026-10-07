import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type FormEvent, type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { chatErrorMessage } from '../chat/chatApi'
import { Card, ErrorNotice, PageHeader, StatusBadge } from '../components/Page'
import { CollectionView, ViewToggle, useViewMode } from '../components/collection'
import {
  MODELS_ROUTE,
  PROVIDERS_ROUTE,
  type Model,
  type ModelPool,
  type PoolEntry,
  type Provider,
  type ProviderInput,
  type ProviderProtocol,
  createProvider,
  deleteProvider,
  listProviderModels,
  listProviders,
  loadModelPool,
  setDefaultProvider,
  setProviderEnabled,
  setProviderModel,
  updateProvider,
} from './api'
import {
  CUSTOM_PRESET_ID,
  type PresetProvider,
  type PresetTab,
  presetDefaultBaseUrl,
  presetProvidersOf,
  presetSiteCount,
  presetVendorsOf,
} from './presets'
import './models.css'
import './models-row.css'

const PROVIDERS_KEY = ['admin-providers'] as const
const POOL_KEY = ['admin-models'] as const
const PROTOCOLS: ProviderProtocol[] = ['openai', 'anthropic']

/** 本页用到的全部 i18n key，接线时并入 `src/i18n/resources.ts` 即可。 */
export const MODEL_I18N_KEYS = [
  'models.title',
  'models.eyebrow',
  'models.description',
  'models.refresh',
  'models.poolTitle',
  'models.poolHint',
  'models.poolAvailable',
  'models.poolGroupCount',
  'models.poolEmpty',
  'models.poolUnavailable',
  'models.poolUnavailableHint',
  'models.modelPath',
  'models.modalityText',
  'models.modalityUnknown',
  'models.contextLabel',
  'models.contextUnknown',
  'models.starModel',
  'models.unstarModel',
  'models.testConnection',
  'models.probeSucceeded',
  'models.probeFailed',
  'models.probeRouteNote',
  'models.presetsTitle',
  'models.tabCloud',
  'models.tabLocal',
  'models.vendorProgress',
  'models.authorized',
  'models.awaitingKey',
  'models.notConfigured',
  'models.baseUrlLabel',
  'models.modelLabel',
  'models.sitesLabel',
  'models.sitesCount',
  'models.pickSiteFirst',
  'models.configure',
  'models.configured',
  'models.isDefault',
  'models.saveProvider',
  'models.saveCustom',
  'models.cancel',
  'models.delete',
  'models.confirmDelete',
  'models.setDefault',
  'models.name',
  'models.baseUrl',
  'models.apiKey',
  'models.apiKeyStored',
  'models.apiKeyEmpty',
  'models.apiKeyHelp',
  'models.protocol',
  'models.protocolOpenai',
  'models.protocolAnthropic',
  'models.maxContextTokens',
  'models.compactionThreshold',
  'models.maxOutputTokens',
  'models.customTitle',
  'models.customDescription',
  'models.credentialsNote',
  'models.loadProvidersFailed',
  'models.loadPoolFailed',
  'models.mutateFailed',
  'models.nameRequired',
  'models.baseUrlRequired',
  'models.numberRequired',
] as const

export function ModelsPage(): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()
  const providers = useQuery({ queryKey: PROVIDERS_KEY, queryFn: listProviders, retry: false })
  const pool = useQuery({ queryKey: POOL_KEY, queryFn: loadModelPool, retry: false })
  const [tab, setTab] = useState<PresetTab>('cloud')
  const [openPresetId, setOpenPresetId] = useState<string | null>(null)
  const [openCustomId, setOpenCustomId] = useState<string | null>(null)
  const [creatingCustom, setCreatingCustom] = useState(false)
  const { viewMode, setViewMode } = useViewMode('models')

  const invalidate = async (): Promise<void> => {
    await Promise.all([
      client.invalidateQueries({ queryKey: PROVIDERS_KEY }),
      client.invalidateQueries({ queryKey: POOL_KEY }),
    ])
  }

  const savePreset = useMutation({
    mutationFn: (input: { provider: Provider | undefined; payload: ProviderInput }) =>
      input.provider
        ? updateProvider(input.provider.id, input.payload)
        : createProvider(input.payload),
    onSuccess: async () => {
      setOpenPresetId(null)
      await invalidate()
    },
  })

  const remove = useMutation({
    mutationFn: (id: string) => deleteProvider(id),
    onSuccess: async () => {
      setOpenPresetId(null)
      await invalidate()
    },
  })

  const makeDefault = useMutation({
    mutationFn: (id: string) => setDefaultProvider(id),
    onSuccess: invalidate,
  })

  const star = useMutation({
    mutationFn: (entry: PoolEntry) => setProviderModel(entry.provider_id, entry.model.id),
    onSuccess: invalidate,
  })

  const createCustom = useMutation({
    mutationFn: (input: ProviderInput) => createProvider(input),
    onSuccess: async () => {
      setCreatingCustom(false)
      await invalidate()
    },
  })

  const saveCustom = useMutation({
    mutationFn: (input: { id: string; payload: ProviderInput }) => updateProvider(input.id, input.payload),
    onSuccess: async () => {
      setOpenCustomId(null)
      await invalidate()
    },
  })

  const toggleEnabled = useMutation({
    mutationFn: (input: { id: string; enabled: boolean }) => setProviderEnabled(input.id, input.enabled),
    onSuccess: invalidate,
  })

  const configured = providers.data ?? []
  // 已经建好的自定义端点：启动时从环境变量种下来的那个就属于这一类。
  // 它不对应任何预设卡片，所以必须在这里单独列出来，否则页面上无从改、也无从删。
  const customProviders = configured.filter((p) => p.kind === 'custom')
  const presetMutationError = savePreset.error ?? remove.error ?? makeDefault.error

  return (
    <div className="page-scroll models-page">
      <PageHeader
        eyebrow={t('models.eyebrow', { defaultValue: '管理' })}
        title={t('models.title', { defaultValue: '模型管理' })}
        description={t('models.description', { defaultValue: '配置 LLM 提供商和 API 密钥' })}
        actions={(
          <button
            className="secondary-button"
            data-testid="models-refresh"
            onClick={() => {
              void providers.refetch()
              void pool.refetch()
            }}
          >
            {t('models.refresh', { defaultValue: '刷新' })}
          </button>
        )}
      />

      <div className="settings-stack">
        <PoolCard
          pool={pool.data}
          providers={configured}
          isPending={pool.isPending}
          error={pool.error}
          busy={star.isPending}
          onStar={(entry) => star.mutate(entry)}
        />
        <ErrorNotice
          error={star.error
            ? chatErrorMessage(star.error, t('models.mutateFailed', { defaultValue: '操作失败。' }))
            : null}
        />

        <Card
          title={t('models.presetsTitle', { defaultValue: '预设提供商' })}
          actions={(
            <div className="models-toolbar">
              <div className="models-tabs" role="tablist">
                {(['cloud', 'local'] as PresetTab[]).map((value) => (
                  <button
                    key={value}
                    type="button"
                    role="tab"
                    className="models-tab"
                    aria-selected={tab === value}
                    onClick={() => setTab(value)}
                  >
                    {value === 'cloud'
                      ? t('models.tabCloud', { defaultValue: '云端' })
                      : t('models.tabLocal', { defaultValue: '本地' })}
                  </button>
                ))}
              </div>
              <ViewToggle viewMode={viewMode} onChange={setViewMode} testIdPrefix="models-view" />
            </div>
          )}
        >
          <ErrorNotice
            error={providers.error
              ? chatErrorMessage(providers.error, t('models.loadProvidersFailed', { defaultValue: '提供商列表加载失败。' }))
              : presetMutationError
                ? chatErrorMessage(presetMutationError, t('models.mutateFailed', { defaultValue: '操作失败。' }))
                : null}
          />
          {providers.data ? (
            <div className="models-vendors">
              {presetVendorsOf(tab).map((entry) => {
                const presets = presetProvidersOf(entry.vendor)
                const done = presets.filter((preset) => configured.some((p) => p.preset_id === preset.preset_id)).length
                return (
                  <section className="models-vendor" key={entry.vendor}>
                    <header className="models-vendor-head">
                      <strong>{entry.vendor}</strong>
                      <span className="models-count">
                        {t('models.vendorProgress', {
                          configured: done,
                          total: presets.length,
                          defaultValue: '{{configured}}/{{total}}',
                        })}
                      </span>
                    </header>
                    <CollectionView
                      viewMode={viewMode}
                      items={presets}
                      cardKey={(preset) => preset.preset_id}
                      renderCard={(preset) => {
                        const provider = configured.find((p) => p.preset_id === preset.preset_id)
                        return (
                          <PresetCard
                            preset={preset}
                            provider={provider}
                            open={openPresetId === preset.preset_id}
                            isPending={savePreset.isPending}
                            deletePending={remove.isPending}
                            defaultPending={makeDefault.isPending}
                            onToggle={() => setOpenPresetId((current) => (
                              current === preset.preset_id ? null : preset.preset_id
                            ))}
                            onSave={(payload) => savePreset.mutate({ provider, payload })}
                            onDelete={remove.mutate}
                            onSetDefault={makeDefault.mutate}
                          />
                        )
                      }}
                      renderList={(preset) => {
                        const provider = configured.find((p) => p.preset_id === preset.preset_id)
                        return (
                          <PresetRow
                            preset={preset}
                            provider={provider}
                            open={openPresetId === preset.preset_id}
                            isPending={savePreset.isPending}
                            deletePending={remove.isPending}
                            defaultPending={makeDefault.isPending}
                            onToggle={() => setOpenPresetId((current) => (
                              current === preset.preset_id ? null : preset.preset_id
                            ))}
                            onSave={(payload) => savePreset.mutate({ provider, payload })}
                            onDelete={remove.mutate}
                            onSetDefault={makeDefault.mutate}
                          />
                        )
                      }}
                    />
                  </section>
                )
              })}
            </div>
          ) : null}
        </Card>

        <Card
          title={t('models.customTitle', { defaultValue: '自定义提供商' })}
          description={t('models.customDescription', {
            defaultValue: '任何兼容 OpenAI Chat Completions 或 Anthropic Messages 的地址都可以填在这里。',
          })}
        >
          <ErrorNotice
            error={createCustom.error || saveCustom.error || toggleEnabled.error
              ? chatErrorMessage(
                createCustom.error ?? saveCustom.error ?? toggleEnabled.error,
                t('models.mutateFailed', { defaultValue: '保存失败。' }),
              )
              : null}
          />
          {customProviders.length > 0 ? (
            <CollectionView
              viewMode={viewMode}
              items={customProviders}
              cardKey={(provider) => provider.id}
              renderCard={(provider) => (
                <CustomProviderCard
                  provider={provider}
                  open={openCustomId === provider.id}
                  isPending={saveCustom.isPending}
                  deletePending={remove.isPending}
                  defaultPending={makeDefault.isPending}
                  togglePending={toggleEnabled.isPending}
                  onToggle={() => setOpenCustomId((current) => (current === provider.id ? null : provider.id))}
                  onSave={(payload) => saveCustom.mutate({ id: provider.id, payload })}
                  onDelete={remove.mutate}
                  onSetDefault={makeDefault.mutate}
                  onToggleEnabled={(enabled) => toggleEnabled.mutate({ id: provider.id, enabled })}
                />
              )}
              renderList={(provider) => (
                <CustomProviderRow
                  provider={provider}
                  open={openCustomId === provider.id}
                  isPending={saveCustom.isPending}
                  deletePending={remove.isPending}
                  defaultPending={makeDefault.isPending}
                  togglePending={toggleEnabled.isPending}
                  onToggle={() => setOpenCustomId((current) => (current === provider.id ? null : provider.id))}
                  onSave={(payload) => saveCustom.mutate({ id: provider.id, payload })}
                  onDelete={remove.mutate}
                  onSetDefault={makeDefault.mutate}
                  onToggleEnabled={(enabled) => toggleEnabled.mutate({ id: provider.id, enabled })}
                />
              )}
            />
          ) : (
            <p className="field-help full-row">
              {t('models.customEmpty', {
                defaultValue: '还没有自定义端点。启动时从服务端环境变量种下来的那个端点会出现在这里，可以改名、停用或删除。',
              })}
            </p>
          )}
          {creatingCustom ? (
            <ProviderForm
              id="models-custom-form"
              initial={CUSTOM_DEFAULTS}
              submitLabel={t('models.saveCustom', { defaultValue: '保存自定义提供商' })}
              hasStoredKey={false}
              isPending={createCustom.isPending}
              onSubmit={(payload) => createCustom.mutate(payload)}
              onCancel={() => setCreatingCustom(false)}
            />
          ) : (
            <div className="form-actions full-row">
              <button
                type="button"
                className="secondary-button"
                data-testid="models-custom-new"
                onClick={() => setCreatingCustom(true)}
              >
                {t('models.addCustom', { defaultValue: '新增自定义提供商' })}
              </button>
            </div>
          )}
          <p className="field-help full-row">
            {t('models.credentialsNote', {
              defaultValue: '数据来自 {{route}}：服务端只回 has_api_key，密码框永远不回填已保存的密钥。',
              route: PROVIDERS_ROUTE,
            })}
          </p>
        </Card>
      </div>
    </div>
  )
}

const CUSTOM_DEFAULTS: ProviderInput = {
  name: '',
  preset_id: CUSTOM_PRESET_ID,
  kind: 'custom',
  protocol: 'openai',
  base_url: '',
  model: '',
  max_context_tokens: 0,
  compaction_threshold_tokens: 0,
  max_output_tokens: 0,
}

/**
 * 「你配置的上限比模型实际能吞的大」——这一条不提示，用户就只会看到
 * 一条 `exceed_context_size_error` 的 503，而完全不知道根因是自己的配置。
 * 本机实测：配置 32768、真实窗口 8192，差 4 倍。
 *
 * **只在探测值**小于**配置值时出现。探测值缺失（`null`）不出现** ——
 * 那是「不知道」，不是「一致」，把「不知道」当成没问题是另一种谎。
 */
function ContextMismatchWarning({ group }: { group: PoolGroup }): ReactNode {
  const { t } = useTranslation()
  const probed = contextMismatch(group)
  if (probed === null || group.configuredContext === null) return null
  return (
    <p className="models-context-mismatch" role="status">
      {t('models.contextMismatch', {
        configured: formatContextWindow(group.configuredContext),
        probed: formatContextWindow(probed),
        defaultValue:
          '注意：上面「上下文」探测到的是模型实际能吞 {{probed}}，比你配置的 {{configured}} 小。'
          + '按 {{configured}} 算的话，请求会超过模型真实窗口而被直接拒绝；'
          + '下一步：把「上下文长度」改成 {{probed}}，或调大模型服务的 -c 参数。',
      })}
    </p>
  )
}

function PoolCard({
  pool,
  providers,
  isPending,
  error,
  busy,
  onStar,
}: {
  pool: ModelPool | undefined
  providers: Provider[]
  isPending: boolean
  error: unknown
  busy: boolean
  onStar: (entry: PoolEntry) => void
}): ReactNode {
  const { t } = useTranslation()
  const groups = groupPool(pool?.pool ?? [], providers)
  return (
    <Card
      title={(
        <span className="models-title-with-hint">
          {t('models.poolTitle', { defaultValue: '可用模型池' })}
          <span className="info-button" aria-hidden="true">i</span>
        </span>
      )}
      description={t('models.poolHint', {
        defaultValue: '以下模型均可用于自动路由。系统会根据每条消息的特征智能选择最优模型，⭐ 标记的偏好模型在无特殊需求时会被优先使用。',
      })}
      actions={pool ? (
        <StatusBadge tone="success">
          {t('models.poolAvailable', { count: pool.pool.length, defaultValue: '{{count}} 个可用' })}
        </StatusBadge>
      ) : null}
    >
      <ErrorNotice error={error} />
      {!error && !pool ? (
        <p className="empty-card-copy">
          {isPending
            ? t('common.loading', { defaultValue: '加载中…' })
            : t('models.loadPoolFailed', { defaultValue: '模型池加载失败。' })}
        </p>
      ) : null}
      {pool && groups.length === 0 ? (
        <p className="empty-card-copy">
          {t('models.poolEmpty', { defaultValue: '还没有可用模型：先在下面配置一个提供商。' })}
        </p>
      ) : null}
      {groups.map((group) => (
        <section className="models-pool-group" key={group.providerId}>
          <header className="models-pool-head">
            <strong>{group.name}</strong>
            {group.protocol ? <span className="models-protocol">{group.protocol}</span> : null}
            <span className="models-count">
              {t('models.poolGroupCount', { count: group.entries.length, defaultValue: '{{count}} 个' })}
            </span>
          </header>
          <ContextMismatchWarning group={group} />
          <ul className="models-pool-list">
            {group.entries.map((entry) => (
              <li key={`${entry.provider_id}/${entry.model.id}`}>
                <button
                  type="button"
                  className="models-star"
                  disabled={busy}
                  aria-pressed={entry.starred}
                  aria-label={entry.starred
                    ? t('models.unstarModel', { defaultValue: '取消偏好模型' })
                    : t('models.starModel', { defaultValue: '设为偏好模型' })}
                  onClick={() => onStar(entry)}
                >
                  {entry.starred ? '★' : '☆'}
                </button>
                <span className="models-model-path" title={entry.model.id}>
                  {entry.provider_name} / {entry.model.display_name || entry.model.id}
                </span>
                <ModelTags model={entry.model} />
                <ProbeButton providerId={entry.provider_id} />
              </li>
            ))}
          </ul>
        </section>
      ))}
      {pool && pool.unavailable.length > 0 ? (
        <section className="models-pool-unavailable">
          <h3>{t('models.poolUnavailable', { defaultValue: '暂不可用的提供商' })}</h3>
          <ul>
            {pool.unavailable.map((entry) => (
              <li key={entry.provider_id}>
                <strong>{entry.provider_name}</strong>
                <span>{entry.error}</span>
              </li>
            ))}
          </ul>
          <p className="field-help">
            {t('models.poolUnavailableHint', {
              defaultValue: '以上提供商在 {{route}} 探测时失败，原文来自服务端。',
              route: MODELS_ROUTE,
            })}
          </p>
        </section>
      ) : null}
      {pool && pool.disabled.length > 0 ? (
        <section className="models-pool-unavailable">
          <h3>{t('models.poolDisabled', { defaultValue: '已停用的提供商' })}</h3>
          <ul>
            {pool.disabled.map((entry) => (
              <li key={entry.provider_id}>
                <strong>{entry.provider_name}</strong>
                <span>{entry.reason}</span>
              </li>
            ))}
          </ul>
          <p className="field-help">
            {t('models.poolDisabledHint', {
              defaultValue: '停用是你自己的选择，不是故障：这些端点不会参与模型池探测，也不会被调用。要恢复，在对应卡片上重新打开开关。',
            })}
          </p>
        </section>
      ) : null}
    </Card>
  )
}

function ModelTags({ model }: { model: Model }): ReactNode {
  const { t } = useTranslation()
  const context = model.context_window === null || model.context_window <= 0
    ? t('models.contextUnknown', { defaultValue: '上下文 未知' })
    : `${t('models.contextLabel', { defaultValue: '上下文' })} ${formatContextWindow(model.context_window)}`
  return (
    <span className="models-tags">
      <span className={`models-tag${model.modality === 'text' ? ' models-tag-text' : ''}`}>
        {model.modality === 'text'
          ? t('models.modalityText', { defaultValue: '文本' })
          : t('models.modalityUnknown', { defaultValue: '未知' })}
      </span>
      <span className="models-tag models-tag-muted">{context}</span>
    </span>
  )
}

/** 上下文窗口按 1024 折成 K，到 1M 用 M；服务端给多少就显示多少。 */
export function formatContextWindow(tokens: number): string {
  if (tokens >= 1_048_576) {
    return `${Math.round((tokens / 1_048_576) * 10) / 10}M`
  }
  return `${Math.round(tokens / 1024)}K`
}

function ProbeButton({ providerId }: { providerId: string }): ReactNode {
  const { t } = useTranslation()
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null)
  const probe = useMutation({
    mutationFn: () => listProviderModels(providerId),
    onSuccess: (data) => {
      // 探测失败时服务端仍返 200（把中文原因放在 error 字段），HTTP 成功 ≠ 连上了。
      // 不先判 error 就会把失败显示成「连上了，共 0 个模型」——那是假在线状态。
      if (data.error) {
        // 服务端给的 error 本身就是完整的中文句子（含下一步），直接显示，不要再包一层。
        setResult({ ok: false, text: data.error })
        return
      }
      setResult({
        ok: true,
        text: t('models.probeSucceeded', { count: data.models.length, defaultValue: '连上了，共 {{count}} 个模型。' }),
      })
    },
    onError: (error) => {
      setResult({
        ok: false,
        text: chatErrorMessage(error, t('models.probeFailed', { defaultValue: '探测失败。' })),
      })
    },
  })

  return (
    <span className="models-probe">
      <button
        type="button"
        className="models-probe-button"
        disabled={probe.isPending}
        title={t('models.probeRouteNote', {
          route: `GET ${PROVIDERS_ROUTE}/{id}/models`,
          defaultValue: '测试连接调用 {{route}}。',
        })}
        aria-label={t('models.testConnection', { defaultValue: '测试连接' })}
        onClick={() => probe.mutate()}
      >
        {probe.isPending ? '…' : '⚡'}
      </button>
      {result ? (
        <span className={`models-probe-result${result.ok ? '' : ' is-error'}`} role="status">{result.text}</span>
      ) : null}
    </span>
  )
}

function PresetCard({
  preset,
  provider,
  open,
  isPending,
  deletePending,
  defaultPending,
  onToggle,
  onSave,
  onDelete,
  onSetDefault,
}: {
  preset: PresetProvider
  provider: Provider | undefined
  open: boolean
  isPending: boolean
  deletePending: boolean
  defaultPending: boolean
  onToggle: () => void
  onSave: (payload: ProviderInput) => void
  onDelete: (id: string) => void
  onSetDefault: (id: string) => void
}): ReactNode {
  const { t } = useTranslation()
  const [confirming, setConfirming] = useState(false)
  const status = provider === undefined
    ? { label: t('models.notConfigured', { defaultValue: '未配置' }), tone: 'neutral' as const }
    : provider.has_api_key
      ? { label: t('models.authorized', { defaultValue: '已授权' }), tone: 'success' as const }
      : { label: t('models.awaitingKey', { defaultValue: '待填密钥' }), tone: 'warning' as const }

  return (
    <article className="models-preset-card" data-preset={preset.preset_id}>
      <header className="models-preset-head">
        <span className="models-logo" aria-hidden="true">{preset.display_name.slice(0, 1)}</span>
        <strong>{preset.display_name}</strong>
        <span className={`models-dot is-${status.tone}`} role="img" aria-label={status.label} />
        <span className="models-preset-status">{status.label}</span>
        {provider?.is_default ? (
          <StatusBadge tone="success">{t('models.isDefault', { defaultValue: '默认' })}</StatusBadge>
        ) : null}
      </header>
      <dl className="models-preset-meta">
        <div>
          <dt>{t('models.baseUrlLabel', { defaultValue: 'Base URL:' })}</dt>
          <dd><code>{provider ? provider.base_url : presetDefaultBaseUrl(preset)}</code></dd>
        </div>
        <div>
          <dt>{t('models.modelLabel', { defaultValue: '模型:' })}</dt>
          <dd>
            {provider
              ? provider.model || '—'
              : t('models.pickSiteFirst', { defaultValue: '请先选择站点' })}
          </dd>
        </div>
        {provider ? null : (
          <div>
            <dt>{t('models.sitesLabel', { defaultValue: '站点:' })}</dt>
            <dd>
              {t('models.sitesCount', { count: presetSiteCount(preset), defaultValue: '{{count}} 个站点' })}
            </dd>
          </div>
        )}
      </dl>
      <footer className="models-preset-foot">
        <span className="models-protocol">{provider ? provider.protocol : preset.protocol}</span>
        <button type="button" className="models-configure" aria-expanded={open} onClick={onToggle}>
          {provider
            ? t('models.configured', { defaultValue: '已配置' })
            : t('models.configure', { name: preset.display_name, defaultValue: '配置 {{name}}' })}
        </button>
      </footer>
      {open ? (
        <ProviderForm
          id={`models-form-${preset.preset_id}`}
          initial={provider ? inputOf(provider) : presetDefaults(preset)}
          submitLabel={t('models.saveProvider', { defaultValue: '保存提供商' })}
          hasStoredKey={provider?.has_api_key ?? false}
          isPending={isPending}
          onSubmit={onSave}
          onCancel={() => setConfirming(false)}
        />
      ) : null}
      {open && provider ? (
        <div className="models-preset-admin">
          <button
            type="button"
            className="secondary-button"
            disabled={defaultPending}
            onClick={() => onSetDefault(provider.id)}
          >
            {t('models.setDefault', { defaultValue: '设为默认' })}
          </button>
          {confirming ? (
            <span className="models-delete-confirm">
              <span>{t('models.confirmDelete', { name: provider.name, defaultValue: '确认删除「{{name}}」？' })}</span>
              <button
                type="button"
                className="danger-button"
                disabled={deletePending}
                onClick={() => onDelete(provider.id)}
              >
                {t('models.delete', { defaultValue: '删除' })}
              </button>
              <button type="button" className="secondary-button" onClick={() => setConfirming(false)}>
                {t('models.cancel', { defaultValue: '取消' })}
              </button>
            </span>
          ) : (
            <button type="button" className="danger-link" onClick={() => setConfirming(true)}>
              {t('models.delete', { defaultValue: '删除' })}
            </button>
          )}
        </div>
      ) : null}
    </article>
  )
}

/** 已建好的自定义端点。启动时从环境变量种下来的那个就在这里，所以它可改、可停用、可删。 */
function CustomProviderCard({
  provider,
  open,
  isPending,
  deletePending,
  defaultPending,
  togglePending,
  onToggle,
  onSave,
  onDelete,
  onSetDefault,
  onToggleEnabled,
}: {
  provider: Provider
  open: boolean
  isPending: boolean
  deletePending: boolean
  defaultPending: boolean
  togglePending: boolean
  onToggle: () => void
  onSave: (payload: ProviderInput) => void
  onDelete: (id: string) => void
  onSetDefault: (id: string) => void
  onToggleEnabled: (enabled: boolean) => void
}): ReactNode {
  const { t } = useTranslation()
  const [confirming, setConfirming] = useState(false)
  const status = provider.has_api_key
    ? { label: t('models.authorized', { defaultValue: '已授权' }), tone: 'success' as const }
    : { label: t('models.awaitingKey', { defaultValue: '待填密钥' }), tone: 'warning' as const }

  return (
    <article className="models-preset-card" data-custom-provider={provider.id}>
      <header className="models-preset-head">
        <span className="models-logo" aria-hidden="true">{(provider.name || '?').slice(0, 1).toUpperCase()}</span>
        <strong>{provider.name}</strong>
        <span className={`models-dot is-${provider.enabled ? status.tone : 'neutral'}`} role="img" aria-label={status.label} />
        <span className="models-preset-status">
          {provider.enabled ? status.label : t('models.disabled', { defaultValue: '已停用' })}
        </span>
        {provider.is_default ? (
          <StatusBadge tone="success">{t('models.isDefault', { defaultValue: '默认' })}</StatusBadge>
        ) : null}
      </header>
      <dl className="models-preset-meta">
        <div>
          <dt>{t('models.baseUrlLabel', { defaultValue: 'Base URL:' })}</dt>
          <dd><code>{provider.base_url}</code></dd>
        </div>
        <div>
          <dt>{t('models.modelLabel', { defaultValue: '模型:' })}</dt>
          <dd>{provider.model || '—'}</dd>
        </div>
        <div>
          <dt>{t('models.configuredContextLabel', { defaultValue: '配置上下文:' })}</dt>
          <dd>{formatContextWindow(provider.max_context_tokens)}</dd>
        </div>
      </dl>
      <footer className="models-preset-foot">
        <span className="models-protocol">{provider.protocol}</span>
        <button type="button" className="models-configure" aria-expanded={open} onClick={onToggle}>
          {open
            ? t('models.cancel', { defaultValue: '取消' })
            : t('models.editProvider', { defaultValue: '编辑' })}
        </button>
      </footer>
      {open ? (
        <ProviderForm
          id={`models-form-${provider.id}`}
          initial={inputOf(provider)}
          submitLabel={t('models.saveProvider', { defaultValue: '保存提供商' })}
          hasStoredKey={provider.has_api_key}
          isPending={isPending}
          onSubmit={onSave}
          onCancel={() => setConfirming(false)}
        />
      ) : null}
      <div className="models-preset-admin">
        <button
          type="button"
          className="secondary-button"
          disabled={togglePending}
          data-testid={`models-toggle-${provider.id}`}
          onClick={() => onToggleEnabled(!provider.enabled)}
        >
          {provider.enabled
            ? t('models.disableProvider', { defaultValue: '停用' })
            : t('models.enableProvider', { defaultValue: '启用' })}
        </button>
        <button
          type="button"
          className="secondary-button"
          disabled={defaultPending || provider.is_default}
          onClick={() => onSetDefault(provider.id)}
        >
          {t('models.setDefault', { defaultValue: '设为默认' })}
        </button>
        {confirming ? (
          <span className="models-delete-confirm">
            <span>{t('models.confirmDelete', { name: provider.name, defaultValue: '确认删除「{{name}}」？' })}</span>
            <button
              type="button"
              className="danger-button"
              disabled={deletePending}
              onClick={() => onDelete(provider.id)}
            >
              {t('models.delete', { defaultValue: '删除' })}
            </button>
            <button type="button" className="secondary-button" onClick={() => setConfirming(false)}>
              {t('models.cancel', { defaultValue: '取消' })}
            </button>
          </span>
        ) : (
          <button type="button" className="danger-link" onClick={() => setConfirming(true)}>
            {t('models.delete', { defaultValue: '删除' })}
          </button>
        )}
      </div>
    </article>
  )
}

/**
 * 列表形态：卡片那一整套（展开的编辑表单、状态徽标、操作按钮）照搬，
 * 只是竖着排成一列、不并排放。**编辑表单两种看法都能用** —— 少一种能力
 * 就等于告诉用户「切到列表就不能改了」，而那不是真的。
 */
function PresetRow(props: React.ComponentProps<typeof PresetCard>): ReactNode {
  return <article className="model-row">{<PresetCard {...props} />}</article>
}

function CustomProviderRow(props: React.ComponentProps<typeof CustomProviderCard>): ReactNode {
  return <article className="model-row">{<CustomProviderCard {...props} />}</article>
}

function ProviderForm({
  id,
  initial,
  submitLabel,
  hasStoredKey,
  isPending,
  onSubmit,
  onCancel,
}: {
  id: string
  initial: ProviderInput
  submitLabel: string
  hasStoredKey: boolean
  isPending: boolean
  onSubmit: (payload: ProviderInput) => void
  onCancel?: () => void
}): ReactNode {
  const { t } = useTranslation()
  const [draft, setDraft] = useState<ProviderInput>(initial)
  const [apiKey, setApiKey] = useState('')
  const [localError, setLocalError] = useState<string | null>(null)

  const patch = (next: Partial<ProviderInput>): void => {
    setDraft((current) => ({ ...current, ...next }))
  }

  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault()
    const name = draft.name.trim()
    const baseUrl = draft.base_url.trim()
    if (!name) {
      setLocalError(t('models.nameRequired', { defaultValue: '请填写名称。' }))
      return
    }
    if (!baseUrl) {
      setLocalError(t('models.baseUrlRequired', { defaultValue: '请填写 Base URL。' }))
      return
    }
    if (draft.max_context_tokens <= 0 || draft.compaction_threshold_tokens <= 0 || draft.max_output_tokens <= 0) {
      setLocalError(t('models.numberRequired', { defaultValue: '上下文长度、压缩阈值与最大输出都要是大于 0 的整数。' }))
      return
    }
    setLocalError(null)
    const key = apiKey.trim()
    onSubmit({
      ...draft,
      name,
      base_url: baseUrl,
      // 防「空串等于删密钥」：留空时不带 api_key，服务端沿用已保存的值。
      ...(key ? { api_key: key } : {}),
    })
  }

  return (
    <form className="form-grid models-form" id={id} onSubmit={submit}>
      {localError ? <p className="form-error full-row" role="alert">{localError}</p> : null}
      <label className="full-row">
        {t('models.name', { defaultValue: '名称' })}
        <input
          name="name"
          value={draft.name}
          autoComplete="off"
          disabled={isPending}
          onChange={(event) => patch({ name: event.target.value })}
        />
      </label>
      <label className="full-row">
        {t('models.baseUrl', { defaultValue: 'Base URL' })}
        <input
          name="base_url"
          type="url"
          value={draft.base_url}
          autoComplete="off"
          disabled={isPending}
          onChange={(event) => patch({ base_url: event.target.value })}
        />
      </label>
      <label className="full-row">
        {t('models.apiKey', { defaultValue: 'API 密钥' })}
        <input
          name="api_key"
          type="password"
          value={apiKey}
          autoComplete="off"
          disabled={isPending}
          placeholder={hasStoredKey
            ? t('models.apiKeyStored', { defaultValue: '已保存（不显示）' })
            : t('models.apiKeyEmpty', { defaultValue: '未填写' })}
          onChange={(event) => setApiKey(event.target.value)}
        />
        <small>{t('models.apiKeyHelp', { defaultValue: '留空表示保留服务端已保存的密钥。' })}</small>
      </label>
      <label>
        {t('models.protocol', { defaultValue: '协议' })}
        <select
          name="protocol"
          value={draft.protocol}
          disabled={isPending}
          onChange={(event) => patch({ protocol: event.target.value as ProviderProtocol })}
        >
          {PROTOCOLS.map((value) => (
            <option key={value} value={value}>
              {value === 'openai'
                ? t('models.protocolOpenai', { defaultValue: 'OpenAI Chat Completions' })
                : t('models.protocolAnthropic', { defaultValue: 'Anthropic Messages' })}
            </option>
          ))}
        </select>
      </label>
      <label>
        {t('models.modelLabel', { defaultValue: '模型:' }).replace(':', '')}
        <input
          name="model"
          value={draft.model}
          autoComplete="off"
          spellCheck={false}
          disabled={isPending}
          onChange={(event) => patch({ model: event.target.value })}
        />
      </label>
      <label>
        {t('models.maxContextTokens', { defaultValue: '上下文长度' })}
        <input
          name="max_context_tokens"
          type="number"
          min="1"
          value={draft.max_context_tokens || ''}
          disabled={isPending}
          onChange={(event) => patch({ max_context_tokens: Number(event.target.value) })}
        />
      </label>
      <label>
        {t('models.compactionThreshold', { defaultValue: '压缩阈值' })}
        <input
          name="compaction_threshold_tokens"
          type="number"
          min="1"
          value={draft.compaction_threshold_tokens || ''}
          disabled={isPending}
          onChange={(event) => patch({ compaction_threshold_tokens: Number(event.target.value) })}
        />
        {/* 这个数**存下来了，但没有任何代码读它**（全仓库只出现在迁移、
            增删改查与测试里）。不给标注的话，用户会在这里改一个不生效的
            数字，然后发现对话该超还是超。压缩本体是 M3 的活。 */}
        <small className="field-help" data-testid="models-compaction-not-enforced">
          {t('models.compactionNotEnforced', {
            defaultValue:
              '暂未生效：这个值会被保存并显示在对话页，但目前没有任何压缩逻辑会读它 —— 超过上限不会自动摘要，需要自己新建会话。',
          })}
        </small>
      </label>
      <label>
        {t('models.maxOutputTokens', { defaultValue: '最大输出' })}
        <input
          name="max_output_tokens"
          type="number"
          min="1"
          value={draft.max_output_tokens || ''}
          disabled={isPending}
          onChange={(event) => patch({ max_output_tokens: Number(event.target.value) })}
        />
      </label>
      <div className="form-actions full-row">
        {onCancel ? (
          <button type="button" className="secondary-button" disabled={isPending} onClick={onCancel}>
            {t('models.cancel', { defaultValue: '取消' })}
          </button>
        ) : null}
        <button type="submit" className="primary-button" disabled={isPending}>
          {submitLabel}
        </button>
      </div>
    </form>
  )
}

function presetDefaults(preset: PresetProvider): ProviderInput {
  return {
    name: preset.display_name,
    preset_id: preset.preset_id,
    kind: 'preset',
    protocol: preset.protocol,
    base_url: presetDefaultBaseUrl(preset),
    model: '',
    max_context_tokens: 0,
    compaction_threshold_tokens: 0,
    max_output_tokens: 0,
  }
}

function inputOf(provider: Provider): ProviderInput {
  return {
    name: provider.name,
    preset_id: provider.preset_id,
    kind: provider.kind,
    protocol: provider.protocol,
    base_url: provider.base_url,
    model: provider.model,
    max_context_tokens: provider.max_context_tokens,
    compaction_threshold_tokens: provider.compaction_threshold_tokens,
    max_output_tokens: provider.max_output_tokens,
    enabled: provider.enabled,
  }
}

export interface PoolGroup {
  providerId: string
  name: string
  protocol: ProviderProtocol | ''
  entries: PoolEntry[]
  /**
   * 这个 provider **配置里**写的 `max_context_tokens`，拿不到就是 `null`。
   *
   * 留着它是为了能和 `entry.model.context_window`（**探测**到的真实窗口）
   * 对账。两者不一致时必须在界面上说清楚 —— 本机实测就撞上过：
   * 配置写 32768、实际只吞 8192，于是请求被上游 `exceed_context_size_error`
   * 拒掉，而用户完全看不出这两个数打架。见 ISSUE-023。
   */
  configuredContext: number | null
}

/**
 * 这组里**有没有**探测值小于配置值的情况。
 *
 * 刻意只报「配置值 > 探测值」这一种：反过来的（配置得比实际小）不会导致请求
 * 被拒，只是白占窗口，报出来是噪音。而拿不到探测值（`null`）也**不算**不匹配 ——
 * 那是「不知道」，不是「对」，不能拿它当成一致。
 *
 * 导出是为了能单测这三条判断（`ModelsPage.test.tsx`）——
 * 这个函数决定用户会不会看到那条警告，判错了就是漏报或者误报。
 */
export function contextMismatch(group: PoolGroup): number | null {
  const configured = group.configuredContext
  if (configured === null || configured <= 0) return null
  for (const entry of group.entries) {
    const probed = entry.model.context_window
    if (probed !== null && probed > 0 && probed < configured) return probed
  }
  return null
}

function groupPool(entries: PoolEntry[], providers: Provider[]): PoolGroup[] {
  const groups: PoolGroup[] = []
  for (const entry of entries) {
    const provider = providers.find((p) => p.id === entry.provider_id)
    const found = groups.find((group) => group.providerId === entry.provider_id)
    if (found) found.entries.push(entry)
    else {
      groups.push({
        providerId: entry.provider_id,
        name: entry.provider_name,
        protocol: provider?.protocol ?? '',
        entries: [entry],
        configuredContext: provider?.max_context_tokens ?? null,
      })
    }
  }
  return groups
}
