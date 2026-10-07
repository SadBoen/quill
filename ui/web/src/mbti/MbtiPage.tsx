import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice, PageHeader } from '../components/Page'
import { listExperts, EXPERTS_KEY } from '../experts/api'
import {
  applyMbti,
  AXES,
  AXIS_LABEL,
  BEHAVIOR_KEYS,
  firstUnanswered,
  isComplete,
  loadMbtiHistory,
  loadMbtiQuestions,
  submitMbti,
  type MbtiLang,
  type MbtiProfile,
  type MbtiQuestion,
  type MbtiResult,
} from './api'
import './mbti.css'

const HISTORY_KEY = ['mbti', 'history'] as const
const QUESTIONS_KEY = ['mbti', 'questions'] as const

type Stage = 'view' | 'quiz' | 'result'
type Answers = Record<string, 'A' | 'B'>

/** 界面语言 → 后端要的 lang。 */
function langOf(i18nLanguage: string): MbtiLang {
  return i18nLanguage.startsWith('en') ? 'en' : 'zh'
}

export function MbtiPage({
  embedded = false,
  expertId = '',
}: {
  embedded?: boolean
  /** 个性化页当前选中的专家。给了就作为「应用到」的默认目标。 */
  expertId?: string
}): ReactNode {
  const { t, i18n } = useTranslation()
  const lang = langOf(i18n.language)
  const qc = useQueryClient()

  const [stage, setStage] = useState<Stage>('view')
  const [answers, setAnswers] = useState<Answers>({})
  const [cursor, setCursor] = useState(0)
  const [fresh, setFresh] = useState<MbtiResult | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  const history = useQuery({
    queryKey: [...HISTORY_KEY, lang],
    queryFn: () => loadMbtiHistory(lang),
  })
  // 题库只有 28 条、只读、不随人变 —— 一直开着，进测评时立刻就有题。
  const questions = useQuery({
    queryKey: [...QUESTIONS_KEY, lang],
    queryFn: () => loadMbtiQuestions(lang),
  })

  const submit = useMutation({
    mutationFn: () => submitMbti(answers, lang),
    onSuccess: (data) => {
      setFresh(data.result)
      setStage('result')
      setNotice(null)
      void qc.invalidateQueries({ queryKey: HISTORY_KEY })
    },
  })

  const shown = stage === 'result' && fresh ? fresh : (history.data?.current ?? null)
  const asked = Object.keys(answers).length

  const startQuiz = (): void => {
    // 上一次的作答必须清干净，否则「还剩几题」是假的。
    setAnswers({})
    setCursor(0)
    setNotice(null)
    setFresh(null)
    setStage('quiz')
  }

  return (
    // 嵌在个性化页的页签里时不带自己的页头 —— 上面已经有「个性化 / 人格」了，
    // 同一个页面上出现两个标题会让人以为进了两个地方。
    // 也不套 page-scroll：外层已经滚了，套两层会出现两条滚动条。
    <div className={embedded ? 'mbti-page mbti-page-embedded' : 'page-scroll mbti-page'}>
      {embedded ? null : (
        <PageHeader
          title={t('mbti.title', { defaultValue: '人格' })}
          description={t('mbti.description', {
            defaultValue:
              '28 道题、四个维度，算出一个人格类型。选一个专家，就能把这套说话风格写进它的人格正文。',
          })}
        />
      )}

      {history.isPending ? (
        <p className="page-status">{t('common.loading', { defaultValue: '加载中…' })}</p>
      ) : null}
      {history.isError ? <ErrorNotice error={history.error} /> : null}

      {history.isSuccess ? (
        <div className="mbti-stack">
          {stage === 'quiz' ? (
            <Quiz
              questions={questions.data?.questions ?? []}
              minAnswers={questions.data?.min_answers ?? 0}
              answers={answers}
              cursor={cursor}
              asked={asked}
              loading={questions.isPending}
              error={questions.error}
              submitting={submit.isPending}
              submitError={submit.error}
              notice={notice}
              onAnswer={(id, choice) => {
                setAnswers((prev) => ({ ...prev, [String(id)]: choice }))
                setNotice(null)
              }}
              onJump={setCursor}
              onQuit={() => {
                setStage('view')
                setNotice(null)
              }}
              onSubmit={() => {
                const list = questions.data?.questions ?? []
                const gap = list.length - asked
                if (gap > 0) {
                  // 别只说一句「还有 N 题没答」：直接跳到第一道没答的题，
                  // 否则用户得自己在答题卡里翻。
                  setCursor(Math.max(0, firstUnanswered(list, answers)))
                  setNotice(
                    t('mbti.remaining', {
                      count: gap,
                      defaultValue: `还有 {{count}} 题没答，已跳到第一道没答的。`,
                    }),
                  )
                  return
                }
                submit.mutate()
              }}
            />
          ) : null}

          {stage !== 'quiz' ? (
            <>
              {shown ? (
                <ResultCard result={shown} />
              ) : (
                <p className="form-notice">
                  {t('mbti.noneYet', {
                    defaultValue: '还没测过。点下面那条从头开始 —— 28 道题，没有对错，按第一反应选。',
                  })}
                </p>
              )}

              <div className="mbti-actions">
                <button type="button" className="btn" onClick={startQuiz}>
                  {shown
                    ? t('mbti.retake', { defaultValue: '重新测一次' })
                    : t('mbti.start', { defaultValue: '开始测评' })}
                </button>
              </div>

              {shown ? <ApplyPanel result={shown} lang={lang} expertId={expertId} /> : null}

              {history.data && history.data.history.length > 1 ? (
                <HistoryList rows={history.data.history} keep={history.data.keep} />
              ) : null}
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

// ------------------------------------------------------------------ 答题

function Quiz({
  questions,
  minAnswers,
  answers,
  cursor,
  asked,
  loading,
  error,
  submitting,
  submitError,
  notice,
  onAnswer,
  onJump,
  onQuit,
  onSubmit,
}: {
  questions: MbtiQuestion[]
  minAnswers: number
  answers: Answers
  cursor: number
  asked: number
  loading: boolean
  error: unknown
  submitting: boolean
  submitError: unknown
  notice: string | null
  onAnswer: (id: number, choice: 'A' | 'B') => void
  onJump: (index: number) => void
  onQuit: () => void
  onSubmit: () => void
}): ReactNode {
  const { t } = useTranslation()
  const total = questions.length
  const current = questions[Math.min(cursor, Math.max(0, total - 1))]
  const complete = isComplete(questions, answers, minAnswers)
  const pct = total ? Math.round((asked / total) * 100) : 0

  if (loading) {
    return <p className="page-status">{t('common.loading', { defaultValue: '加载中…' })}</p>
  }
  if (error) return <ErrorNotice error={error} />

  return (
    <Card title={t('mbti.quizTitle', { defaultValue: '答题' })}>
      <div
        className="mbti-progress"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={total}
        aria-valuenow={asked}
        aria-label={t('mbti.progress', { defaultValue: '答题进度' })}
      >
        <div className="mbti-progress-fill" style={{ width: `${pct}%` }} />
      </div>
      <p className="mbti-progress-text">
        {asked} / {total}
      </p>

      {/* 答题卡：漏题时唯一能一眼看出「哪几道没做」的地方。 */}
      <ol className="mbti-sheet" aria-label={t('mbti.sheet', { defaultValue: '答题卡' })}>
        {questions.map((q, idx) => {
          const done = Boolean(answers[String(q.id)])
          return (
            <li key={q.id}>
              <button
                type="button"
                className={`mbti-dot${done ? ' mbti-dot-done' : ''}${idx === cursor ? ' mbti-dot-current' : ''}`}
                onClick={() => onJump(idx)}
                aria-current={idx === cursor ? 'true' : undefined}
                aria-label={`${idx + 1}${done ? '' : ` (${t('mbti.unanswered', { defaultValue: '未答' })})`}`}
              >
                {idx + 1}
              </button>
            </li>
          )
        })}
      </ol>

      {notice ? <p className="form-notice">{notice}</p> : null}

      {current ? (
        <div className="mbti-question">
          <h3 className="mbti-question-text">{current.question}</h3>
          <div className="mbti-options">
            <button
              type="button"
              className={`mbti-option${answers[String(current.id)] === 'A' ? ' mbti-option-picked' : ''}`}
              onClick={() => onAnswer(current.id, 'A')}
            >
              <span className="mbti-option-letter" aria-hidden="true">
                A
              </span>
              <span>{current.option_a}</span>
            </button>
            <button
              type="button"
              className={`mbti-option${answers[String(current.id)] === 'B' ? ' mbti-option-picked' : ''}`}
              onClick={() => onAnswer(current.id, 'B')}
            >
              <span className="mbti-option-letter" aria-hidden="true">
                B
              </span>
              <span>{current.option_b}</span>
            </button>
          </div>
        </div>
      ) : (
        <p className="form-notice">
          {t('mbti.noQuestions', { defaultValue: '题库是空的，请联系管理员。' })}
        </p>
      )}

      {submitError ? <ErrorNotice error={submitError} /> : null}

      <div className="mbti-nav">
        <button
          type="button"
          className="btn"
          onClick={() => onQuit()}
        >
          {t('mbti.quit', { defaultValue: '退出' })}
        </button>
        <button
          type="button"
          className="btn"
          onClick={() => onJump(Math.max(0, cursor - 1))}
          disabled={cursor <= 0}
        >
          {t('mbti.prev', { defaultValue: '上一题' })}
        </button>
        <button
          type="button"
          className="btn btn-primary"
          onClick={onSubmit}
          disabled={submitting}
        >
          {complete
            ? t('mbti.viewResult', { defaultValue: '看结果' })
            : t('mbti.submitHint', {
                count: asked,
                min: minAnswers,
                defaultValue: `交卷（至少 {{min}} 题，现在 {{count}}）`,
              })}
        </button>
        <button
          type="button"
          className="btn"
          onClick={() => onJump(Math.min(total - 1, cursor + 1))}
          disabled={total === 0 || cursor >= total - 1}
        >
          {t('mbti.next', { defaultValue: '下一题' })}
        </button>
      </div>
    </Card>
  )
}

// ------------------------------------------------------------------ 结果

function ResultCard({ result }: { result: MbtiResult }): ReactNode {
  const { t } = useTranslation()
  const profile = result.profile
  if (!profile) {
    // 档案缺失是后端的 bug，不是用户的错。说清楚，别画个空壳。
    return (
      <Card title={t('mbti.title', { defaultValue: '人格' })}>
        <p className="form-notice">
          {t('mbti.noProfile', {
            code: result.code,
            defaultValue: '这个类型（{{code}}）没有对应的档案。',
          })}
        </p>
      </Card>
    )
  }
  return <ProfileCard profile={profile} result={result} />
}

function ProfileCard({
  profile,
  result,
}: {
  profile: MbtiProfile
  result: MbtiResult
}): ReactNode {
  const { t } = useTranslation()
  return (
    <Card title={t('mbti.resultTitle', { defaultValue: '你的类型' })}>
      <div className="mbti-hero" style={{ borderColor: profile.color }}>
        <span className="mbti-symbol" style={{ color: profile.color }} aria-hidden="true">
          {profile.symbol}
        </span>
        <div>
          <div className="mbti-code" style={{ color: profile.color }}>
            {profile.code}
          </div>
          <div className="mbti-name">
            {profile.name}
            {profile.nickname ? (
              <span className="mbti-nickname">「{profile.nickname}」</span>
            ) : null}
          </div>
          <p className="mbti-summary">{profile.summary}</p>
          <p className="mbti-descriptors">{profile.descriptors}</p>
        </div>
      </div>

      <ul className="mbti-axes">
        {AXES.map((axis) => {
          const [pole, pct] = result.dimensions[axis]
          return (
            <li key={axis} className="mbti-axis-row">
              <span className="mbti-axis-label">{AXIS_LABEL[axis]}</span>
              <span className="mbti-axis-track">
                <span
                  className="mbti-axis-fill"
                  style={{ width: `${pct}%`, background: profile.color }}
                />
              </span>
              <span className="mbti-axis-value">
                {pole} {pct}%
              </span>
            </li>
          )
        })}
      </ul>

      <dl className="mbti-behavior">
        {BEHAVIOR_KEYS.map((key) => (
          <div key={key} className="mbti-behavior-row">
            <dt>{t(`mbti.behavior.${key}`, { defaultValue: key })}</dt>
            <dd>{profile.behavior[key]}</dd>
          </div>
        ))}
      </dl>
    </Card>
  )
}

// ------------------------------------------------------------------ 应用

function ApplyPanel({
  result,
  lang,
  expertId: preset,
}: {
  result: MbtiResult
  lang: MbtiLang
  /** 个性化页当前选中的专家，优先于已应用的那个当默认值。 */
  expertId?: string
}): ReactNode {
  const { t } = useTranslation()
  const qc = useQueryClient()
  // 默认目标：先看已经应用过谁，没有就用页面当前选中的那个专家。
  // 这么排是因为「已经应用过」是既成事实，而页面选中的是当前意图 ——
  // 用户切了专家就是要改那个。
  const [expertId, setExpertId] = useState(result.applied_expert_id || preset || '')
  const experts = useQuery({ queryKey: EXPERTS_KEY, queryFn: listExperts })

  const apply = useMutation({
    mutationFn: () => applyMbti(result.row_id, expertId, lang),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: HISTORY_KEY })
    },
  })

  return (
    <Card title={t('mbti.applyTitle', { defaultValue: '把它用起来' })}>
      <p className="form-notice">
        {t('mbti.applyHint', {
          defaultValue:
            '人格会写进下面这个专家的人格正文（它本来就决定这个智能体怎么说话）。同一段只会有一份，重复点不会越叠越长。',
        })}
      </p>

      {result.applied_expert_id ? (
        <p className="mbti-applied">
          {t('mbti.alreadyApplied', {
            expert: result.applied_expert_id,
            defaultValue: '当前已应用在专家 {{expert}} 上。',
          })}
        </p>
      ) : null}

      {experts.isPending ? (
        <p className="page-status">{t('common.loading', { defaultValue: '加载中…' })}</p>
      ) : null}
      {experts.isError ? <ErrorNotice error={experts.error} /> : null}
      {experts.isSuccess && experts.data.length === 0 ? (
        <p className="form-notice">
          {t('mbti.noExperts', {
            defaultValue: '还没有任何专家。先到「专家」页建一个，再回来用人格。',
          })}
        </p>
      ) : null}

      {experts.isSuccess && experts.data.length > 0 ? (
        <div className="mbti-apply-row">
          <label className="mbti-apply-label" htmlFor="mbti-expert">
            {t('mbti.chooseExpert', { defaultValue: '写到哪个专家' })}
          </label>
          <select
            id="mbti-expert"
            className="input"
            value={expertId}
            onChange={(e) => setExpertId(e.target.value)}
          >
            <option value="">{t('mbti.pickOne', { defaultValue: '请选择…' })}</option>
            {experts.data.map((e) => (
              <option key={e.id} value={e.id}>
                {e.display_name}（{e.id}）
              </option>
            ))}
          </select>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => apply.mutate()}
            disabled={!expertId || apply.isPending}
          >
            {apply.isPending
              ? t('mbti.applying', { defaultValue: '写入中…' })
              : t('mbti.apply', { defaultValue: '应用这个人格' })}
          </button>
        </div>
      ) : null}

      {apply.isError ? <ErrorNotice error={apply.error} /> : null}
      {apply.isSuccess ? (
        <p className="mbti-applied">
          {t('mbti.applied', {
            expert: apply.data.expert_id,
            code: apply.data.code,
            defaultValue: '已把 {{code}} 的说话风格写进专家 {{expert}}。',
          })}
        </p>
      ) : null}
    </Card>
  )
}

// ------------------------------------------------------------------ 历史

function HistoryList({ rows, keep }: { rows: MbtiResult[]; keep: number }): ReactNode {
  const { t, i18n } = useTranslation()
  return (
    <Card title={t('mbti.historyTitle', { defaultValue: '历史' })}>
      <p className="form-notice">
        {t('mbti.historyHint', {
          keep,
          defaultValue: '最近 {{keep}} 次。上面显示的是最新一次。',
        })}
      </p>
      <ul className="mbti-history">
        {rows.map((r) => (
          <li key={r.row_id} className="mbti-history-row">
            <span
              className="mbti-history-code"
              style={{ color: r.profile?.color ?? 'inherit' }}
            >
              {r.profile?.symbol ? `${r.profile.symbol} ` : ''}
              {r.code}
            </span>
            <span className="mbti-history-name">{r.profile?.name ?? ''}</span>
            <span className="mbti-history-time">
              {new Date(r.created_at).toLocaleString(
                i18n.language.startsWith('en') ? 'en-US' : 'zh-CN',
              )}
            </span>
            <span className="mbti-history-applied">
              {r.applied_expert_id
                ? t('mbti.appliedTo', {
                    expert: r.applied_expert_id,
                    defaultValue: '已用于 {{expert}}',
                  })
                : t('mbti.notApplied', { defaultValue: '未应用' })}
            </span>
          </li>
        ))}
      </ul>
    </Card>
  )
}