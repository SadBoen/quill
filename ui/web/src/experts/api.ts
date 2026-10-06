import { apiJson } from '../api/client'

export const EXPERTS_ROUTE = '/api/experts'

export const TEAMS_ROUTE = '/api/teams'

/** 团队列表的 react-query key，定义理由同 EXPERTS_KEY。 */
export const TEAMS_KEY = ['teams'] as const

/** 成员人数下限/上限，与后端 400 的判定一致，前端先挡一道。 */
export const TEAM_MEMBERS_MIN = 2
export const TEAM_MEMBERS_MAX = 8

/** 专家 id 同时是 goose agent 文件名，所以必须是 kebab-case。 */
export const EXPERT_ID_PATTERN = /^[a-z0-9-]{1,64}$/

/**
 * 专家列表的 react-query key。专家页、专家库 tab 和对话页的专家下拉共用它，
 * 所以只在这里定义一次：三处若各写各的字面量，queryFn 的返回形状很容易悄悄走偏
 * （曾出现过一处包成 {experts:[...]}、另一处存裸数组，跳转后下拉直接是空的）。
 */
export const EXPERTS_KEY = ['experts'] as const

export interface Expert {
  id: string
  owner: string
  display_name: string
  description: string
  /** 人格正文（agent 文件的 markdown 正文），可能为空串。 */
  instructions: string
  /** null = 跟随实例默认模型。 */
  model: string | null
  visibility: string
  default_enabled: boolean
  is_builtin: boolean
  /** 是不是那个自动补齐的通用专家。由服务端判定，前端不许自己认 id。 */
  is_general: boolean
  /** 被引用的专家库模板 id（见 library.ts 的 LIBRARY_EXPERTS_BY_ID）；手写专家为 null。 */
  source_template: string | null
}

export interface ExpertCreateInput {
  id: string
  display_name: string
  description: string
  instructions?: string
  model?: string | null
  source_template?: string | null
}

export interface ExpertUpdateInput {
  display_name?: string
  description?: string
  instructions?: string
  model?: string | null
  default_enabled?: boolean
  source_template?: string | null
}

export interface ExpertDeleteResult {
  id: string
  deleted: boolean
  note: string
}

export function isValidExpertId(value: string): boolean {
  return EXPERT_ID_PATTERN.test(value)
}

export async function listExperts(): Promise<Expert[]> {
  const body = await apiJson<{ experts: Expert[] }>(EXPERTS_ROUTE)
  return body.experts ?? []
}

export function createExpert(input: ExpertCreateInput): Promise<Expert> {
  return apiJson<Expert>(EXPERTS_ROUTE, {
    method: 'POST',
    body: JSON.stringify(input),
  })
}

export function getExpert(id: string): Promise<Expert> {
  return apiJson<Expert>(`${EXPERTS_ROUTE}/${encodeURIComponent(id)}`)
}

export function updateExpert(id: string, input: ExpertUpdateInput): Promise<Expert> {
  return apiJson<Expert>(`${EXPERTS_ROUTE}/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify(input),
  })
}

export function deleteExpert(id: string): Promise<ExpertDeleteResult> {
  return apiJson<ExpertDeleteResult>(`${EXPERTS_ROUTE}/${encodeURIComponent(id)}`, { method: 'DELETE' })
}

export interface ExpertDraft {
  display_name: string
  id: string
  description: string
  instructions: string
  /** 空串表示跟随实例默认模型，对应契约里的 model: null。 */
  model: string
  default_enabled: boolean
  /** 派生来源模板 id；手写/编辑既有专家时为 null。 */
  source_template: string | null
}

export function emptyDraft(): ExpertDraft {
  return {
    display_name: '',
    id: '',
    description: '',
    instructions: '',
    model: '',
    default_enabled: true,
    source_template: null,
  }
}

export function draftOf(expert: Expert): ExpertDraft {
  return {
    display_name: expert.display_name,
    id: expert.id,
    description: expert.description,
    instructions: expert.instructions ?? '',
    model: expert.model ?? '',
    default_enabled: expert.default_enabled,
    source_template: expert.source_template ?? null,
  }
}

/** 团队即「一名主持人 + 2~8 名成员」的编队名单。quill 只存这份名单，不执行派工。 */
export interface Team {
  team_id: string
  name: string
  description: string | null
  leader_id: string
  /** 不含 leader，按 id 升序。 */
  member_ids: string[]
  created_at: number
  updated_at: number
}

export interface TeamCreateInput {
  team_id: string
  name: string
  description?: string | null
  leader_id: string
  member_ids: string[]
}

export type TeamUpdateInput = Omit<TeamCreateInput, 'team_id'>

export interface TeamDeleteResult {
  team_id: string
  deleted: boolean
  note: string
}

export async function listTeams(): Promise<Team[]> {
  const body = await apiJson<{ teams: Team[] }>(TEAMS_ROUTE)
  return body.teams ?? []
}

export function getTeam(id: string): Promise<Team> {
  return apiJson<Team>(`${TEAMS_ROUTE}/${encodeURIComponent(id)}`)
}

export function createTeam(input: TeamCreateInput): Promise<Team> {
  return apiJson<Team>(TEAMS_ROUTE, { method: 'POST', body: JSON.stringify(input) })
}

/** team_id 是路由的一部分，改名/改成员之外它不可改（与专家 id 同理）。 */
export function updateTeam(id: string, input: TeamUpdateInput): Promise<Team> {
  return apiJson<Team>(`${TEAMS_ROUTE}/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify(input),
  })
}

export function deleteTeam(id: string): Promise<TeamDeleteResult> {
  return apiJson<TeamDeleteResult>(`${TEAMS_ROUTE}/${encodeURIComponent(id)}`, { method: 'DELETE' })
}

export interface TeamDraft {
  team_id: string
  name: string
  description: string
  leader_id: string
  member_ids: string[]
}

export function emptyTeamDraft(): TeamDraft {
  return { team_id: '', name: '', description: '', leader_id: '', member_ids: [] }
}

export function teamDraftOf(team: Team): TeamDraft {
  return {
    team_id: team.team_id,
    name: team.name,
    description: team.description ?? '',
    leader_id: team.leader_id,
    member_ids: [...team.member_ids],
  }
}
