import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type FormEvent, type ReactNode, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { chatErrorMessage } from '../chat/chatApi'
import { expertInitial, expertTone } from '../chat/expertAvatar'
import { Card, StatusBadge } from '../components/Page'
import {
  EXPERTS_KEY,
  TEAMS_KEY,
  TEAMS_ROUTE,
  TEAM_MEMBERS_MAX,
  TEAM_MEMBERS_MIN,
  type Expert,
  type Team,
  type TeamDraft,
  createTeam,
  deleteTeam,
  emptyTeamDraft,
  isValidExpertId,
  listExperts,
  listTeams,
  teamDraftOf,
  updateTeam,
} from './api'
import { Popconfirm } from './ExpertsUi'
import { IconCheck, IconClose, IconPencil, IconPlus, IconRefresh, IconSearch, IconTrash, IconUsers } from './icons'

/** Octop TeamCard.tsx:56 折叠成员头像时最多露出 6 个，多出来的并成 +N。 */
const MAX_VISIBLE_MEMBERS = 6

export function TeamsTab({ onGoToLibrary }: { onGoToLibrary: () => void }): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()
  const teams = useQuery({ queryKey: TEAMS_KEY, queryFn: listTeams, retry: false })
  const experts = useQuery({ queryKey: EXPERTS_KEY, queryFn: listExperts, retry: false })
  const [searchQuery, setSearchQuery] = useState('')
  const [refreshing, setRefreshing] = useState(false)
  const [creating, setCreating] = useState(false)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  const invalidate = async (): Promise<void> => {
    await client.invalidateQueries({ queryKey: TEAMS_KEY })
  }

  const save = useMutation({
    mutationFn: (input: { id: string | null; draft: TeamDraft }) => {
      const description = input.draft.description.trim()
      const body = {
        name: input.draft.name.trim(),
        description: description === '' ? null : description,
        leader_id: input.draft.leader_id,
        member_ids: input.draft.member_ids,
      }
      return input.id ? updateTeam(input.id, body) : createTeam({ team_id: input.draft.team_id, ...body })
    },
    onSuccess: async (_saved, input) => {
      setCreating(false)
      setEditingId(null)
      setNotice(input.id
        ? t('experts.teams.updated', { defaultValue: '团队已更新' })
        : t('experts.teams.created', { defaultValue: '团队已创建' }))
      await invalidate()
    },
  })

  const remove = useMutation({
    mutationFn: (id: string) => deleteTeam(id),
    onSuccess: async () => {
      setEditingId(null)
      await invalidate()
    },
  })

  const list = teams.data ?? []
  const expertList = experts.data ?? []
  const nameById = useMemo(() => new Map(expertList.map((item) => [item.id, item])), [expertList])
  const mutationError = save.error ?? remove.error
  const needle = searchQuery.trim().toLowerCase()

  const visible = list.filter((team) => {
    if (!needle) return true
    const memberNames = team.member_ids
      .map((id) => nameById.get(id)?.display_name ?? '')
      .join(' ')
    return [team.name, team.team_id, team.description ?? '', memberNames]
      .some((part) => part.toLowerCase().includes(needle))
  })

  const handleRefresh = async (): Promise<void> => {
    setRefreshing(true)
    try {
      await teams.refetch()
    } finally {
      setRefreshing(false)
    }
  }

  return (
    <div className="settings-stack">
      <Card
        title={t('experts.teamTitle', { defaultValue: '我的团队' })}
        description={t('experts.teams.listDescription', {
          defaultValue: '数据来自 {{route}}：一个团队由一名主持人和 2~8 名成员组成。quill 目前只把这份名单记下来，不会真的派工。',
          route: TEAMS_ROUTE,
        })}
      >
        {teams.error ? (
          <p className="form-error" role="alert">
            {chatErrorMessage(teams.error, t('experts.teams.loadFailed', { defaultValue: '团队列表加载失败。' }))}
          </p>
        ) : null}
        {mutationError ? (
          <p className="form-error" role="alert">
            {chatErrorMessage(mutationError, t('experts.teams.mutateFailed', { defaultValue: '团队保存失败。' }))}
          </p>
        ) : null}
        {teams.isPending ? (
          <p className="empty-card-copy">{t('common.loading', { defaultValue: '加载中…' })}</p>
        ) : null}
        {notice ? <p className="experts-lib-created" role="status">{notice}</p> : null}

        {teams.data && list.length === 0 ? (
          <TeamEmptyGuide
            canCreate={expertList.length >= TEAM_MEMBERS_MIN + 1}
            membersMinText={t('experts.teams.membersMin', { defaultValue: '至少选择两名专家。' })}
            expertsLoading={experts.isPending}
            onCreate={() => setCreating(true)}
            onGoToLibrary={onGoToLibrary}
          />
        ) : null}

        {list.length > 0 ? (
          <>
            <div className="experts-grid-toolbar">
              <span className="experts-grid-count">
                {t('experts.teams.total', { count: visible.length, defaultValue: '共 {{count}} 个团队' })}
              </span>
              <div className="experts-grid-toolbar-right">
                <span className="experts-toolbar-search">
                  <span className="experts-toolbar-search-icon"><IconSearch /></span>
                  <input
                    type="search"
                    value={searchQuery}
                    aria-label={t('experts.teams.searchPlaceholder', { defaultValue: '搜索团队' })}
                    placeholder={t('experts.teams.searchPlaceholder', { defaultValue: '搜索团队' })}
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
                <button
                  type="button"
                  className="experts-toolbar-icon-btn"
                  data-testid="teams-refresh"
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
                  data-testid="teams-new"
                  onClick={() => setCreating(true)}
                >
                  <IconPlus />
                  {t('experts.teams.create', { defaultValue: '创建团队' })}
                </button>
              </div>
            </div>

            {visible.length === 0 ? (
              <div className="experts-search-empty">{t('experts.teams.searchEmpty', { defaultValue: '没有匹配的团队' })}</div>
            ) : (
              <div className="experts-grid">
                {visible.map((team) => (
                  <TeamCard
                    key={team.team_id}
                    team={team}
                    experts={expertList}
                    open={editingId === team.team_id}
                    isPending={save.isPending}
                    deletePending={remove.isPending}
                    onEdit={() => setEditingId((current) => (current === team.team_id ? null : team.team_id))}
                    onSave={(draft) => save.mutate({ id: team.team_id, draft })}
                    onDelete={() => remove.mutate(team.team_id)}
                  />
                ))}
              </div>
            )}
          </>
        ) : null}

        {/* 创建面板挂在两种状态之外，照 MyExperts（ExpertsPage.tsx:401）：空态点「创建团队」也要能开。 */}
        {creating ? (
          <TeamForm
            id="teams-create-form"
            initial={emptyTeamDraft()}
            experts={expertList}
            isPending={save.isPending}
            onSubmit={(draft) => save.mutate({ id: null, draft })}
            onCancel={() => setCreating(false)}
          />
        ) : null}
      </Card>
    </div>
  )
}

