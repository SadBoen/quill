import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { FormEvent, ReactNode } from 'react'
import { useEffect, useState } from 'react'
import { Navigate, Outlet, useLocation } from 'react-router-dom'
import { useTranslation } from 'react-i18next'

import { ApiError, apiJson, getToken, setToken } from '../api/client'
import type { User } from '../api/types'
import { ErrorNotice } from '../components/Page'
import { LanguageSelector } from '../i18n/LanguageSelector'
import { ThemeToggle } from '../theme/ThemeToggle'
import { createInitialAdmin, loadSetupStatus, signIn } from './api'
import { AuthenticatedUserContext, useAuthenticatedUser } from './context'

const CURRENT_USER_KEY = ['current-user'] as const

function currentUserQuery() {
  return {
    queryKey: CURRENT_USER_KEY,
    queryFn: () => apiJson<User>('/api/auth/me'),
    retry: false,
    staleTime: 30_000,
  }
}

export function RequireAuth(): ReactNode {
  const { t } = useTranslation()
  const location = useLocation()
  const query = useQuery(currentUserQuery())

  if (query.isPending) return <FullPageStatus label={t('auth.connecting', { defaultValue: '正在连接…' })} />
  if (query.error instanceof ApiError && (query.error.status === 401 || query.error.status === 403)) {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />
  }
  if (query.isError) {
    return (
      <FullPageStatus
        label={t('auth.unavailable', { defaultValue: '服务不可用' })}
        action={<button onClick={() => void query.refetch()}>{t('common.retry', { defaultValue: '重试' })}</button>}
      />
    )
  }
  return (
    <AuthenticatedUserContext.Provider value={query.data}>
      <Outlet />
    </AuthenticatedUserContext.Provider>
  )
}

export function RequireAdmin(): ReactNode {
  const user = useAuthenticatedUser()
  return user.is_admin ? <Outlet /> : <Navigate to="/chat" replace />
}

/** 登录页的两个入口：账号口令（主）与访问令牌（给 QUILL_TOKENS 用户）。 */
type LoginMethod = 'password' | 'token'

/**
 * 登录页。
 *
 * 三段结构：
 * 1. **首管闸门**：进来先问 `GET /api/setup/status`。`setup_required` 为真时
 *    整页只给「创建初始管理员」表单 —— 此时 users 表为空，没有任何账号可登录，
 *    显示登录表单只会让人对着一个必然 401 的表单点。
 * 2. **账号口令登录**：`POST /api/auth/login` 换会话令牌。失败文案一律走
 *    `ErrorNotice` 渲染服务端 `detail`，前端**不区分**「用户不存在」和「口令错误」
 *    —— 后端对此返回逐字相同的 401，任何前端区分都会把它变成用户名枚举器。
 * 3. **访问令牌登录**：原有入口原样保留，`QUILL_TOKENS` 用户靠它进站。
 */
