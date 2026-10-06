import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useSearchParams } from 'react-router-dom'

import { chatErrorMessage } from '../chat/chatApi'
import { expertInitial, expertTone } from '../chat/expertAvatar'
import { Card, PageHeader, StatusBadge } from '../components/Page'
import {
  EXPERTS_KEY,
  EXPERTS_ROUTE,
  type Expert,
  type ExpertDraft,
  createExpert,
  deleteExpert,
  draftOf,
  emptyDraft,
  listExperts,
  updateExpert,
} from './api'
import { ExpertForm } from './ExpertForm'
import { ExpertTable } from './ExpertTable'
import { Popconfirm, ToggleSwitch, copyText } from './ExpertsUi'
import {
  IconCheck,
  IconClose,
  IconCopy,
  IconLayoutGrid,
  IconList,
  IconPencil,
  IconPlus,
  IconRefresh,
  IconSearch,
  IconTrash,
} from './icons'
import { LibraryTab } from './LibraryTab'
import { MarketTab } from './MarketTab'
import { TeamsTab } from './TeamsTab'
import './experts.css'

const TABS = ['mine', 'team', 'library', 'market'] as const
type ExpertsTab = typeof TABS[number]

type ViewMode = 'card' | 'table'

/** 视图记忆沿用 Octop 的 key（Experts/index.tsx:70），切换时写回 localStorage。 */
const VIEW_STORAGE_KEY = 'octop:experts-view'

function loadViewMode(): ViewMode {
  return localStorage.getItem(VIEW_STORAGE_KEY) === 'table' ? 'table' : 'card'
}

/** Octop 的 useCardTableView 只是 useState + showCardView，这里去掉它用不上的 isMobile。 */
function useCardTableView(defaultMode: ViewMode): {
  viewMode: ViewMode
  setViewMode: (mode: ViewMode) => void
  showCardView: boolean
} {
  const [viewMode, setViewMode] = useState(defaultMode)
  return { viewMode, setViewMode, showCardView: viewMode === 'card' }
}

function textMatchesQuery(query: string, ...parts: Array<string | null | undefined>): boolean {
  if (!query) return true
  return parts.some((part) => (part ?? '').toLowerCase().includes(query))
}

/** ErrorNotice 只认 Error 对象，传字符串会退化成通用文案、把服务端原文吃掉；这里直接渲染成句。 */
function ErrorText({ text }: { text: string | null }): ReactNode {
  if (!text) return null
  return <p className="form-error" role="alert">{text}</p>
}

export function ExpertsPage(): ReactNode {
  const { t } = useTranslation()
  const [params, setParams] = useSearchParams()
  const raw = params.get('tab')
  const tab: ExpertsTab = TABS.includes(raw as ExpertsTab) ? (raw as ExpertsTab) : 'mine'

  const switchTab = (next: ExpertsTab): void => {
    setParams(next === 'mine' ? {} : { tab: next })
  }

  return (
    <div className="page-scroll experts-page">
      <PageHeader
        eyebrow={t('experts.eyebrow', { defaultValue: '智能体人格' })}
        title={t('experts.title', { defaultValue: '专家' })}
        description={t('experts.description', {
          defaultValue: '一个专家就是一份带 frontmatter 的 markdown：名称、描述、模型，正文即它的人格提示。',
        })}
        actions={(
          <div className="experts-tabs" role="tablist">
            {TABS.map((value) => (
              <button
                key={value}
                type="button"
                role="tab"
                className="experts-tab"
                aria-selected={tab === value}
                onClick={() => switchTab(value)}
              >
                {value === 'mine'
                  ? t('experts.tabMine', { defaultValue: '我的专家' })
                  : value === 'team'
                    ? t('experts.tabTeam', { defaultValue: '我的团队' })
                    : value === 'library'
                      ? t('experts.tabLibrary', { defaultValue: '专家库' })
                      : t('experts.tabMarket', { defaultValue: '市场' })}
              </button>
            ))}
          </div>
        )}
      />

      {tab === 'mine' ? <MyExperts onGoToLibrary={() => switchTab('library')} /> : null}
      {tab === 'team' ? <TeamsTab onGoToLibrary={() => switchTab('library')} /> : null}
      {tab === 'library' ? <LibraryTab /> : null}
      {tab === 'market' ? <MarketTab /> : null}
    </div>
  )
}