/** 空态照 Octop index.tsx:572-606 的 StreamSetupGuide 骨架（标题 + 三步 + 主次按钮）。 */
function TeamEmptyGuide({
  canCreate,
  membersMinText,
  expertsLoading,
  onCreate,
  onGoToLibrary,
}: {
  canCreate: boolean
  membersMinText: string
  expertsLoading: boolean
  onCreate: () => void
  onGoToLibrary: () => void
}): ReactNode {
  const { t } = useTranslation()
  const steps = [
    { label: t('experts.teams.emptyGuideStepWhat', { defaultValue: '什么是专家团队？' }), detail: t('experts.teams.emptyGuideStepWhatDetail', { defaultValue: '团队有且只有一名主持人，另外有 2~8 名成员。主持人不计入成员人数。' }) },
    { label: t('experts.teams.emptyGuideStepHow', { defaultValue: '怎么创建' }), detail: t('experts.teams.emptyGuideStepHowDetail', { defaultValue: '先在「我的专家」里准备至少两名专家，再回到这里选一名当主持人、勾选其余成员。创建后可以随时改。' }) },
    { label: t('experts.teams.emptyGuideStepChat', { defaultValue: '现在能拿它做什么' }), detail: t('experts.teams.emptyGuideStepChatDetail', { defaultValue: '目前只有编队和记账：quill 不会因为建了团队就自动派工，团队也没有专属房间。要找成员专家，去对话页单独选它提问。' }) },
  ]

  return (
    <div className="experts-empty">
      <div className="experts-team-guide">
        <h3 className="experts-team-guide-title">
          <IconUsers size={16} />
          {t('experts.teams.emptyGuideTitle', { defaultValue: '还没有团队' })}
        </h3>
        <p className="experts-team-guide-desc">
          {t('experts.teams.emptyGuideDesc', {
            defaultValue: '一个团队由一名主持专家和至少两名成员组成。目前 quill 只负责把这份名单记下来，还不会真的派工。',
          })}
        </p>
        <ol className="experts-team-guide-steps">
          {steps.map((step) => (
            <li key={step.label}>
              <strong>{step.label}</strong>
              <span>{step.detail}</span>
            </li>
          ))}
        </ol>
        {expertsLoading ? (
          <p className="empty-card-copy">{t('common.loading', { defaultValue: '加载中…' })}</p>
        ) : null}
      </div>
      <div className="experts-empty-actions">
        <button
          type="button"
          className="experts-toolbar-btn-primary"
          data-testid="teams-new"
          disabled={!canCreate}
          title={canCreate ? undefined : membersMinText}
          onClick={onCreate}
        >
          <IconPlus />
          {t('experts.teams.create', { defaultValue: '创建团队' })}
        </button>
        <button type="button" className="experts-toolbar-btn" onClick={onGoToLibrary}>
          {t('experts.goToLibrary', { defaultValue: '去内置专家' })}
        </button>
      </div>
    </div>
  )
}

