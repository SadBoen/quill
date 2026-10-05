import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, StatusBadge } from '../components/Page'
import { chatErrorMessage } from '../chat/chatApi'
import {
  EXPERTS_KEY,
  type Expert,
  type ExpertDraft,
  createExpert,
  isValidExpertId,
  listExperts,
} from './api'
import { ExpertForm } from './ExpertForm'
import { CAPABILITY_GAPS, capabilityStatusLabel } from '../capabilityGaps'
import {
  LIBRARY_EXPERTS,
  LIBRARY_MAX_INSTRUCTIONS_CHARS,
  LIBRARY_SOURCE,
  type LibraryExpert,
} from './library'

/** 与 EXPERT_ID_PATTERN 的 {1,64} 上限同口径。 */
const EXPERT_ID_MAX_LEN = 64

/** 展开中的生成面板：模板 id + 打开那一刻算好的初值。之后 id 归用户管，不再跟着名称跳。 */
interface GeneratePanel {
  templateId: string
  draft: ExpertDraft
}

/** 刚创建成功的专家，用于卡片上的提示与聚焦。 */
interface CreatedRef {
  templateId: string
  expertId: string
}

/** 显示名 → kebab-case。中文等非 ASCII 字符整段丢掉，词间空白折成连字符。 */
function slugifyName(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, EXPERT_ID_MAX_LEN)
    .replace(/-+$/, '')
}

/** 只由 ASCII 字母组成的 slug 才值得用；纯数字（「新媒体运营 1」→ 1）没有辨识度，直接退回模板 id。 */
function isMeaningfulSlug(slug: string): boolean {
  return isValidExpertId(slug) && /[a-z]/.test(slug)
}

/**
 * 只有整个显示名本来就是拉丁字母时，才拿规整结果当标识。
 * 中文标签里混着个别拉丁词时（「AI 编程实战导师 2」）规整出来是 ai-2 这种
 * 四五个字符的残渣：既看不出是什么专家，也丢掉了「它派生自哪个模板」这层血缘。
 */
function isLatinName(name: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9 ._&-]*$/.test(name.trim())
}