export function AuthPage(): ReactNode {
  const { t } = useTranslation()
  const location = useLocation()
  const queryClient = useQueryClient()
  const currentUser = useQuery(currentUserQuery())
  const setup = useQuery({ queryKey: ['setup-status'], queryFn: loadSetupStatus, retry: false, staleTime: 30_000 })

  const [method, setMethod] = useState<LoginMethod>('password')
  const [token, setTokenDraft] = useState(() => getToken())
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<unknown>(null)
  const [busy, setBusy] = useState(false)
  // 429 之后服务端要求静默的秒数：倒计时期间禁用提交，且不自己发请求去轮询。
  const [cooldown, setCooldown] = useState(0)

  useEffect(() => {
    if (cooldown <= 0) return
    const timer = window.setInterval(() => setCooldown((left) => Math.max(0, left - 1)), 1000)
    return () => window.clearInterval(timer)
  }, [cooldown])

  if (currentUser.isSuccess) return <Navigate to="/chat" replace />

  /** 登录成功后的统一收尾：写令牌 → 重新确认身份 → 跳回原去处。 */
  const finishLogin = async (accessToken: string): Promise<void> => {
    setToken(accessToken)
    queryClient.removeQueries({ queryKey: CURRENT_USER_KEY })
    const requested = (location.state as { from?: string } | null)?.from
    await queryClient.fetchQuery(currentUserQuery())
    window.history.replaceState({}, '', requested && requested.startsWith('/') ? requested : '/chat')
    void queryClient.invalidateQueries({ queryKey: CURRENT_USER_KEY })
  }

  /** 429 的 Retry-After 落进倒计时，按钮在此期间禁用。 */
  const reportError = (caught: unknown): void => {
    setError(caught)
    if (caught instanceof ApiError && caught.status === 429 && caught.retryAfterSeconds !== null) {
      setCooldown(caught.retryAfterSeconds)
    }
  }

  const submitPassword = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault()
    const name = username.trim()
    if (!name) {
      setError(new Error(t('auth.usernameRequired', { defaultValue: '请填写用户名。' })))
      return
    }
    if (!password) {
      setError(new Error(t('auth.passwordRequired', { defaultValue: '请填写口令。' })))
      return
    }
    setBusy(true)
    setError(null)
    try {
      const session = await signIn({ username: name, password })
      setPassword('')
      await finishLogin(session.access_token)
    } catch (caught) {
      setPassword('')
      reportError(caught)
    } finally {
      setBusy(false)
    }
  }

  const submitToken = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault()
    const trimmed = token.trim()
    if (!trimmed) {
      setError(new Error(t('auth.tokenRequired', { defaultValue: '请填写访问令牌。' })))
      return
    }
    setError(null)
    setToken(trimmed)
    queryClient.removeQueries({ queryKey: CURRENT_USER_KEY })
    const requested = (location.state as { from?: string } | null)?.from
    void queryClient
      .fetchQuery(currentUserQuery())
      .then(() => {
        window.history.replaceState({}, '', requested && requested.startsWith('/') ? requested : '/chat')
        void queryClient.invalidateQueries({ queryKey: CURRENT_USER_KEY })
      })
      .catch((caught) => {
        setToken('')
        setError(caught)
      })
  }

  // 切换登录方式时清掉**另一种**方式的输入：两个表单共用一个提交区，
  // 残留的半截口令留在 DOM 里既没有用也不安全。
  const switchMethod = (next: LoginMethod): void => {
    if (next === method) return
    setMethod(next)
    setError(null)
    if (next === 'token') {
      setUsername('')
      setPassword('')
    } else {
      setTokenDraft('')
    }
  }

  const shell = (card: ReactNode): ReactNode => (
    <main className="auth-page">
      <div className="auth-theme"><LanguageSelector /><ThemeToggle compact /></div>
      <section className="auth-brand" aria-label={t('auth.introLabel', { defaultValue: 'Quill 简介' })}>
        <Brand />
        <div className="auth-promise">
          <span className="eyebrow">{t('auth.eyebrow', { defaultValue: '本地智能体 · 对话即入口' })}</span>
          <h1>{t('auth.hero', { defaultValue: '一个能记住上下文的对话入口' })}</h1>
          <p>{t('auth.heroDescription', { defaultValue: '用账号或访问令牌连接你的 Quill 服务，然后直接开始对话。' })}</p>
        </div>
      </section>
      <section className="auth-panel">{card}</section>
    </main>
  )

  // ---- 首管引导：users 表为空，只给建号表单，不显示任何登录入口 ----
  if (setup.isPending) {
    return <FullPageStatus label={t('auth.setupChecking', { defaultValue: '正在检查是否需要初始化…' })} />
  }
  // 查不到 setup 状态时**不猜、也不挡路**：继续渲染登录表单，只多给一条
  // 提示和「重试」。绝大多数实例早就初始化过，为了一次瞬时 500 把所有人
  // 挡在门外是更糟的选择。
  //
  // 早先这里渲染的是一张「只有标题 + 重试按钮、没有任何输入框」的卡片，
  // 注释写着「继续给登录表单」而代码并没有 —— 结果是用户撞到一次 500 就
  // 既登不进去、又看不到任何表单。现在按注释的意图真正给表单。
  // 探测失败时 `setup.data` 是 undefined，直接读 `.setup_required` 会崩 ——
  // 读取必须判空，否则「优雅降级」本身变成白屏。
  const setupProbeFailed = setup.isError
  if (!setupProbeFailed && setup.data?.setup_required) {
    return <SetupGate onDone={() => { void queryClient.invalidateQueries({ queryKey: ['setup-status'] }) }} />
  }

  const passwordPanel = (
    <div className="auth-card">
      <span className="auth-kicker">QUILL</span>
      <h2>{t('auth.welcome', { defaultValue: '登录 Quill' })}</h2>
      <p className="auth-subtitle">
        {method === 'password'
          ? t('auth.passwordSubtitle', { defaultValue: '用你的账号登录。' })
          : t('auth.loginSubtitle', { defaultValue: '令牌由服务端 QUILL_TOKENS 配置，形如 dev-token。' })}
      </p>
      {setupProbeFailed ? (
        <div className="form-notice" role="status">
          <p>{t('auth.setupProbeFailed', {
            defaultValue: '没能确认这个实例是否已完成初始化，因此这里只显示登录表单。',
          })}</p>
          <button type="button" onClick={() => void setup.refetch()}>
            {t('common.retry', { defaultValue: '重试' })}
          </button>
        </div>
      ) : null}
      <div className="auth-switch" role="tablist" aria-label={t('auth.methodLabel', { defaultValue: '登录方式' })}>        <button
          type="button"
          role="tab"
          aria-selected={method === 'password'}
          className="text-button"
          onClick={() => switchMethod('password')}
        >
          {t('auth.methodPassword', { defaultValue: '账号登录' })}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={method === 'token'}
          className="text-button"
          onClick={() => switchMethod('token')}
        >
          {t('auth.methodToken', { defaultValue: '访问令牌' })}
        </button>
      </div>
      {method === 'password' ? (
        <form onSubmit={(event) => void submitPassword(event)} className="auth-form">
          <label>
            {t('auth.username', { defaultValue: '用户名' })}
            <input
              name="username"
              autoComplete="username"
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              required
            />
          </label>
          <label>
            {t('auth.password', { defaultValue: '口令' })}
            <input
              name="password"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              required
            />
            <small>{t('auth.passwordHint', { defaultValue: '口令至少 12 个字符。' })}</small>
          </label>
          {error ? <ErrorNotice error={error} /> : null}
          {cooldown > 0 ? (
            <p className="form-notice" role="status">
              {t('auth.cooldown', { seconds: cooldown, defaultValue: '尝试过于频繁，请在 {{seconds}} 秒后再试。' })}
            </p>
          ) : null}
          <button type="submit" className="primary-button auth-submit" disabled={busy || cooldown > 0}>
            {t('auth.login', { defaultValue: '连接' })}
          </button>
        </form>
      ) : (
        <form onSubmit={submitToken} className="auth-form">
          <label>
            {t('auth.token', { defaultValue: '访问令牌' })}
            <input name="token" autoComplete="off" value={token} onChange={(event) => setTokenDraft(event.target.value)} required />
          </label>
          {error ? <ErrorNotice error={error} /> : null}
          <button type="submit" className="primary-button auth-submit">
            {t('auth.connect', { defaultValue: '连接' })}
          </button>
        </form>
      )}
    </div>
  )

  return shell(passwordPanel)
}

