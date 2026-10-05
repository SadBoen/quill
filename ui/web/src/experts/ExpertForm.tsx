import { type FormEvent, type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { type ExpertDraft, isValidExpertId } from './api'
import { findLibraryExpert } from './library'

export interface ExpertFormProps {
  id: string
  initial: ExpertDraft
  isPending: boolean
  /**
   * create = 新建（id 可改），edit = 改既有专家（id 锁定）。
   * 不传时按 initial.id 是否为空推断，保持旧调用点的行为。
   */
  mode?: 'create' | 'edit'
  /** 人格正文的可见行数；生成面板要读整份灵魂文件，单独放大。 */
  instructionsRows?: number
  onSubmit: (draft: ExpertDraft) => void
  onCancel: () => void
}

export function ExpertForm({
  id,
  initial,
  isPending,
  mode,
  instructionsRows = 12,
  onSubmit,
  onCancel,
}: ExpertFormProps): ReactNode {
  const { t, i18n } = useTranslation()
  const editingExisting = mode ? mode === 'edit' : initial.id !== ''
  const [draft, setDraft] = useState<ExpertDraft>(initial)
  const [followDefault, setFollowDefault] = useState(initial.model === '')
  const [localError, setLocalError] = useState<string | null>(null)

  const patch = (next: Partial<ExpertDraft>): void => {
    setDraft((current) => ({ ...current, ...next }))
  }

  const idInvalid = draft.id !== '' && !isValidExpertId(draft.id)
  const sourceLabel = draft.source_template ? findLibraryExpert(draft.source_template) : undefined
  // 先算好模板显示名再传进 t()：把三元表达式写进调用点会让 .i18n-check.mjs
  // 把三元里的 labelEn / labelZh 当成传参，报出「死参数」假阳性。
  const sourceLabelText = sourceLabel
    ? (i18n.resolvedLanguage?.startsWith('en') ? (sourceLabel.labelEn || sourceLabel.labelZh) : sourceLabel.labelZh)
    : ''

  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault()
    const displayName = draft.display_name.trim()
    if (!displayName) {
      setLocalError(t('experts.nameRequired', { defaultValue: '请填写显示名称。' }))
      return
    }
    if (!editingExisting) {
      if (!draft.id) {
        setLocalError(t('experts.idRequired', { defaultValue: '请填写标识。' }))
        return
      }
      if (!isValidExpertId(draft.id)) {
        setLocalError(t('experts.idInvalid', {
          defaultValue: '标识不合法：标识只能是小写字母、数字和连字符，长度 1~64；它是专家的对外标识，也是底层 agent 的文件名。',
        }))
        return
      }
    }
    if (!followDefault && !draft.model.trim()) {
      setLocalError(t('experts.modelRequired', { defaultValue: '选了「指定模型」就要填模型名。' }))
      return
    }
    setLocalError(null)
    onSubmit({ ...draft, display_name: displayName, model: followDefault ? '' : draft.model.trim() })
  }

  return (
    <form className="form-grid experts-form" id={id} onSubmit={submit}>
      {draft.source_template ? (
        <p className="field-help experts-form-source full-row">
          {t('experts.formBasedOnTemplate', {
            template: draft.source_template,
            defaultValue: '此专家基于模板 {{template}} 生成。',
          })}
          {sourceLabel ? (
            <span className="experts-form-source-label">
              {t('experts.formSourceTemplateName', {
                label: sourceLabelText,
                defaultValue: '（{{label}}）',
              })}
            </span>
          ) : null}
        </p>
      ) : null}
      {localError ? <p className="form-error full-row" role="alert">{localError}</p> : null}
      <label className="full-row">
        {t('experts.displayName', { defaultValue: '显示名称' })}
        <input
          name="display_name"
          value={draft.display_name}
          autoComplete="off"
          disabled={isPending}
          onChange={(event) => patch({ display_name: event.target.value })}
        />
      </label>
      <label className="full-row">
        {t('experts.idLabel', { defaultValue: '标识（id）' })}
        <input
          name="id"
          value={draft.id}
          autoComplete="off"
          spellCheck={false}
          disabled={isPending || editingExisting}
          aria-invalid={idInvalid}
          onChange={(event) => patch({ id: event.target.value })}
        />
        <small className={idInvalid ? 'experts-id-help is-error' : 'experts-id-help'}>
          {editingExisting
            ? t('experts.idLocked', { defaultValue: '标识创建后不可改：它就是底层 agent 的文件名。' })
            : t('experts.idHelp', {
              defaultValue: '标识只能是小写字母、数字和连字符，长度 1~64；它是专家的对外标识，也是底层 agent 的文件名。',
            })}
        </small>
      </label>
      <label className="full-row">
        {t('experts.descriptionLabel', { defaultValue: '描述' })}
        <input
          name="description"
          value={draft.description}
          autoComplete="off"
          disabled={isPending}
          onChange={(event) => patch({ description: event.target.value })}
        />
      </label>
      <label className="full-row">
        <span className="label-with-help">
          {t('experts.instructionsLabel', { defaultValue: '人格正文' })}
          <span className="experts-char-count">
            {t('experts.charCount', { count: draft.instructions.length, defaultValue: '{{count}} 字' })}
          </span>
        </span>
        <textarea
          name="instructions"
          rows={instructionsRows}
          value={draft.instructions}
          spellCheck={false}
          disabled={isPending}
          placeholder={t('experts.instructionsPlaceholder', {
            defaultValue: '写成这个专家的说话方式、做事原则和边界。',
          })}
          onChange={(event) => patch({ instructions: event.target.value })}
        />
        <small>
          {t('experts.instructionsHelp', {
            defaultValue: '这就是 markdown 正文，会作为该专家的系统提示发给模型；留空表示只有名称和描述。',
          })}
        </small>
      </label>
      <fieldset className="choice-field full-row">
        <legend>{t('experts.modelLabel', { defaultValue: '模型' })}</legend>
        <label>
          <input
            type="radio"
            name="model_mode"
            checked={followDefault}
            disabled={isPending}
            onChange={() => setFollowDefault(true)}
          />
          {t('experts.modelFollowDefault', { defaultValue: '跟随实例默认模型' })}
        </label>
        <label>
          <input
            type="radio"
            name="model_mode"
            checked={!followDefault}
            disabled={isPending}
            onChange={() => setFollowDefault(false)}
          />
          {t('experts.modelCustom', { defaultValue: '指定模型' })}
        </label>
        {followDefault ? null : (
          <label className="experts-model-input">
            {t('experts.modelName', { defaultValue: '模型名' })}
            <input
              name="model"
              value={draft.model}
              autoComplete="off"
              spellCheck={false}
              disabled={isPending}
              onChange={(event) => patch({ model: event.target.value })}
            />
            <small>
              {t('experts.modelNameHelp', {
                defaultValue: '直接填模型标识，服务端不会校验拼写；不选「跟随实例默认模型」时才生效。',
              })}
            </small>
          </label>
        )}
      </fieldset>
      {editingExisting ? (
        <label className="toggle-field full-row">
          <input
            type="checkbox"
            name="default_enabled"
            checked={draft.default_enabled}
            disabled={isPending}
            onChange={(event) => patch({ default_enabled: event.target.checked })}
          />
          <span>{t('experts.defaultEnabled', { defaultValue: '默认启用（出现在对话页的专家下拉里）' })}</span>
        </label>
      ) : null}
      <div className="form-actions full-row">
        <button type="submit" className="primary-button" disabled={isPending || idInvalid}>
          {editingExisting
            ? t('experts.saveChanges', { defaultValue: '保存修改' })
            : t('experts.create', { defaultValue: '创建专家' })}
        </button>
        <button type="button" className="secondary-button" onClick={onCancel}>
          {t('common.cancel', { defaultValue: '取消' })}
        </button>
      </div>
    </form>
  )
}
