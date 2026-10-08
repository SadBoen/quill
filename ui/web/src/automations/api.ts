import { apiJson } from '../api/client'

/**
 * quill 的定时任务（Q042，2026-10-09 接通）：`/api/cron*` 是真后端了 ——
 * 排期支持 `every`（固定间隔）与 `at`（指定时刻，一次性），**不支持 `cron_expr`**
 * （要 cron 解析 + IANA 时区库，宁可不做也不做错；表单里那个选项已同步去掉）。
 */
export type CronSchedule =
  | { type: 'every'; every_seconds: number }
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
