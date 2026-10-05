import { apiJson } from '../api/client'
import type { PresetProtocol } from './presets'

export const PROVIDERS_ROUTE = '/api/admin/providers'
export const MODELS_ROUTE = '/api/admin/models'

export type ProviderProtocol = PresetProtocol

export type ProviderKind = 'preset' | 'custom'

/** `GET/POST/PUT /api/admin/providers` 里的 provider 记录；没有 api_key 字段，只有 has_api_key。 */
export interface Provider {
  id: string
  name: string
  preset_id: string
  kind: ProviderKind
  protocol: ProviderProtocol
  base_url: string
  has_api_key: boolean
  model: string
  max_context_tokens: number
  compaction_threshold_tokens: number
  max_output_tokens: number
  enabled: boolean
  is_default: boolean
  created_at: number
  updated_at: number
}

/** 新建/更新 provider 的请求体；`api_key` 留空表示沿用服务端已保存的密钥。 */
export interface ProviderInput {
  name: string
  preset_id: string
  kind: ProviderKind
  protocol: ProviderProtocol
  base_url: string
  api_key?: string
  model: string
  max_context_tokens: number
  compaction_threshold_tokens: number
  max_output_tokens: number
  enabled?: boolean
}

export interface Model {
  id: string
  display_name: string
  modality: 'text' | 'unknown'
  context_window: number | null
  owned_by: string | null
}

export interface ProviderModels {
  provider_id: string
  probed_at: number
  /** 探测失败时服务端仍返 200，把中文原因放在这里；`models` 会是空数组。 */
  error: string | null
  models: Model[]
}

export interface PoolEntry {
  provider_id: string
  provider_name: string
  model: Model
  starred: boolean
}

export interface UnavailableProvider {
  provider_id: string
  provider_name: string
  error: string
}

/** 管理员主动停用（`enabled=false`）的 provider。与「探测失败」是两回事，必须分开显示。 */
export interface DisabledProvider {
  provider_id: string
  provider_name: string
  reason: string
}

export interface ModelPool {
  generated_at: number
  default_provider_id: string | null
  pool: PoolEntry[]
  unavailable: UnavailableProvider[]
  disabled: DisabledProvider[]
}

export async function listProviders(): Promise<Provider[]> {
  const body = await apiJson<{ providers: Provider[] }>(PROVIDERS_ROUTE)
  return body.providers
}

export function createProvider(input: ProviderInput): Promise<Provider> {
  return apiJson<Provider>(PROVIDERS_ROUTE, {
    method: 'POST',
    body: JSON.stringify(input),
  })
}

export function updateProvider(id: string, input: ProviderInput): Promise<Provider> {
  return apiJson<Provider>(`${PROVIDERS_ROUTE}/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify(input),
  })
}

export function deleteProvider(id: string): Promise<void> {
  return apiJson<void>(`${PROVIDERS_ROUTE}/${encodeURIComponent(id)}`, { method: 'DELETE' })
}

export function setDefaultProvider(id: string): Promise<Provider> {
  return apiJson<Provider>(`${PROVIDERS_ROUTE}/${encodeURIComponent(id)}/default`, { method: 'PUT' })
}

/** 把 provider 的代表模型改成 `model`（模型池里点 ☆ 走的就是这条）。 */
export function setProviderModel(id: string, model: string): Promise<Provider> {
  return apiJson<Provider>(`${PROVIDERS_ROUTE}/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify({ model }),
  })
}

/** 启用 / 停用。后端 PUT 是部分更新，只发这一个字段，其余字段原样保留。 */
export function setProviderEnabled(id: string, enabled: boolean): Promise<Provider> {
  return apiJson<Provider>(`${PROVIDERS_ROUTE}/${encodeURIComponent(id)}`, {
    method: 'PUT',
    body: JSON.stringify({ enabled }),
  })
}

export async function listProviderModels(id: string): Promise<ProviderModels> {
  return apiJson<ProviderModels>(`${PROVIDERS_ROUTE}/${encodeURIComponent(id)}/models`)
}

export async function loadModelPool(): Promise<ModelPool> {
  return apiJson<ModelPool>(MODELS_ROUTE)
}