function MyExperts({ onGoToLibrary }: { onGoToLibrary: () => void }): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()
  const experts = useQuery({ queryKey: EXPERTS_KEY, queryFn: listExperts, retry: false })
  const { viewMode, setViewMode, showCardView } = useCardTableView(loadViewMode())
  const [searchQuery, setSearchQuery] = useState('')
  const [refreshing, setRefreshing] = useState(false)
  const [creating, setCreating] = useState(false)
  const [editingId, setEditingId] = useState<string | null>(null)

  const invalidate = async (): Promise<void> => {
    await client.invalidateQueries({ queryKey: EXPERTS_KEY })
  }

  const save = useMutation({
    mutationFn: (input: { id: string | null; draft: ExpertDraft }) => (input.id
      ? updateExpert(input.id, {
        display_name: input.draft.display_name,
        description: input.draft.description,
        instructions: input.draft.instructions,
        model: input.draft.model.trim() ? input.draft.model.trim() : null,
        default_enabled: input.draft.default_enabled,
        source_template: input.draft.source_template,
      })
      : createExpert({
        id: input.draft.id,
        display_name: input.draft.display_name,
        description: input.draft.description,
        instructions: input.draft.instructions,
        model: input.draft.model.trim() ? input.draft.model.trim() : null,
        source_template: input.draft.source_template,
      })),
    onSuccess: async () => {
      setCreating(false)
      setEditingId(null)
      await invalidate()
    },
  })

  const remove = useMutation({
    mutationFn: (id: string) => deleteExpert(id),
    onSuccess: async () => {
      setEditingId(null)
      await invalidate()
    },
  })

  const toggleEnabled = useMutation({
    mutationFn: (input: { id: string; enabled: boolean }) => updateExpert(input.id, { default_enabled: input.enabled }),
    onSuccess: invalidate,
  })

  const list = experts.data ?? []
  const mutationError = save.error ?? remove.error ?? toggleEnabled.error
  const needle = searchQuery.trim().toLowerCase()
  const visible = list.filter((expert) => textMatchesQuery(
    needle,
    expert.display_name,
    expert.id,
    expert.description,
    expert.source_template,
  ))

  const onViewChange = (mode: ViewMode): void => {
    setViewMode(mode)
    localStorage.setItem(VIEW_STORAGE_KEY, mode)
  }

  const handleRefresh = async (): Promise<void> => {
    setRefreshing(true)
    try {
      await experts.refetch()
    } finally {
      setRefreshing(false)
    }
  }

  return (
    <div className="settings-stack">
      <Card
        title={t('experts.listTitle', { defaultValue: '我的专家' })}
        description={t('experts.listDescription', {
          defaultValue: '数据来自 {{route}}：这些是你名下的专家。',
          route: EXPERTS_ROUTE,
        })}
      >
        <ErrorText
          text={experts.error
            ? chatErrorMessage(experts.error, t('experts.loadFailed', { defaultValue: '专家列表加载失败。' }))
            : mutationError
              ? chatErrorMessage(mutationError, t('experts.mutateFailed', { defaultValue: '保存失败。' }))
              : null}
        />
        {experts.isPending ? (
          <p className="empty-card-copy">{t('common.loading', { defaultValue: '加载中…' })}</p>
        ) : null}

        {experts.data && list.length === 0 ? (
          <div className="experts-empty">
            <p className="empty-card-copy">
              {t('experts.empty', { defaultValue: '还没有专家。新建一个，它在服务端就是一份 goose agent 文件。' })}
            </p>
            <div className="experts-empty-actions">
              <button
                type="button"
                className="experts-toolbar-btn-primary"
                data-testid="experts-new"
                onClick={() => setCreating(true)}
              >
                <IconPlus />
                {t('experts.newExpert', { defaultValue: '新建专家' })}
              </button>
              <button type="button" className="experts-toolbar-btn" onClick={onGoToLibrary}>
                {t('experts.goToLibrary', { defaultValue: '去内置专家' })}
              </button>
            </div>
          </div>
        ) : null}

        {list.length > 0 ? (
          <>
            <div className="experts-grid-toolbar">
              <span className="experts-grid-count">
                {t('experts.totalAgents', { count: visible.length, defaultValue: '共 {{count}} 个专家' })}
              </span>
              <div className="experts-grid-toolbar-right">
                <span className="experts-toolbar-search">
                  <span className="experts-toolbar-search-icon"><IconSearch /></span>
                  <input
                    type="search"
                    value={searchQuery}
                    aria-label={t('experts.searchPlaceholder', { defaultValue: '搜索专家' })}
                    placeholder={t('experts.searchPlaceholder', { defaultValue: '搜索专家' })}
                    onChange={(event) => setSearchQuery(event.target.value)}
                  />
                  {searchQuery ? (
                    <button
                      type="button"
                      className="experts-toolbar-search-clear"
                      aria-label={t('common.clear', { defaultValue: '清除' })}
                      onClick={() => setSearchQuery('')}
                    >
                      <IconClose />
                    </button>
                  ) : null}
                </span>
                <span className="experts-view-mode" role="group">
                  <button
                    type="button"
                    aria-pressed={viewMode === 'card'}
                    className="experts-view-mode-item"
                    onClick={() => onViewChange('card')}
                  >
                    <span className="experts-view-mode-label">
                      <IconLayoutGrid />
                      {t('experts.viewCard', { defaultValue: '卡片' })}
                    </span>
                  </button>
                  <button
                    type="button"
                    aria-pressed={viewMode === 'table'}
                    className="experts-view-mode-item"
                    onClick={() => onViewChange('table')}
                  >
                    <span className="experts-view-mode-label">
                      <IconList />
                      {t('experts.viewTable', { defaultValue: '表格' })}
                    </span>
                  </button>
                </span>
                <button
                  type="button"
                  className="experts-toolbar-icon-btn"
                  data-testid="experts-refresh"
                  title={t('experts.refresh', { defaultValue: '刷新' })}
                  aria-label={t('experts.refresh', { defaultValue: '刷新' })}
                  disabled={refreshing}
                  onClick={() => void handleRefresh()}
                >
                  <span className={refreshing ? 'experts-spinning' : undefined}><IconRefresh /></span>
                </button>
                <button
                  type="button"
                  className="experts-toolbar-btn-primary"
                  data-testid="experts-new"
                  onClick={() => setCreating(true)}
                >
                  <IconPlus />
                  {t('experts.newExpert', { defaultValue: '新建专家' })}
                </button>
                <button type="button" className="experts-toolbar-btn" onClick={onGoToLibrary}>
                  {t('experts.goToLibrary', { defaultValue: '去内置专家' })}
                </button>
              </div>
            </div>

            {visible.length === 0 ? (
              <div className="experts-search-empty">{t('experts.searchEmpty', { defaultValue: '没有匹配的专家' })}</div>
            ) : showCardView ? (
              <div className="experts-grid">
                {visible.map((expert) => (
                  <ExpertCard
                    key={expert.id}
                    expert={expert}
                    open={editingId === expert.id}
                    isPending={save.isPending}
                    deletePending={remove.isPending}
                    togglePending={toggleEnabled.isPending}
                    onEdit={() => setEditingId((current) => (current === expert.id ? null : expert.id))}
                    onSave={(draft) => save.mutate({ id: expert.id, draft })}
                    onDelete={() => remove.mutate(expert.id)}
                    onToggleEnabled={(enabled) => toggleEnabled.mutate({ id: expert.id, enabled })}
                  />
                ))}
              </div>
            ) : (
              <ExpertTable
                experts={visible}
                editingId={editingId}
                isPending={save.isPending}
                deletePending={remove.isPending}
                togglePending={toggleEnabled.isPending}
                onEdit={(id) => setEditingId((current) => (current === id ? null : id))}
                onDelete={(id) => remove.mutate(id)}
                onToggleEnabled={(id, enabled) => toggleEnabled.mutate({ id, enabled })}
              />
            )}

            {editingId && !showCardView ? (
              <TableEditor
                expert={list.find((item) => item.id === editingId) ?? null}
                isPending={save.isPending}
                onSave={(draft) => save.mutate({ id: editingId, draft })}
                onCancel={() => setEditingId(null)}
              />
            ) : null}
          </>
        ) : null}

        {creating ? (
          <ExpertForm
            id="experts-create-form"
            initial={emptyDraft()}
            isPending={save.isPending}
            onSubmit={(draft) => save.mutate({ id: null, draft })}
            onCancel={() => setCreating(false)}
          />
        ) : null}
      </Card>
    </div>
  )
}

