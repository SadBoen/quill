import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice, PageHeader, StatusBadge } from '../components/Page'
import { chatErrorMessage } from '../chat/chatApi'
import {
  deleteSkill,
  enabledSummaryChars,
  listSkills,
  setSkillEnabled,
  skillsOf,
  SKILLS_ROUTE,
  type Skill,
} from './skillsApi'
import './skills.css'

/**
 * 技能包页。
 *
 * ## 这个页面为什么以前没有
 *
 * 后端的技能接口一直都在，**但前端一次都没调用过**。技能只在聊天页的
 * 「工具」面板里以一行「能力状态」出现过 —— 那是在说「这项能力接没接通」，
 * 不是在管技能。所以真机上一个技能都开不了：库里全是 `enabled = false`，
 * 而唯一能改它的 `PATCH /api/extensions/skills/{name}` 此前没登记。
 *
 * **后端做完了而开关没有出口，功能就等于没做。**
 *
 * ## 两条不能越过的线
 *
 * 1. **`enabled` 与 `model_can_see` 分开显示。** 开关开了但磁盘正文被删了，
 *    模型这一轮照样调不到。只显示「已启用」就是在说假话 —— 界面上看着能用，
 *    对话里模型压根不知道有这个技能。
 * 2. **常驻开销摆在最显眼的地方。** 每个启用技能的摘要都跟着每一轮请求走，
 *    开太多会把上下文窗口顶爆（ISSUE-036 实测 13 个技能 ≈ 10810 tokens）。
 *    开关随便点，所以代价必须一直摆在眼前。
 */

const SKILLS_KEY = ['skills', SKILLS_ROUTE] as const

/** 删除是破坏性的，且没有撤销 —— 必须二次确认。 */
function DeleteSkill({ skill }: { skill: Skill }): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()
  const [armed, setArmed] = useState(false)

  const remove = useMutation({
    mutationFn: () => deleteSkill(skill.slug),
    onSuccess: () => {
      setArmed(false)
      void client.invalidateQueries({ queryKey: SKILLS_KEY })
    },
  })

  if (!armed) {
    return (
      <button
        type="button"
        className="skills-link-button"
        data-testid={`skills-delete-${skill.slug}`}
        onClick={() => setArmed(true)}
      >
        {t('skills.delete', { defaultValue: '删除' })}
      </button>
    )
  }
  return (
    <span className="skills-confirm">
      <span className="field-help">
        {t('skills.deleteConfirm', {
          name: skill.slug,
          defaultValue: '删除「{{name}}」？正文文件也会一起删掉，这一步没法撤销。',
        })}
      </span>
      <button
        type="button"
        className="skills-link-button is-danger"
        disabled={remove.isPending}
        onClick={() => remove.mutate()}
      >
        {t('skills.deleteYes', { defaultValue: '确认删除' })}
      </button>
      <button type="button" className="skills-link-button" onClick={() => setArmed(false)}>
        {t('common.cancel', { defaultValue: '取消' })}
      </button>
      {remove.isError ? (
        <span className="form-error" role="alert">
          {chatErrorMessage(remove.error, t('skills.deleteFailed', { defaultValue: '删除失败。' }))}
        </span>
      ) : null}
    </span>
  )
}