export function LibraryTab(): ReactNode {
  const { t, i18n } = useTranslation()
  const client = useQueryClient()
  const mine = useQuery({ queryKey: EXPERTS_KEY, queryFn: listExperts, retry: false })
  const [panel, setPanel] = useState<GeneratePanel | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [detailId, setDetailId] = useState<string | null>(null)
  const [created, setCreated] = useState<CreatedRef | null>(null)
  const cardRefs = useRef(new Map<string, HTMLElement | null>())

  // 专家库原文是中文库，界面语言切到英文时用 En 字段，不要让英文界面混进中文。
  // 人格正文是数据不是界面文案，所以 instructions 任何语言都用模板原文。
  const en = i18n.resolvedLanguage?.startsWith('en') ?? false
  const myExperts: Expert[] = mine.data ?? []
  const derivedOf = (templateId: string): Expert[] =>
    myExperts.filter((expert) => expert.source_template === templateId)
  const generatedTemplateCount = LIBRARY_EXPERTS.filter((expert) => derivedOf(expert.id).length > 0).length

  const openPanel = (expert: LibraryExpert): void => {
    const label = en ? expert.labelEn || expert.labelZh : expert.labelZh
    const n = derivedOf(expert.id).length + 1
    const displayName = `${label} ${n}`
    const slug = slugifyName(displayName)
    const taken = new Set(myExperts.map((mineExpert) => mineExpert.id))
    const preferSlug = isLatinName(displayName) && isMeaningfulSlug(slug) && !taken.has(slug)
    const id = preferSlug ? slug : `${expert.id}-${n}`
    setError(null)
    setPanel({
      templateId: expert.id,
      draft: {
        display_name: displayName,
        id,
        description: en ? expert.descriptionEn || expert.descriptionZh : expert.descriptionZh,
        instructions: expert.instructions,
        model: '',
        default_enabled: true,
        source_template: expert.id,
      },
    })
  }

  const create = useMutation({
    mutationFn: (input: GeneratePanel) => createExpert({
      id: input.draft.id,
      display_name: input.draft.display_name,
      description: input.draft.description,
      instructions: input.draft.instructions,
      model: input.draft.model.trim() ? input.draft.model.trim() : null,
      source_template: input.draft.source_template,
    }),
    onSuccess: async (expert, input) => {
      setPanel(null)
      setError(null)
      setCreated({ templateId: input.templateId, expertId: expert.id })
      await client.invalidateQueries({ queryKey: EXPERTS_KEY })
    },
    onError: (err) => {
      setError(chatErrorMessage(err, t('experts.libGenerateFailed', { defaultValue: '生成失败。' })))
    },
  })

  // 等新专家真的出现在列表里再把焦点放到它所属的模板卡片上，且只放一次：
  // myExperts 每次渲染都是新数组，放进依赖会导致每次重渲染都抢焦点。
  const focusedRef = useRef<string | null>(null)
  useEffect(() => {
    if (!created || focusedRef.current === created.expertId) return
    if (!myExperts.some((item) => item.id === created.expertId)) return
    focusedRef.current = created.expertId
    cardRefs.current.get(created.templateId)?.focus()
  }, [created, mine.data, myExperts])

  return (
    <div className="settings-stack">
      <Card
        title={t('experts.libTitle', { defaultValue: '专家库' })}
        description={t('experts.libDescription', {
          count: LIBRARY_EXPERTS.length,
          added: generatedTemplateCount,
          defaultValue: '{{count}} 个预设专家，你已基于其中 {{added}} 个生成过自己的专家。每个模板可以生成任意多个专家。',
        })}
        actions={(
          <StatusBadge tone={generatedTemplateCount > 0 ? 'success' : 'neutral'}>
            {t('experts.libProgress', { added: generatedTemplateCount, count: LIBRARY_EXPERTS.length, defaultValue: '{{added}} / {{count}}' })}
          </StatusBadge>
        )}
      >
        <p className="field-help experts-lib-source">
          {t('experts.libSource', {
            repo: LIBRARY_SOURCE.repo,
            pin: LIBRARY_SOURCE.pin.slice(0, 12),
            license: LIBRARY_SOURCE.license,
            date: LIBRARY_SOURCE.extractedAt,
            defaultValue: '数据逐字取自 {{repo}} @ {{pin}}（{{license}}，抽取于 {{date}}），人格 markdown 逐字节保留。',
          })}
        </p>
        {/* 人格原文里有 Octop 专属能力（技能包/插件/渠道/定时），quill 还没接。
            不改原文（改了就不是抄 Octop），但必须让用户知道模型可能会提到做不到的事。 */}
        <p className="field-help experts-lib-warning" role="note">
          {t('experts.libCapabilityNote', {
            defaultValue:
              '注意：这些人格原文提到了技能包、插件、渠道、定时任务等能力，quill 目前都还没接通（相关路由仍是 501）。选这些专家后，模型可能会提到它实际上做不到的事——人格正文我们没有改写，也不做截断。',
          })}
        </p>
        {mine.isPending ? <p className="empty-card-copy">{t('common.loading', { defaultValue: '加载中…' })}</p> : null}

        <div className="experts-grid experts-lib-grid">
          {LIBRARY_EXPERTS.map((expert) => {
            const derived = derivedOf(expert.id)
            const open = panel?.templateId === expert.id
            const detailOpen = detailId === expert.id
            return (
              <article
                className="experts-card experts-lib-card"
                key={expert.id}
                data-expert={expert.id}
                tabIndex={-1}
                ref={(node) => {
                  if (node) cardRefs.current.set(expert.id, node)
                  else cardRefs.current.delete(expert.id)
                }}
              >
                <header className="experts-card-head">
                  <span
                    className="experts-lib-avatar"
                    style={{ background: expert.color || undefined }}
                    aria-hidden="true"
                  >
                    {(en ? expert.labelEn || expert.labelZh : expert.labelZh).slice(0, 1)}
                  </span>
                  <strong>{en ? expert.labelEn || expert.labelZh : expert.labelZh}</strong>
                  {derived.length > 0 ? (
                    <StatusBadge tone="success">
                      {t('experts.libDerivedCount', { count: derived.length, defaultValue: '已基于此模板创建 {{count}} 个' })}
                    </StatusBadge>
                  ) : null}
                  {expert.personaMissing ? (
                    <StatusBadge tone="warning">{t('experts.libPersonaMissing', { defaultValue: '人格缺失' })}</StatusBadge>
                  ) : null}
                </header>
                <p className="experts-card-id"><code>{expert.id}</code></p>
                <p className="experts-card-desc">
                  {en ? expert.descriptionEn || expert.descriptionZh : expert.descriptionZh}
                </p>
                <p className="experts-card-model">
                  <span className="experts-card-label">
                    {t('experts.libPersonaSize', { defaultValue: '人格' })}
                  </span>
                  <span>
                    {t('experts.libPersonaChars', {
                      chars: expert.instructionsChars,
                      max: LIBRARY_MAX_INSTRUCTIONS_CHARS,
                      defaultValue: '{{chars}} 字 / 上限 {{max}}',
                    })}
                    {' · '}
                    {t('experts.libQuickPrompts', { count: expert.quickPrompts.length, defaultValue: '{{count}} 条快捷提问' })}
                  </span>
                </p>
                {expert.personaMissing ? (
                  <p className="experts-persona is-missing">{expert.personaMissingNote}</p>
                ) : null}
                <footer className="experts-card-foot">
                  <button
                    type="button"
                    className="primary-button"
                    aria-expanded={open}
                    disabled={create.isPending || expert.personaMissing}
                    onClick={() => (open ? setPanel(null) : openPanel(expert))}
                  >
                    {open
                      ? t('common.collapse', { defaultValue: '收起' })
                      : t('experts.libGenerate', { defaultValue: '生成专家' })}
                  </button>
                  <button
                    type="button"
                    className="secondary-button"
                    aria-expanded={detailOpen}
                    onClick={() => setDetailId(detailOpen ? null : expert.id)}
                  >
                    {detailOpen
                      ? t('common.collapse', { defaultValue: '收起' })
                      : t('experts.libPreview', { defaultValue: '看人格与欢迎语' })}
                  </button>
                </footer>
                {created && created.templateId === expert.id
                  && derived.some((item) => item.id === created.expertId) ? (
                  <p className="experts-lib-created" role="status">
                    {t('experts.libCreated', {
                      name: myExperts.find((item) => item.id === created.expertId)?.display_name ?? created.expertId,
                      defaultValue: '已创建专家「{{name}}」，它已经是你自己的了，可以在「我的专家」里继续改。',
                    })}
                  </p>
                ) : null}
                {open && panel ? (
                  <div className="experts-lib-generate">
                    <p className="field-help experts-lib-prefill">
                      {t('experts.libPrefillNote', {
                        defaultValue: '人格正文已按模板全文预填，可以随意修改。原模板不受影响——改的是你自己这一份。',
                      })}
                    </p>
                    <div className="experts-lib-gap" role="note">
                      <p className="experts-card-label">
                        {t('experts.libGapTitle', { defaultValue: '这些能力还没做到能用' })}
                      </p>
                      <p className="experts-lib-gap-hint">
                        {t('experts.libGapHint', {
                          defaultValue: '人格原文提到了下面这些能力。标「部分接通」的是存储层已经能用、但对话里还调不到；标「未接通」的是路由压根不存在。表单里都没有对应开关——选它们不会有任何效果。',
                        })}
                      </p>
                      <ul className="experts-lib-gap-list">
                        {CAPABILITY_GAPS.map((capability) => (
                          <li key={capability.route}>
                            <span>{t(capability.labelKey, { defaultValue: capability.fallback })}</span>
                            <code>{capability.route}</code>
                            <span className="experts-card-label">
                              {capabilityStatusLabel(capability, t)}
                            </span>
                          </li>
                        ))}
                      </ul>
                    </div>
                    {error ? <p className="form-error" role="alert">{error}</p> : null}
                    {derived.length > 0 ? (
                      <div className="experts-lib-derived">
                        <p className="experts-card-label">
                          {t('experts.libDerivedTitle', { defaultValue: '由该模板生成的专家' })}
                        </p>
                        <ul className="experts-lib-derived-list">
                          {derived.map((item) => (
                            <li key={item.id}>
                              <span>{item.display_name || item.id}</span>
                              <code>{item.id}</code>
                            </li>
                          ))}
                        </ul>
                      </div>
                    ) : null}
                    <ExpertForm
                      id={`experts-generate-${expert.id}`}
                      mode="create"
                      initial={panel.draft}
                      instructionsRows={16}
                      isPending={create.isPending}
                      onSubmit={(draft) => create.mutate({ templateId: panel.templateId, draft })}
                      onCancel={() => setPanel(null)}
                    />
                  </div>
                ) : null}
                {detailOpen ? (
                  <div className="experts-lib-detail">
                    <p className="experts-lib-welcome">
                      <span className="experts-card-label">
                        {t('experts.libWelcome', { defaultValue: '欢迎语' })}
                      </span>
                      <span>{en ? expert.welcomeEn || expert.welcomeZh : expert.welcomeZh || '—'}</span>
                    </p>
                    <p className="experts-card-label">
                      {t('experts.libFiles', { count: expert.instructionsFiles.length, defaultValue: '人格文件（{{count}}）' })}
                    </p>
                    <ul className="experts-lib-files">
                      {expert.instructionsFiles.map((file) => <li key={file}><code>{file}</code></li>)}
                    </ul>
                    <p className="experts-card-label">
                      {t('experts.libPersonaHead', { defaultValue: '人格正文开头' })}
                    </p>
                    <pre className="experts-lib-persona">{expert.instructions.slice(0, 600)}</pre>
                  </div>
                ) : null}
              </article>
            )
          })}
        </div>
      </Card>
    </div>
  )
}
