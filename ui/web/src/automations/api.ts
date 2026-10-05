import { apiJson } from '../api/client'

/**
 * quill 后端没有定时任务能力：下面这些函数保留了上游的调用形状，
 * 但当前实例上 `/api/cron*` 全部返回 404，页面据此渲染「尚未接通」提示。
 */
export type CronSchedule =
  | { type: 'every'; every_seconds: number }
  | { type: 'cron'; cron_expr: string; tz: string }
  | { type: 'at'; at: string; tz: string }

export interface CronJobSummary {
  id: string
  name: string
  schedule: CronSchedule
  last_fired_at: string | null
  next_fire_at: string
  session_id: string | null
}

export interface CronJob {
  id: string
  name: string
  message: string
  schedule: CronSchedule
}

export interface CronJobPage {
  items: CronJobSummary[]
  next_offset?: number | null
}

export type CronWrite = { name?: string; message: string } & (
  | { every_seconds: number }
  | { cron_expr: string; tz: string }
  | { at: string; tz: string }
)

export const CRON_ROUTE = '/api/cron'
export const CRON_JOB_ROUTE = '/api/cron/{id}'

export function listCronJobs(offset: number): Promise<CronJobPage> {
  return apiJson<CronJobPage>(`${CRON_ROUTE}?limit=50&offset=${offset}`)
}

export function getCronJob(id: string): Promise<CronJob> {
  return apiJson<CronJob>(`${CRON_ROUTE}/${encodeURIComponent(id)}`)
}

export function createCronJob(body: CronWrite): Promise<CronJob> {
  return apiJson<CronJob>(CRON_ROUTE, { method: 'POST', body: JSON.stringify(body) })
}

export function updateCronJob(id: string, body: CronWrite): Promise<CronJob> {
  return apiJson<CronJob>(`${CRON_ROUTE}/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify(body),
  })
}

export function deleteCronJob(id: string): Promise<undefined> {
  return apiJson<undefined>(`${CRON_ROUTE}/${encodeURIComponent(id)}`, { method: 'DELETE' })
}