function SkillRow({ skill }: { skill: Skill }): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()

  const toggle = useMutation({
    mutationFn: (next: boolean) => setSkillEnabled(skill.slug, next),
    onSuccess: () => void client.invalidateQueries({ queryKey: SKILLS_KEY }),
  })

  // 开关点亮后，界面上的「模型看得见」必须等服务端重新算过才更新，
  // 所以这里用的是重新拉回来的列表，而不是本地乐观改写。
  const canSee = skill.model_can_see
  const modelSees = skill.model_sees_summary ?? ''

  return (
    <li className="skills-row" data-slug={skill.slug} data-enabled={String(skill.enabled)}>
      <div className="skills-row-main">
        <div className="skills-row-head">
          <code className="skills-slug">{skill.slug}</code>
          {skill.enabled ? (
            <StatusBadge tone="success">
              {t('skills.badgeEnabled', { defaultValue: '已启用' })}
            </StatusBadge>
          ) : (
            <StatusBadge tone="neutral">
              {t('skills.badgeDisabled', { defaultValue: '未启用' })}
            </StatusBadge>
          )}
          {skill.enabled && canSee ? (
            <StatusBadge tone="success">
              {t('skills.badgeModelSees', { defaultValue: '模型这一轮看得见' })}
            </StatusBadge>
          ) : null}
          {skill.enabled && !canSee ? (
            <StatusBadge tone="warning">
              {t('skills.badgeEnabledButHidden', { defaultValue: '已启用 · 模型看不见' })}
            </StatusBadge>
          ) : null}
          {skill.content_missing ? (
            <StatusBadge tone="danger">
              {t('skills.badgeContentMissing', { defaultValue: '正文文件不在了' })}
            </StatusBadge>
          ) : null}
        </div>

        {/* 这里显示的是**模型这一轮真正看到的那段字**，不是库里的 description 列。
            两者在 description 为空时会分叉：库里那一列是空的，模型看到的却是
            正文开头一段。只显示空列会让人以为「启用了模型也不知道那是干嘛的」——
            而事实是模型看得到，只是不在那一列里。 */}
        {modelSees ? <p className="skills-desc">{modelSees}</p> : null}
        {!modelSees ? (
          <p className="field-help">
            {t('skills.noSummary', {
              defaultValue:
                '模型这一轮看不到这个技能的任何说明：既没有摘要，磁盘上也没有正文。打开开关也不会让模型知道它是干什么的。',
            })}
          </p>
        ) : null}

        <p className="field-help skills-meta">
          {t('skills.meta', {
            chars: skill.content_chars ?? 0,
            kind: skill.kind === 'builtin'
              ? t('skills.kindBuiltin', { defaultValue: '内置' })
              : t('skills.kindWorkspace', { defaultValue: '工作区' }),
            version: skill.version,
            defaultValue: '正文 {{chars}} 字符 · {{kind}} · v{{version}}',
          })}
        </p>

        {skill.enabled && !canSee && skill.not_mounted_reason ? (
          <p className="skills-why" role="note">
            {t('skills.whyHidden', {
              why: skill.not_mounted_reason,
              defaultValue: '开关是开的，但模型这一轮调不到它：{{why}}',
            })}
          </p>
        ) : null}
        {skill.enabled && !canSee && skill.content_missing ? (
          <p className="skills-why" role="note">
            {t('skills.whyContentMissing', {
              defaultValue: '开关是开的，但磁盘上找不到正文文件，所以工具表里没有它。',
            })}
          </p>
        ) : null}
      </div>

      <div className="skills-row-actions">
        <label className="skills-switch">
          <input
            type="checkbox"
            data-testid={`skills-toggle-${skill.slug}`}
            checked={skill.enabled}
            disabled={toggle.isPending}
            onChange={(e) => toggle.mutate(e.target.checked)}
          />
          <span className="field-help">
            {t('skills.toggleLabel', { defaultValue: '挂进对话工具表' })}
          </span>
        </label>
        {toggle.isError ? (
          <p className="form-error" role="alert">
            {chatErrorMessage(toggle.error, t('skills.toggleFailed', { defaultValue: '切换失败。' }))}
          </p>
        ) : null}
        <DeleteSkill skill={skill} />
      </div>
    </li>
  )
}

export function SkillsPage(): ReactNode {
  const { t } = useTranslation()
  const query = useQuery({ queryKey: SKILLS_KEY, queryFn: listSkills, retry: false })
  const skills = skillsOf(query.data)
  const enabled = skills.filter((s) => s.enabled)
  const visible = skills.filter((s) => s.model_can_see)
  const summaryChars = enabledSummaryChars(skills)

  return (
    <div className="page-scroll">
      <PageHeader
        eyebrow={t('skills.eyebrow', { defaultValue: '扩展' })}
        title={t('skills.title', { defaultValue: '技能包' })}
        description={t('skills.description', {
          defaultValue: '技能包就是挂进对话工具表的一篇方法说明：模型调得到它，正文才在那一刻回灌。',
        })}
        actions={(
          <button
            className="secondary-button"
            data-testid="skills-refresh"
            onClick={() => void query.refetch()}
          >
            {t('common.refresh', { defaultValue: '刷新' })}
          </button>
        )}
      />

      <div className="settings-stack">
        {query.isError ? (
          <ErrorNotice
            error={chatErrorMessage(
              query.error,
              t('skills.loadFailedRetry', {
                defaultValue: '技能列表没加载出来。下一步：点右上角刷新重试。',
              }),
            )}
          />
        ) : null}

        <Card
          title={t('skills.listTitle', { defaultValue: '已安装的技能' })}
          description={t('skills.listDescription', {
            count: skills.length,
            enabled: enabled.length,
            visible: visible.length,
            defaultValue:
              '共 {{count}} 个，启用 {{enabled}} 个 —— 其中模型这一轮真的看得见 {{visible}} 个。',
          })}
        >
          {query.isPending ? (
            <p className="empty-state">{t('common.loading', { defaultValue: '加载中…' })}</p>
          ) : null}
          {!query.isPending && skills.length === 0 ? (
            <p className="empty-state">
              {t('skills.empty', { defaultValue: '还没有安装任何技能包。' })}
            </p>
          ) : null}

          {skills.length > 0 ? (
            <p className="field-help skills-cost">
              {t('skills.cost', {
                chars: summaryChars,
                defaultValue:
                  '当前启用技能里，模型每一轮都会看到的说明合计 {{chars}} 字符。这部分每一轮请求都带着 —— 开太多会把上下文窗口顶爆，下次请求直接失败。',
              })}
            </p>
          ) : null}

          {/* 空列表时不渲染 <ul>：留一个空壳在 DOM 里，读屏软件会念出
              「列表，0 项」，而界面上写的是「还没有安装任何技能包」——
              两句话在描述同一个空状态，不该同时出现。 */}
          {skills.length > 0 ? (
            <ul className="skills-list">
              {skills.map((item) => (
                <SkillRow key={item.slug} skill={item} />
              ))}
            </ul>
          ) : null}
        </Card>
      </div>
    </div>
  )
}
