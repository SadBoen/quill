import { useInfiniteQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { type FormEvent, type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router-dom'

import { Card, ErrorNotice, PageHeader } from '../components/Page'
import {
  CRON_JOB_ROUTE,
  CRON_ROUTE,
  createCronJob,
  deleteCronJob,
  getCronJob,
  listCronJobs,
  updateCronJob,
  type CronJob,
  type CronJobSummary,
  type CronSchedule,
  type CronWrite,
} from './api'
import { DREAM_ROUTE, listDream, type DreamItem } from './dreamApi'
import { pickVisibleError, routeMissing } from './capability'

const HEARTBEAT_PATH = 'HEARTBEAT.md'
const HEARTBEAT_TEMPLATE = `# Heartbeat

## Active Tasks

<!-- Add recurring checks here. Use Cron for exact execution times. -->
`

type ScheduleKind = CronSchedule['type']

interface CronDraft {
  id: string | null
  name: string
  message: string
  kind: ScheduleKind
  everySeconds: string
  cronExpr: string
  at: string
  timezone: string
}

export function AutomationsPage(): ReactNode {
  const { i18n, t } = useTranslation()
  const queryClient = useQueryClient()
  const timezone = clientTimezone()
  const [draft, setDraft] = useState<CronDraft | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<CronJobSummary | null>(null)
  const [actionError, setActionError] = useState<unknown>(null)

  const cron = useInfiniteQuery({
    queryKey: ['cron-jobs'],
    initialPageParam: 0,
    queryFn: ({ pageParam }) => listCronJobs(pageParam),
    getNextPageParam: (lastPage) => lastPage.next_offset ?? undefined,
    retry: false,
  })
  const dream = useInfiniteQuery({
    queryKey: ['dream'],
    initialPageParam: 0,
    queryFn: ({ pageParam }) => listDream(pageParam),
    getNextPageParam: (lastPage) => lastPage.next_offset ?? undefined,
    retry: false,
  })

  const saveCron = useMutation({
    mutationFn: ({ id, body }: { id: string | null; body: CronWrite }) => (
      id ? updateCronJob(id, body) : createCronJob(body)
    ),
    onSuccess: async () => {
      setDraft(null)
      await queryClient.invalidateQueries({ queryKey: ['cron-jobs'] })
    },
  })

  const removeCron = useMutation({
    mutationFn: (id: string) => deleteCronJob(id),
    onSuccess: async () => {
      setDeleteTarget(null)
      await queryClient.invalidateQueries({ queryKey: ['cron-jobs'] })
    },
  })

  const startEdit = async (job: CronJobSummary): Promise<void> => {
    setActionError(null)
    try {
      setDraft(cronDraft(await getCronJob(job.id), timezone))
    } catch (error) {
      setActionError(error)
    }
  }

  const jobs = cron.data?.pages.flatMap((page) => page.items) ?? []
  const dreamItems: DreamItem[] = dream.data?.pages.flatMap((page) => page.items) ?? []

  // 这台实例有没有定时任务这个能力。**没有就别画那个表单** ——
  // 理由见 capability.ts：能填能提交但必然 404 的表单，比没有这个功能更糟，
  // 因为用户会以为它建成了。下方那句「下一步：实现 /api/cron…」已经说清了缺什么。
  const cronUnavailable = routeMissing(cron.error)

  return (
    <div className="page-scroll">
      <PageHeader
        eyebrow={t('automations.eyebrow', { defaultValue: '自动化' })}
        title={t('automations.title', { defaultValue: '自动化' })}
        description={t('automations.description', { defaultValue: '定时任务与心跳。' })}
      />
      <ErrorNotice error={pickVisibleError(actionError, saveCron.error, removeCron.error, cron.error, dream.error)} />
      <div className="settings-stack automations-stack">
        <Card
          title={t('automations.cronTitle', { defaultValue: '定时任务' })}
          description={t('automations.cronDescription', { defaultValue: '按固定间隔、cron 表达式或指定时刻投递一条消息。' })}
          actions={draft || cronUnavailable ? null : (
            <button
              type="button"
              className="primary-button"
              onClick={() => {
                setActionError(null)
                setDraft(emptyCronDraft(timezone))
              }}
            >
              {t('automations.createAutomation', { defaultValue: '新建自动化' })}
            </button>
          )}
        >
          {draft && !cronUnavailable ? (
            <CronForm
              draft={draft}
              pending={saveCron.isPending}
              onChange={setDraft}
              onCancel={() => setDraft(null)}
              onSubmit={(body) => saveCron.mutate({ id: draft.id, body })}
            />
          ) : null}

          {cron.isPending ? <p className="page-status">{t('automations.loadingCron', { defaultValue: '正在加载定时任务…' })}</p> : null}
          {cronUnavailable ? (
            <p className="field-help" role="status">
              {t('automations.cronNotWired', {
                defaultValue: 'quill 后端没有定时任务能力：{{route}} 未在本实例登记，所以这里没有任何任务，也不会显示示例数据。下一步：实现 {{route}}（列表）与 {{job}}（增删改），本卡片会自动接上。',
                route: CRON_ROUTE,
                job: CRON_JOB_ROUTE,
              })}
            </p>
          ) : null}
          {!cron.isPending && !cronUnavailable && !jobs.length ? <p className="empty-card-copy">{t('automations.noCron', { defaultValue: '还没有定时任务。' })}</p> : null}
          {jobs.length ? (
            <div className="automation-list">
              {jobs.map((job) => (
                <article className="automation-row" key={job.id}>
                  <div className="automation-row-main">
                    <h3>{job.name}</h3>
                    <p><ScheduleLabel schedule={job.schedule} language={i18n.resolvedLanguage} t={t} /></p>
                    <dl className="automation-times">
                      <div>
                        <dt>{t('automations.lastRun', { defaultValue: '上次运行' })}</dt>
                        <dd>{job.last_fired_at
                          ? <Timestamp value={job.last_fired_at} timezone={timezone} language={i18n.resolvedLanguage} />
                          : t('automations.neverRun', { defaultValue: '从未运行' })}</dd>
                      </div>
                      <div>
                        <dt>{t('automations.nextRun', { defaultValue: '下次运行' })}</dt>
                        <dd><Timestamp value={job.next_fire_at} timezone={timezone} language={i18n.resolvedLanguage} /></dd>
                      </div>
                    </dl>
                  </div>
                  <div className="automation-row-actions">
                    {job.session_id ? (
                      <Link className="secondary-button" to={`/chat/${encodeURIComponent(job.session_id)}?automation=cron`}>
                        {t('automations.viewHistory', { defaultValue: '查看记录' })}
                      </Link>
                    ) : <span className="status-badge">{t('automations.noRuns', { defaultValue: '暂无记录' })}</span>}
                    <button
                      type="button"
                      className="secondary-button"
                      onClick={() => void startEdit(job)}
                    >
                      {t('common.edit', { defaultValue: '编辑' })}
                    </button>
                    <button
                      type="button"
                      className="danger-button"
                      onClick={() => setDeleteTarget(job)}
                    >
                      {t('common.delete', { defaultValue: '删除' })}
                    </button>
                  </div>
                </article>
              ))}
            </div>
          ) : null}
          {cron.hasNextPage ? (
            <div className="form-actions automation-load-more">
              <button
                type="button"
                className="secondary-button"
                disabled={cron.isFetchingNextPage}
                onClick={() => void cron.fetchNextPage()}
              >
                {t('automations.loadMore', { defaultValue: '加载更多' })}
              </button>
            </div>
          ) : null}
        </Card>

        <Card
          title={t('automations.heartbeatTitle', { defaultValue: '心跳' })}
          description={t('automations.heartbeatDescription', { defaultValue: '一份长期存在的待办清单，智能体每轮都会读它。' })}
        >
          <div className="heartbeat-meta">
            <span>{t('automations.accountTimezone', { timezone, defaultValue: '本机时区：{{timezone}}' })}</span>
            <Link to="/account">{t('automations.changeTimezone', { defaultValue: '去账户页' })}</Link>
          </div>
          <p className="field-help">
            {t('automations.heartbeatNotWired', {
              defaultValue: 'quill 没有工作区文件接口（{{route}} 未登记），所以 {{file}} 既读不到也写不了。下方是空白模板，不是任何一次真实运行的结果。下一步：实现 {{route}} 的读写，或者把心跳文件落到资料库（目前只有只读的 GET /api/wiki/pages/{{path}}）。',
              route: '/api/workspace/files',
              file: HEARTBEAT_PATH,
              path: 'index.md',
            })}
          </p>
          <div className="heartbeat-editor">
            <label>
              {t('automations.heartbeatFile', { defaultValue: '心跳文件（' + HEARTBEAT_PATH + '）' })}
              <textarea rows={13} maxLength={32000} readOnly defaultValue={HEARTBEAT_TEMPLATE} />
            </label>
            <div className="form-actions">
              <button type="button" className="primary-button" disabled>
                {t('automations.saveHeartbeat', { defaultValue: '保存心跳文件' })}
              </button>
            </div>
          </div>
        </Card>

        <Card
          title={t('automations.dreamTitle', { defaultValue: '记忆整理' })}
          description={t('automations.dreamDescription', { defaultValue: '后台把对话里的结论整理成长期记忆。' })}
        >
          <p className="field-help" role="status">
            {dream.isError
              ? t('automations.dreamNotWired', {
                defaultValue: 'quill 没有记忆整理服务：{{route}} 未登记，因此这里没有可展示的运行记录（不显示任何假时间戳或假状态）。下一步：实现 {{route}}，再把整理结果写进资料库页面。',
                route: DREAM_ROUTE,
              })
              : t('automations.dreamEmpty', { defaultValue: '还没有记忆整理记录。' })}
          </p>
          {dream.isPending ? <p className="page-status">{t('automations.dreamLoading', { defaultValue: '正在加载整理记录…' })}</p> : null}
          <div className="automation-list">
            {dreamItems.map((item) => (
              <article className="automation-row" key={item.id}>
                <div className="automation-row-main">
                  <h3>{t(`automations.dreamStatus.${item.status}`, { defaultValue: item.status })}</h3>
                  <p>{t('automations.dreamMessages', { count: item.message_count, defaultValue: '整理了 {{count}} 条消息' })}</p>
                </div>
              </article>
            ))}
          </div>
        </Card>
      </div>

      {deleteTarget ? (
        <div className="modal-backdrop" role="presentation">
          <div
            className="modal-card"
            role="dialog"
            aria-modal="true"
            aria-labelledby="delete-cron-title"
            onKeyDown={(event) => {
              if (event.key === 'Escape') setDeleteTarget(null)
            }}
          >
            <h2 id="delete-cron-title">{t('automations.deleteTitle', { defaultValue: '删除定时任务' })}</h2>
            <p>{t('automations.deleteWarning', { name: deleteTarget.name, defaultValue: '确认删除「{{name}}」？' })}</p>
            <div className="form-actions">
              <button type="button" className="secondary-button" autoFocus onClick={() => setDeleteTarget(null)}>
                {t('common.cancel', { defaultValue: '取消' })}
              </button>
              <button
                type="button"
                className="danger-button"
                disabled={removeCron.isPending}
                onClick={() => removeCron.mutate(deleteTarget.id)}
              >
                {t('automations.confirmDelete', { defaultValue: '确认删除' })}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  )
}

function CronForm({
  draft,
  pending,
  onChange,
  onCancel,
  onSubmit,
}: {
  draft: CronDraft
  pending: boolean
  onChange: (draft: CronDraft) => void
  onCancel: () => void
  onSubmit: (body: CronWrite) => void
}): ReactNode {
  const { t } = useTranslation()
  const submit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault()
    onSubmit(cronWrite(draft))
  }
  return (
    <form className="form-grid automation-form" onSubmit={submit}>
      <label>
        {t('automations.name', { defaultValue: '名称' })}
        <input name="name" autoComplete="off" required value={draft.name} maxLength={120} onChange={(event) => onChange({ ...draft, name: event.target.value })} />
      </label>
      <label>
        {t('automations.scheduleType', { defaultValue: '触发方式' })}
        <select name="scheduleType" autoComplete="off" value={draft.kind} onChange={(event) => onChange({ ...draft, kind: event.target.value as ScheduleKind })}>
          <option value="every">{t('automations.every', { defaultValue: '固定间隔' })}</option>
          <option value="cron">{t('automations.cron', { defaultValue: 'cron 表达式' })}</option>
          <option value="at">{t('automations.once', { defaultValue: '指定时刻' })}</option>
        </select>
      </label>
      <label className="full-row">
        {t('automations.task', { defaultValue: '要发送的内容' })}
        <textarea name="message" autoComplete="off" rows={5} required maxLength={32000} value={draft.message} onChange={(event) => onChange({ ...draft, message: event.target.value })} />
      </label>
      {draft.kind === 'every' ? (
        <label>
          {t('automations.everySeconds', { defaultValue: '间隔秒数' })}
          <input name="everySeconds" autoComplete="off" type="number" min="60" max="31536000" required value={draft.everySeconds} onChange={(event) => onChange({ ...draft, everySeconds: event.target.value })} />
        </label>
      ) : null}
      {draft.kind === 'cron' ? (
        <label>
          {t('automations.cronExpression', { defaultValue: 'cron 表达式' })}
          <input name="cronExpr" autoComplete="off" required maxLength={256} value={draft.cronExpr} placeholder="0 9 * * 1-5" onChange={(event) => onChange({ ...draft, cronExpr: event.target.value })} />
        </label>
      ) : null}
      {draft.kind === 'at' ? (
        <label>
          {t('automations.runAt', { defaultValue: '运行时刻' })}
          <input name="runAt" autoComplete="off" required maxLength={128} value={draft.at} placeholder="2026-09-03T10:00:00+08:00" onChange={(event) => onChange({ ...draft, at: event.target.value })} />
        </label>
      ) : null}
      {draft.kind !== 'every' ? (
        <label>
          {t('automations.timezone', { defaultValue: '时区' })}
          <input name="timezone" autoComplete="off" required maxLength={64} value={draft.timezone} onChange={(event) => onChange({ ...draft, timezone: event.target.value })} />
        </label>
      ) : null}
      <div className="form-actions full-row">
        <button type="button" className="secondary-button" disabled={pending} onClick={onCancel}>{t('common.cancel', { defaultValue: '取消' })}</button>
        <button className="primary-button" disabled={pending}>
          {draft.id ? t('automations.saveChanges', { defaultValue: '保存改动' }) : t('automations.create', { defaultValue: '创建' })}
        </button>
      </div>
    </form>
  )
}

function emptyCronDraft(timezone: string): CronDraft {
  return {
    id: null,
    name: '',
    message: '',
    kind: 'every',
    everySeconds: '3600',
    cronExpr: '0 9 * * 1-5',
    at: '',
    timezone,
  }
}

function cronDraft(job: CronJob, fallbackTimezone: string): CronDraft {
  const draft = emptyCronDraft(fallbackTimezone)
  if (job.schedule.type === 'every') {
    return { ...draft, id: job.id, name: job.name, message: job.message, everySeconds: String(job.schedule.every_seconds) }
  }
  if (job.schedule.type === 'cron') {
    return { ...draft, id: job.id, name: job.name, message: job.message, kind: 'cron', cronExpr: job.schedule.cron_expr, timezone: job.schedule.tz }
  }
  return {
    ...draft,
    id: job.id,
    name: job.name,
    message: job.message,
    kind: 'at',
    at: job.schedule.at,
    timezone: job.schedule.tz,
  }
}

function cronWrite(draft: CronDraft): CronWrite {
  const base = { name: draft.name, message: draft.message }
  if (draft.kind === 'every') return { ...base, every_seconds: Number(draft.everySeconds) }
  if (draft.kind === 'cron') return { ...base, cron_expr: draft.cronExpr, tz: draft.timezone }
  return { ...base, at: draft.at, tz: draft.timezone }
}

function clientTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}

function ScheduleLabel({
  schedule,
  language,
  t,
}: {
  schedule: CronSchedule
  language?: string
  t: (key: string, options?: Record<string, unknown>) => string
}): ReactNode {
  if (schedule.type === 'every') {
    return t('automations.everySchedule', {
      count: schedule.every_seconds,
      defaultValue: '每 {{count}} 秒',
    })
  }
  if (schedule.type === 'cron') return `${schedule.cron_expr} · ${schedule.tz}`
  return <><time dateTime={schedule.at}>{formatTimestamp(schedule.at, schedule.tz, language)}</time> · {schedule.tz}</>
}

function Timestamp({ value, timezone, language }: { value: string; timezone: string; language?: string }): ReactNode {
  return <time dateTime={value}>{formatTimestamp(value, timezone, language)}</time>
}

function formatTimestamp(value: string, timezone: string, language?: string): string {
  return new Intl.DateTimeFormat(language, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    timeZoneName: 'short',
    timeZone: timezone,
  }).format(new Date(value))
}