/** 表格视图下编辑：Octop 在表格里点编辑会开 Drawer，quill 沿用现有内联表单，挂在表格下方。 */
function TableEditor({
  expert,
  isPending,
  onSave,
  onCancel,
}: {
  expert: Expert | null
  isPending: boolean
  onSave: (draft: ExpertDraft) => void
  onCancel: () => void
}): ReactNode {
  if (!expert) return null
  return (
    <div className="experts-table-editor">
      <ExpertForm
        id={`experts-form-${expert.id}`}
        initial={draftOf(expert)}
        isPending={isPending}
        onSubmit={onSave}
        onCancel={onCancel}
      />
    </div>
  )
}

function ExpertCard({
  expert,
  open,
  isPending,
  deletePending,
  togglePending,
  onEdit,
  onSave,
  onDelete,
  onToggleEnabled,
}: {
  expert: Expert
  open: boolean
  isPending: boolean
  deletePending: boolean
  togglePending: boolean
  onEdit: () => void
  onSave: (draft: ExpertDraft) => void
  onDelete: () => void
  onToggleEnabled: (enabled: boolean) => void
}): ReactNode {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  const name = expert.display_name || expert.id
  const hasInstructions = (expert.instructions ?? '').trim().length > 0
  const copyHint = t('experts.copyAgentId', { defaultValue: '点击复制专家 ID' })

  return (
    <article className="experts-card" data-expert={expert.id}>
      <header className="experts-card-head">
        <div className="experts-card-title">
          <span className="experts-card-avatar" data-tone={expertTone(expert.id)}>{expertInitial(expert.id)}</span>
          <div className="experts-card-title-block">
            <div className="experts-card-name">
              {name}
              {expert.is_builtin ? <StatusBadge tone="neutral">{t('experts.builtin', { defaultValue: '内置' })}</StatusBadge> : null}
              {expert.source_template ? (
                <StatusBadge tone="neutral">{t('experts.fromTemplate', { defaultValue: '来自模板' })}</StatusBadge>
              ) : null}
            </div>
            <button
              type="button"
              className="experts-copy-id-btn"
              title={copyHint}
              aria-label={copyHint}
              onClick={() => {
                void copyText(expert.id).then((ok) => {
                  if (ok) setCopied(true)
                })
              }}
            >
              <span className="experts-copy-id-value">{expert.id}</span>
              {copied
                ? <span className="experts-copied-tip" role="status">{t('experts.copied', { defaultValue: '已复制' })}</span>
                : null}
            </button>
          </div>
        </div>
        <div className="experts-card-head-actions">
          <ToggleSwitch
            checked={expert.default_enabled}
            pending={togglePending}
            label={t('experts.toggleDefault', { defaultValue: '设为默认启用' })}
            onChange={onToggleEnabled}
          />
        </div>
      </header>
      <p className="experts-card-desc">
        {expert.description || t('experts.noDescription', { defaultValue: '（没写描述）' })}
      </p>
      <p className={hasInstructions ? 'experts-persona' : 'experts-persona is-missing'}>
        {hasInstructions
          ? t('experts.hasInstructions', { defaultValue: '已设置人格' })
          : t('experts.noInstructions', { defaultValue: '未设置人格' })}
      </p>
      <p className="experts-card-model">
        <span className="experts-card-label">{t('experts.modelLabel', { defaultValue: '模型' })}</span>
        {expert.model
          ? `${expert.model} ${t('experts.modelPending', { defaultValue: '（已保存，暂未参与路由）' })}`
          : t('experts.modelFollowDefault', { defaultValue: '跟随实例默认模型' })}
      </p>
      <footer className="experts-card-foot">
        <button
          type="button"
          className="experts-card-icon-btn"
          aria-label={t('common.edit', { defaultValue: '编辑' })}
          aria-expanded={open}
          onClick={onEdit}
        >
          <IconPencil />
        </button>
        <button
          type="button"
          className="experts-card-icon-btn"
          aria-label={copyHint}
          onClick={() => {
            void copyText(expert.id).then((ok) => {
              if (ok) setCopied(true)
            })
          }}
        >
          {copied ? <IconCheck size={13} /> : <IconCopy size={13} />}
        </button>
        <Popconfirm
          className="experts-card-icon-btn is-danger"
          label={t('common.delete', { defaultValue: '删除' })}
          icon={<IconTrash />}
          title={t('experts.confirmDelete', { name, defaultValue: '删除「{{name}}」？' })}
          description={t('experts.confirmDeleteHint', {
            defaultValue: '这是软删除：删除后该专家对所有用户（含属主）不可见，同名 id 可以再次创建。',
          })}
          okText={t('common.delete', { defaultValue: '删除' })}
          cancelText={t('common.cancel', { defaultValue: '取消' })}
          dangerOk
          pending={deletePending}
          onConfirm={onDelete}
        />
      </footer>
      {open ? (
        <ExpertForm
          id={`experts-form-${expert.id}`}
          initial={draftOf(expert)}
          isPending={isPending}
          onSubmit={onSave}
          onCancel={onEdit}
        />
      ) : null}
    </article>
  )
}