function TeamCard({
  team,
  experts,
  open,
  isPending,
  deletePending,
  onEdit,
  onSave,
  onDelete,
}: {
  team: Team
  experts: Expert[]
  open: boolean
  isPending: boolean
  deletePending: boolean
  onEdit: () => void
  onSave: (draft: TeamDraft) => void
  onDelete: () => void
}): ReactNode {
  const { t } = useTranslation()
  const leader = experts.find((item) => item.id === team.leader_id) ?? null
  // 契约说 member_ids 里找不到的专家要跳过（Octop TeamCard.tsx:80-82 的 resolveMembers），
  // 但跳过不等于没这回事：数量仍按 member_ids.length 显示，缺失单列一行。
  const members = team.member_ids
    .map((id) => experts.find((item) => item.id === id))
    .filter((item): item is Expert => Boolean(item))
  const visibleMembers = members.slice(0, MAX_VISIBLE_MEMBERS)
  const hiddenCount = Math.max(0, members.length - visibleMembers.length)
  const missingCount = Math.max(0, team.member_ids.length - members.length)

  return (
    <article className="experts-card" data-team={team.team_id}>
      <header className="experts-card-head">
        <div className="experts-card-title">
          <span className="experts-card-avatar" data-tone="accent"><IconUsers size={16} /></span>
          <div className="experts-card-title-block">
            <div className="experts-card-name">
              {team.name}
              <StatusBadge tone="warning">
                {t('experts.teams.notExecuted', { defaultValue: '只记账，未派工' })}
              </StatusBadge>
            </div>
            <p className="experts-card-id"><code>{team.team_id}</code></p>
          </div>
        </div>
      </header>
      <p className="experts-card-desc">
        {team.description || t('experts.noDescription', { defaultValue: '（没写描述）' })}
      </p>

      <div className="experts-team-roster">
        <span className="experts-team-roster-label">
          <StatusBadge tone="success">{t('experts.teams.leader', { defaultValue: '主持人' })}</StatusBadge>
        </span>
        {leader ? (
          <span className="experts-team-chip" title={leader.display_name || leader.id}>
            <span className="experts-card-avatar" data-tone={expertTone(leader.id)}>{expertInitial(leader.id)}</span>
            {leader.display_name || leader.id}
          </span>
        ) : (
          <span className="experts-team-missing">
            {t('experts.teams.leaderUnknown', { id: team.leader_id, defaultValue: '主持人「{{id}}」不在专家列表里' })}
          </span>
        )}
      </div>

      <div className="experts-team-roster">
        <span className="experts-team-roster-label">
          {t('experts.teams.memberAvatars', { defaultValue: '成员' })}
          <span className="experts-team-count">
            {t('experts.teams.memberCount', {
              count: team.member_ids.length,
              defaultValue: '{{count}} 名（限 {{min}}~{{max}}）',
              min: TEAM_MEMBERS_MIN,
              max: TEAM_MEMBERS_MAX,
            })}
          </span>
        </span>
        {visibleMembers.map((member) => (
          <span className="experts-team-chip" key={member.id} title={member.display_name || member.id}>
            <span className="experts-card-avatar" data-tone={expertTone(member.id)}>{expertInitial(member.id)}</span>
            {member.display_name || member.id}
          </span>
        ))}
        {hiddenCount > 0 ? (
          <span className="experts-team-more">
            {t('experts.teams.memberMore', { count: hiddenCount, defaultValue: '+{{count}}' })}
          </span>
        ) : null}
      </div>
      {missingCount > 0 ? (
        <p className="experts-team-missing">
          {t('experts.teams.membersMissing', {
            count: missingCount,
            defaultValue: '有 {{count}} 名成员已不在专家列表里，这里只显示了还找得到的那几个。',
          })}
        </p>
      ) : null}

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
        <Popconfirm
          className="experts-card-icon-btn is-danger"
          label={t('common.delete', { defaultValue: '删除' })}
          icon={<IconTrash />}
          title={t('experts.teams.confirmDelete', { name: team.name, defaultValue: '删除「{{name}}」？' })}
          description={t('experts.teams.confirmDeleteHint', {
            defaultValue: '只删这份团队名单，成员专家本身不受影响；同一个标识之后可以再建。',
          })}
          okText={t('common.delete', { defaultValue: '删除' })}
          cancelText={t('common.cancel', { defaultValue: '取消' })}
          dangerOk
          pending={deletePending}
          onConfirm={onDelete}
        />
      </footer>
      {open ? (
        <TeamForm
          id={`teams-form-${team.team_id}`}
          initial={teamDraftOf(team)}
          experts={experts}
          isPending={isPending}
          onSubmit={onSave}
          onCancel={onEdit}
        />
      ) : null}
    </article>
  )
}