/**
 * 首管引导表单：`POST /api/setup/initial-admin`。
 *
 * 只有 users 表为空时才会被渲染（`setup_required` 的口径与 `create_first_owner`
 * 同源），所以这里指向的按钮不会是一个必然 409 的按钮。
 *
 * **注册保持关闭**：这个表单是「首次安装向导」，不是注册入口 ——
 * 建完即焚，之后再访问只会拿到 409。
 */
function SetupGate({ onDone }: { onDone: () => void }): ReactNode {
  const { t } = useTranslation()
  const [username, setUsername] = useState('')
  const [displayName, setDisplayName] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<unknown>(null)
  const [busy, setBusy] = useState(false)

  const submit = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault()
    const name = username.trim()
    if (!name) {
      setError(new Error(t('auth.usernameRequired', { defaultValue: '请填写用户名。' })))
      return
    }
    if (!password) {
      setError(new Error(t('auth.passwordRequired', { defaultValue: '请填写口令。' })))
      return
    }
    setBusy(true)
    setError(null)
    try {
      // display_name 留空就不发：服务端 only_keys 白名单多传字段会直接 400。
      await createInitialAdmin({ username: name, password, display_name: displayName.trim() || undefined })
      onDone()
    } catch (caught) {
      setError(caught)
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="auth-page">
      <div className="auth-theme"><LanguageSelector /><ThemeToggle compact /></div>
      <section className="auth-brand" aria-label={t('auth.introLabel', { defaultValue: 'Quill 简介' })}>
        <Brand />
        <div className="auth-promise">
          <span className="eyebrow">{t('auth.eyebrow', { defaultValue: '本地智能体 · 对话即入口' })}</span>
          <h1>{t('auth.hero', { defaultValue: '一个能记住上下文的对话入口' })}</h1>
          <p>{t('auth.heroDescription', { defaultValue: '用账号或访问令牌连接你的 Quill 服务，然后直接开始对话。' })}</p>
        </div>
      </section>
      <section className="auth-panel">
        <div className="auth-card">
          <span className="auth-kicker">QUILL</span>
          <h2>{t('auth.setupTitle', { defaultValue: '创建初始管理员' })}</h2>
          <p className="auth-subtitle">
            {t('auth.setupSubtitle', {
              defaultValue: '这个实例还没有任何账号。创建第一个管理员后，注册通道会永久关闭。',
            })}
          </p>
          <form onSubmit={(event) => void submit(event)} className="auth-form">
            <label>
              {t('auth.username', { defaultValue: '用户名' })}
              <input
                name="username"
                autoComplete="username"
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                required
              />
              <small>{t('auth.usernameHint', { defaultValue: '3~32 个字母、数字或下划线。' })}</small>
            </label>
            <label>
              {t('auth.displayName', { defaultValue: '显示名' })}
              <input
                name="display_name"
                autoComplete="nickname"
                value={displayName}
                onChange={(event) => setDisplayName(event.target.value)}
              />
              <small>{t('auth.displayNameHint', { defaultValue: '留空就用用户名。' })}</small>
            </label>
            <label>
              {t('auth.password', { defaultValue: '口令' })}
              <input
                name="password"
                type="password"
                autoComplete="new-password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                required
              />
              <small>{t('auth.passwordHint', { defaultValue: '口令至少 12 个字符。' })}</small>
            </label>
            {error ? <ErrorNotice error={error} /> : null}
            <button type="submit" className="primary-button auth-submit" disabled={busy}>
              {t('auth.setupSubmit', { defaultValue: '创建管理员' })}
            </button>
          </form>
        </div>
      </section>
    </main>
  )
}

export function Brand(): ReactNode {
  const { t } = useTranslation()
  return (
    <div className="brand">
      <span className="brand-symbol" aria-hidden="true">Q</span>
      <span>
        <strong className="brand-name">Quill</strong>
        <small className="brand-caption">{t('brand.caption', { defaultValue: '本地智能体' })}</small>
      </span>
    </div>
  )
}

function FullPageStatus({ label, action }: { label: string; action?: ReactNode }): ReactNode {
  return (
    <main className="full-page-status">
      <Brand />
      <p>{label}</p>
      {action}
    </main>
  )
}
