import { apiJson } from '../api/client'

/** 四根轴。键是后端 `score::dimensions_json` 的键名，一个字都不能差。 */
export type AxisKey = 'ei' | 'sn' | 'tf' | 'jp'

export const AXES: readonly AxisKey[] = ['ei', 'sn', 'tf', 'jp'] as const

/** 每根轴在界面上的标法（如 `EI`），与题库的 `dimension` 同名。 */
export const AXIS_LABEL: Record<AxisKey, string> = {
  ei: 'EI',
  sn: 'SN',
  tf: 'TF',
  jp: 'JP',
}

/** 一根轴的落点：`pole` 是胜出的那一极，`pct` 是它的强度百分比（50–85）。 */
export type Pole = [string, number]

export type Dimensions = Record<AxisKey, Pole>

/** 六种场景下的说话风格，后端按固定顺序给。 */
export interface Behavior {
  answerStyle: string
  casualChat: string
  conflict: string
  creativity: string
  emotion: string
  planning: string
}

/** 16 型之一。字段名与后端 `score::profile_json` 一一对应。 */
export interface MbtiProfile {
  code: string
  name: string
  nickname: string
  summary: string
  descriptors: string
  color: string
  symbol: string
  dimensions: Dimensions
  behavior: Behavior
}

/** 一道题。**没有极性** —— 后端不下发（`a_pole` 属于计分内部）。 */
export interface MbtiQuestion {
  id: number
  dimension: string
  question: string
  option_a: string
  option_b: string
}

export interface MbtiQuestionsResponse {
  questions: MbtiQuestion[]
  /** 至少要答几题才出结果。跟题库一起发，别在前端另写死一个 20。 */
  min_answers: number
}

/** 一次测评的结果。`profile` 为 null 表示这个 code 没有档案。 */
export interface MbtiResult {
  row_id: number
  code: string
  dimensions: Dimensions
  /** null = 还没应用到任何专家。 */
  applied_expert_id: string | null
  created_at: number
  profile: MbtiProfile | null
}

export interface MbtiHistoryResponse {
  history: MbtiResult[]
  current: MbtiResult | null
  /** 保留条数。这是接口的一部分，界面可以据此说明「最近 N 次」。 */
  keep: number
}

export interface MbtiSubmitResponse {
  result: MbtiResult
  profile: MbtiProfile
}

export interface MbtiApplyResponse {
  row_id: number
  code: string
  expert_id: string
  instructions: string
  /** 这次是替换掉已有人格段（true）还是第一次追加（false）。 */
  replaced: boolean
  previous_code: string | null
}

/** 界面语言。后端据此选中文还是英文字段。 */
export type MbtiLang = 'zh' | 'en'

function langQuery(lang: MbtiLang): string {
  return `?lang=${lang}`
}

export function loadMbtiTypes(lang: MbtiLang): Promise<{ types: MbtiProfile[] }> {
  return apiJson<{ types: MbtiProfile[] }>(`/api/mbti/types${langQuery(lang)}`)
}

export function loadMbtiQuestions(lang: MbtiLang): Promise<MbtiQuestionsResponse> {
  return apiJson<MbtiQuestionsResponse>(`/api/mbti/questions${langQuery(lang)}`)
}

export function loadMbtiHistory(lang: MbtiLang): Promise<MbtiHistoryResponse> {
  return apiJson<MbtiHistoryResponse>(`/api/mbti/history${langQuery(lang)}`)
}

export function submitMbti(
  answers: Record<string, 'A' | 'B'>,
  lang: MbtiLang,
): Promise<MbtiSubmitResponse> {
  return apiJson<MbtiSubmitResponse>('/api/mbti/test', {
    method: 'POST',
    body: JSON.stringify({ answers, language: lang }),
  })
}

export function applyMbti(
  rowId: number,
  expertId: string,
  lang: MbtiLang,
): Promise<MbtiApplyResponse> {
  return apiJson<MbtiApplyResponse>('/api/mbti/apply', {
    method: 'POST',
    body: JSON.stringify({ row_id: rowId, expert_id: expertId, language: lang }),
  })
}

/** 六项行为的展示顺序与标签键。顺序即后端 `behavior_items` 的顺序。 */
export const BEHAVIOR_KEYS = [
  'answerStyle',
  'casualChat',
  'conflict',
  'creativity',
  'emotion',
  'planning',
] as const

export type BehaviorKey = (typeof BEHAVIOR_KEYS)[number]

/** 还没答的题。纯函数，判据直接测它。 */
export function firstUnanswered(
  questions: MbtiQuestion[],
  answers: Record<string, 'A' | 'B'>,
): number {
  return questions.findIndex((q) => !answers[String(q.id)])
}

/** 答够了吗。`min` 来自接口，前端不自己定。 */
export function isComplete(
  questions: MbtiQuestion[],
  answers: Record<string, 'A' | 'B'>,
  min: number,
): boolean {
  return questions.length > 0 && Object.keys(answers).length >= min
}