/** 内联展开面板：quill 没有 Drawer 基建，形态照 ExpertForm / LibraryTab 生成面板。 */
function TeamForm({
  id,
  initial,
  experts,
  isPending,
  onSubmit,
  onCancel,
}: {
  id: string
  initial: TeamDraft
  experts: Expert[]
  isPending: boolean
  onSubmit: (draft: TeamDraft) => void
  onCancel: () => void
}): ReactNode {
  const { t } = useTranslation()
  const editingExisting = initial.team_id !== ''
  const [draft, setDraft] = useState<TeamDraft>(initial)
  const [localError, setLocalError] = useState<string | null>(null)

  const patch = (next: Partial<TeamDraft>): void => {
    setDraft((current) => ({ ...current, ...next }))
  }

  const idInvalid = draft.team_id !== '' && !isValidExpertId(draft.team_id)
  const members = rosterIds(draft.member_ids, experts, draft.leader_id)
  const belowMin = members.length < TEAM_MEMBERS_MIN

  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault()
    const name = draft.name.trim()
    if (!name) {
      setLocalError(t('experts.teams.nameRequired', { defaultValue: '请填写团队名称。' }))
      return
    }
    if (!editingExisting) {
      if (!draft.team_id) {
        setLocalError(t('experts.teams.idRequired', { defaultValue: '请填写团队标识。' }))
        return
      }
      if (!isValidExpertId(draft.team_id)) {
        setLocalError(t('experts.teams.idInvalid', {
          defaultValue: '标识不合法：只能是小写字母、数字和连字符，长度 1~64。',
        }))
        return
      }
    }
    if (!draft.leader_id) {
      setLocalError(t('experts.teams.leaderRequired', { defaultValue: '请选择一名主持人。' }))
      return
    }
    if (belowMin) {
      setLocalError(t('experts.teams.membersMin', { defaultValue: '至少选择两名专家。' }))
      return
    }
    setLocalError(null)
    // 契约要求 member_ids 升序。选择器按勾选顺序收集，直接发出去就是勾选序；
    // 这里显式排一次，免得把有序性寄托在后端读时的 ORDER BY 上。
    onSubmit({ ...draft, name, member_ids: [...members].sort() })
  }

  return (
    <form className="form-grid experts-form experts-team-form" id={id} onSubmit={submit}>
      {localError ? <p className="form-error full-row" role="alert">{localError}</p> : null}
      <label className="full-row">
        {t('experts.teams.nameLabel', { defaultValue: '团队名称' })}
        <input
          name="team_name"
          value={draft.name}
          autoComplete="off"
          disabled={isPending}
          onChange={(event) => patch({ name: event.target.value })}
        />
      </label>
      <label className="full-row">
        {t('experts.teams.idLabel', { defaultValue: '团队标识（id）' })}
        <input
          name="team_id"
          value={draft.team_id}
          autoComplete="off"
          spellCheck={false}
          disabled={isPending || editingExisting}
          aria-invalid={idInvalid}
          onChange={(event) => patch({ team_id: event.target.value })}
        />
        <small className={idInvalid ? 'experts-id-help is-error' : 'experts-id-help'}>
          {editingExisting
            ? t('experts.teams.idLocked', { defaultValue: '标识创建后不可改：它就是这个团队的地址。' })
            : t('experts.teams.idHelp', { defaultValue: '只能是小写字母、数字和连字符，长度 1~64。' })}
        </small>
      </label>
      <label className="full-row">
        {t('experts.descriptionLabel', { defaultValue: '描述' })}
        <textarea
          name="team_description"
          rows={2}
          value={draft.description}
          disabled={isPending}
          onChange={(event) => patch({ description: event.target.value })}
        />
      </label>
      <label className="full-row">
        {t('experts.teams.leader', { defaultValue: '主持人' })}
        <select
          name="leader_id"
          value={draft.leader_id}
          disabled={isPending}
          onChange={(event) => {
            const leaderId = event.target.value
            // 换了主持人就把旧主持人从成员里摘掉：后端会 400，前端先挡一道。
            patch({ leader_id: leaderId, member_ids: draft.member_ids.filter((mid) => mid !== leaderId) })
          }}
        >
          <option value="">{t('experts.teams.leaderPlaceholder', { defaultValue: '选择一名专家当主持人' })}</option>
          {experts.map((expert) => (
            <option key={expert.id} value={expert.id}>{expert.display_name || expert.id}</option>
          ))}
        </select>
        <small>{t('experts.teams.leaderHint', { defaultValue: '主持人不计入成员人数，也不能同时被选为成员。' })}</small>
      </label>
      <fieldset className="full-row experts-team-members">
        <legend>
          {t('experts.teams.members', { defaultValue: '团队成员' })}
          <span className="experts-team-count">
            {t('experts.teams.membersSelected', { count: members.length, defaultValue: '已选 {{count}} 人' })}
          </span>
        </legend>
        <p className="field-help">{t('experts.teams.membersHint', { defaultValue: '保存时按当前勾选一次性设置成员，不是增量追加。' })}</p>
        <TeamMemberPicker
          value={members}
          experts={experts}
          leaderId={draft.leader_id}
          onChange={(ids) => patch({ member_ids: ids })}
        />
        {belowMin ? (
          <p className="experts-team-warn" role="alert">
            {t('experts.teams.membersMin', { defaultValue: '至少选择两名专家。' })}
          </p>
        ) : null}
        {members.length >= TEAM_MEMBERS_MAX ? (
          <p className="field-help">
            {t('experts.teams.membersMax', { defaultValue: '最多选择八名专家，先取消一名才能再选。' })}
          </p>
        ) : null}
      </fieldset>
      <div className="form-actions full-row">
        <button type="submit" className="primary-button" disabled={isPending || idInvalid || belowMin}>
          {editingExisting
            ? t('experts.teams.saveChanges', { defaultValue: '保存修改' })
            : t('experts.teams.create', { defaultValue: '创建团队' })}
        </button>
        <button type="button" className="secondary-button" onClick={onCancel}>
          {t('common.cancel', { defaultValue: '取消' })}
        </button>
      </div>
    </form>
  )
}

