import { type ReactNode, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { StatusBadge } from '../components/Page'
import { expertInitial, expertTone } from '../chat/expertAvatar'
import type { Expert } from './api'
import { Popconfirm, ToggleSwitch, copyText } from './ExpertsUi'
import { IconCheck, IconCopy, IconPencil, IconTrash } from './icons'

/**
 * 表格视图：列与文案照 Octop 的 AgentExpertsTable（experts.table.*），
 * 数据源按 quill 实情映射——「状态」列是 default_enabled（quill 没有常驻 agent 进程，
 * 没有 running/stopping，也不画启动/停止/重载按钮），「人格」列只展示有无正文。
 */
export function ExpertTable({
  experts,
  editingId,
  isPending,
  deletePending,
  togglePending,
  onEdit,
  onDelete,
  onToggleEnabled,
}: {
  experts: Expert[]
  editingId: string | null
  isPending: boolean
  deletePending: boolean
  togglePending: boolean
  onEdit: (id: string) => void
  onDelete: (id: string) => void
  onToggleEnabled: (id: string, enabled: boolean) => void
}): ReactNode {
  const { t } = useTranslation()

  return (
    <div className="experts-table-wrap">
      <table className="experts-table">
        <thead>
          <tr>
            <th>{t('experts.table.name', { defaultValue: '名称' })}</th>
            <th>{t('experts.table.agentId', { defaultValue: '专家 ID' })}</th>
            <th>{t('experts.table.state', { defaultValue: '状态' })}</th>
            <th>{t('experts.table.description', { defaultValue: '描述' })}</th>
            <th>{t('experts.table.model', { defaultValue: '模型' })}</th>
            <th>{t('experts.table.persona', { defaultValue: '人格' })}</th>
            <th>{t('experts.table.actions', { defaultValue: '操作' })}</th>
          </tr>
        </thead>
        <tbody>
          {experts.map((expert) => {
            const name = expert.display_name || expert.id
            const hasInstructions = (expert.instructions ?? '').trim().length > 0
            return (
              <tr key={expert.id} className={editingId === expert.id ? 'is-editing' : undefined}>
                <td>
                  <div className="experts-table-name-cell">
                    <span className="experts-table-avatar" data-tone={expertTone(expert.id)}>
                      {expertInitial(expert.id)}
                    </span>
                    <span className="experts-table-name-text">{name}</span>
                    {expert.is_builtin ? (
                      <StatusBadge tone="neutral">{t('experts.builtin', { defaultValue: '内置' })}</StatusBadge>
                    ) : null}
                    {expert.source_template ? (
                      <StatusBadge tone="neutral">
                        {t('experts.fromTemplate', { defaultValue: '来自模板' })}
                      </StatusBadge>
                    ) : null}
                  </div>
                </td>
                <td>
                  <CopyIdButton id={expert.id} />
                </td>
                <td>
                  <div className="experts-table-state">
                    <StatusBadge tone={expert.default_enabled ? 'success' : 'neutral'}>
                      {expert.default_enabled
                        ? t('experts.stateEnabled', { defaultValue: '默认启用' })
                        : t('experts.stateDisabled', { defaultValue: '未启用' })}
                    </StatusBadge>
                    <ToggleSwitch
                      checked={expert.default_enabled}
                      pending={togglePending}
                      label={t('experts.toggleDefault', { defaultValue: '设为默认启用' })}
                      onChange={(next) => onToggleEnabled(expert.id, next)}
                    />
                  </div>
                </td>
                <td className="experts-table-ellipsis">{expert.description || '—'}</td>
                <td className="experts-table-ellipsis">{expert.model || '—'}</td>
                <td>
                  <span className={hasInstructions ? 'experts-persona' : 'experts-persona is-missing'}>
                    {hasInstructions
                      ? t('experts.hasInstructions', { defaultValue: '已设置人格' })
                      : t('experts.noInstructions', { defaultValue: '未设置人格' })}
                  </span>
                </td>
                <td>
                  <div className="experts-table-actions">
                    <button
                      type="button"
                      className="experts-table-action-btn"
                      aria-label={t('common.edit', { defaultValue: '编辑' })}
                      disabled={isPending}
                      onClick={() => onEdit(expert.id)}
                    >
                      <IconPencil />
                    </button>
                    <Popconfirm
                      className="experts-table-action-btn is-danger"
                      label={t('common.delete', { defaultValue: '删除' })}
                      icon={<IconTrash />}
                      title={t('experts.confirmDelete', {
                        name,
                        defaultValue: '删除「{{name}}」？',
                      })}
                      description={t('experts.confirmDeleteHint', {
                        defaultValue: '这是软删除：删除后该专家对所有用户（含属主）不可见，同名 id 可以再次创建。',
                      })}
                      okText={t('common.delete', { defaultValue: '删除' })}
                      cancelText={t('common.cancel', { defaultValue: '取消' })}
                      dangerOk
                      pending={deletePending}
                      onConfirm={() => onDelete(expert.id)}
                    />
                  </div>
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

function CopyIdButton({ id }: { id: string }): ReactNode {
  const { t } = useTranslation()
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle')

  useEffect(() => {
    // 成功提示自动消失；失败提示要常驻——它是在叫用户动手，不能一闪而过。
    if (state !== 'copied') return
    const timer = setTimeout(() => setState('idle'), 1800)
    return () => clearTimeout(timer)
  }, [state])

  const hint = t('experts.copyAgentId', { defaultValue: '点击复制专家 ID' })

  return (
    <span className="experts-copy-id">
      <button
        type="button"
        className="experts-copy-id-btn"
        title={hint}
        aria-label={hint}
        onClick={() => {
          void copyText(id).then((outcome) => {
            setState(outcome === 'failed' ? 'failed' : 'copied')
          })
        }}
      >
        <span className="experts-copy-id-value">{id}</span>
        {state === 'copied' ? <IconCheck /> : <IconCopy />}
      </button>
      {state === 'copied' ? (
        <span className="experts-copied-tip" role="status">
          {t('experts.copied', { defaultValue: '已复制' })}
        </span>
      ) : null}
      {state === 'failed' ? (
        <span className="experts-copied-tip is-error" role="alert">
          {t('experts.copyFailed', { defaultValue: '复制失败，请手动选中上面的 id 复制' })}
        </span>
      ) : null}
    </span>
  )
}