/** 去掉主持人与重复项，只留下还存在于专家列表里的 id（Octop TeamMemberPicker.tsx:35-51）。 */
function rosterIds(selected: string[], experts: Expert[], leaderId: string): string[] {
  const allowed = new Set(experts.map((item) => item.id))
  const seen = new Set<string>()
  const out: string[] = []
  for (const expertId of selected) {
    if (!allowed.has(expertId) || expertId === leaderId || seen.has(expertId)) continue
    seen.add(expertId)
    out.push(expertId)
  }
  return out
}

/** 照 Octop TeamMemberPicker.tsx：自带搜索、已选排前、勾选即整份覆盖。 */
function TeamMemberPicker({
  value,
  onChange,
  experts,
  leaderId,
}: {
  value: string[]
  onChange: (ids: string[]) => void
  experts: Expert[]
  leaderId: string
}): ReactNode {
  const { t } = useTranslation()
  const [query, setQuery] = useState('')
  // 第一次勾选后把当时的已选集冻住：新选的成员在本次编辑里不会立刻跳到列表最上面。
  const [frozenFront, setFrozenFront] = useState<Set<string> | null>(null)
  const selected = new Set(value)
  const frontIds = frozenFront ?? selected

  const candidates = experts.filter((item) => item.id !== leaderId)
  const needle = query.trim().toLowerCase()
  const filtered = candidates
    .filter((item) => !needle
      || item.display_name.toLowerCase().includes(needle)
      || (item.description ?? '').toLowerCase().includes(needle)
      || item.id.toLowerCase().includes(needle))
    .sort((left, right) => {
      const leftOn = frontIds.has(left.id) ? 0 : 1
      const rightOn = frontIds.has(right.id) ? 0 : 1
      if (leftOn !== rightOn) return leftOn - rightOn
      return (left.display_name || left.id).localeCompare(right.display_name || right.id)
    })

  const atMax = value.length >= TEAM_MEMBERS_MAX
  const toggle = (expertId: string): void => {
    if (frozenFront === null) setFrozenFront(new Set(value))
    onChange(selected.has(expertId) ? value.filter((item) => item !== expertId) : [...value, expertId])
  }

  return (
    <div className="experts-team-picker">
      <span className="experts-toolbar-search">
        <span className="experts-toolbar-search-icon"><IconSearch /></span>
        <input
          type="search"
          value={query}
          aria-label={t('experts.teams.membersSearch', { defaultValue: '搜索专家' })}
          placeholder={t('experts.teams.membersSearch', { defaultValue: '搜索专家' })}
          onChange={(event) => setQuery(event.target.value)}
        />
        {query ? (
          <button
            type="button"
            className="experts-toolbar-search-clear"
            aria-label={t('common.clear', { defaultValue: '清除' })}
            onClick={() => setQuery('')}
          >
            <IconClose />
          </button>
        ) : null}
      </span>
      {filtered.length === 0 ? (
        <p className="experts-team-picker-empty">
          {t('experts.teams.membersEmpty', { defaultValue: '没有可加入的专家' })}
        </p>
      ) : (
        <div className="experts-team-picker-grid">
          {filtered.map((expert) => {
            const active = selected.has(expert.id)
            const blocked = !active && atMax
            return (
              <button
                key={expert.id}
                type="button"
                className={active ? 'experts-team-pick is-on' : 'experts-team-pick'}
                aria-pressed={active}
                disabled={blocked}
                title={blocked ? t('experts.teams.membersMax', { defaultValue: '最多选择八名专家，先取消一名才能再选。' }) : undefined}
                onClick={() => toggle(expert.id)}
              >
                {active ? <span className="experts-team-pick-check"><IconCheck size={11} /></span> : null}
                <span className="experts-card-avatar" data-tone={expertTone(expert.id)}>{expertInitial(expert.id)}</span>
                <span className="experts-team-pick-text">
                  <strong>{expert.display_name || expert.id}</strong>
                  <small>{expert.description || expert.id}</small>
                </span>
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}
