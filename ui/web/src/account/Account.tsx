import { useQuery, useQueryClient } from '@tanstack/react-query'
import { type ReactNode, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Link, useNavigate } from 'react-router-dom'

import { endSession } from '../auth/api'
import { Card, ErrorNotice, PageHeader, StatusBadge } from '../components/Page'
import { LanguageSelector } from '../i18n/LanguageSelector'
import { ThemeToggle } from '../theme/ThemeToggle'
import { forgetLocalToken, loadAccount, loadServerVersion } from './api'

const PROFILE_PATCH_ROUTE = 'PATCH /api/me'
const SELF_DELETE_ROUTE = 'DELETE /api/me'

export function AccountPage(): ReactNode {
  const { t } = useTranslation()
  const client = useQueryClient()
  const navigate = useNavigate()
  const [timezone, setTimezone] = useState(detectTimezone)
  const [detectedTimezone] = useState(detectTimezone)
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [loggingOut, setLoggingOut] = useState(false)
  const [logoutError, setLogoutError] = useState<unknown>(null)
  const account = useQuery({ queryKey: ['account'], queryFn: loadAccount, staleTime: 30_000 })
  const version = useQuery({ queryKey: ['server-version'], queryFn: loadServerVersion, staleTime: 300_000 })
  const user = account.data

  /**
   * 退出登录：先让服务端吊销会话令牌，再清本机令牌并回登录页。
   *
   * 两种结果都算登出成功：`revoked:true` 是吊销了一个会话；`revoked:false` 表示
   * 这个令牌本来就不在有效会话里（已经登出过，或它是 `QUILL_TOKENS` 的环境变量
   * 令牌 —— 那种令牌不受登出影响）。端点幂等，所以**不把 `revoked:false` 当错误**。
   *
   * 网络失败仍然清本机令牌并回登录页：用户要的是「离开这台机器」，
   * 停在登录态等他重试反而更糟。
   */
  const logout = async (): Promise<void> => {
    setLoggingOut(true)
    setLogoutError(null)
    try {
      await endSession()
    } catch (caught) {
      setLogoutError(caught)
    } finally {
      forgetLocalToken()
      client.clear()
      setLoggingOut(false)
      navigate('/login', { replace: true })
    }
  }

  return (
    <div className="page-scroll">
      <PageHeader
        eyebrow={t('account.eyebrow', { defaultValue: '账户' })}
        title={t('account.title', { defaultValue: '账户设置' })}
        description={t('account.description', { defaultValue: '查看当前身份、语言与主题偏好。quill 用访问令牌鉴权，没有邮箱密码账号。' })}
      />
      <ErrorNotice error={account.error} />
      <div className="settings-stack">
        <Card title={t('account.profile', { defaultValue: '身份' })} description={t('account.profileDescription', { defaultValue: '来自 GET /api/auth/me。' })}>
          {account.isPending ? <p className="page-status">{t('common.loading', { defaultValue: '加载中…' })}</p> : null}
          {user ? (
            <dl className="detail-grid">
              <div>
                <dt>{t('account.userId', { defaultValue: '用户 ID' })}</dt>
                <dd><code>{user.id}</code></dd>
              </div>
              <div>
                <dt>{t('account.username', { defaultValue: '用户名' })}</dt>
                <dd>{user.username}</dd>
              </div>
              <div>
                <dt>{t('account.role', { defaultValue: '角色' })}</dt>
                <dd>
                  <StatusBadge tone={user.is_admin ? 'success' : 'neutral'}>
                    {user.is_admin
                      ? t('admin.adminRole', { defaultValue: '管理员' })
                      : t('admin.memberRole', { defaultValue: '成员' })}
                  </StatusBadge>
                </dd>
              </div>
              <div>
                <dt>{t('account.approvalMode', { defaultValue: '审批模式' })}</dt>
                <dd>{user.approval_mode}</dd>
              </div>
              <div>
                <dt>{t('account.serverVersion', { defaultValue: '服务端版本' })}</dt>
                <dd>{version.data?.version ?? '—'}</dd>
              </div>
            </dl>
          ) : null}
          <p className="field-help full-row">
            {t('account.profileReadOnly', {
              defaultValue: 'quill 目前没有可写的账号资料接口：{{route}} 未登记。下一步：注册 {{route}}，再把这里的只读字段改成可编辑表单。',
              route: PROFILE_PATCH_ROUTE,
            })}
          </p>
        </Card>
        <Card title={t('account.preferences', { defaultValue: '偏好' })} description={t('account.preferencesDescription', { defaultValue: '语言与主题保存在本机浏览器，立即生效。' })}>
          <div className="account-preferences">
            <div className="preference-setting">
              <div className="preference-setting-copy">
                <strong>{t('language.label', { defaultValue: '语言' })}</strong>
                <small>{t('account.languageDescription', { defaultValue: '切换界面语言。' })}</small>
              </div>
              <LanguageSelector />
            </div>
            <div className="preference-setting">
              <div className="preference-setting-copy">
                <strong>{t('account.appearance', { defaultValue: '外观' })}</strong>
                <small>{t('account.appearanceDescription', { defaultValue: '跟随系统或固定深浅色。' })}</small>
              </div>
              <ThemeToggle />
            </div>
            <div className="preference-setting preference-timezone">
              <div className="preference-setting-copy">
                <strong>{t('account.timezone', { defaultValue: '时区' })}</strong>
                <small>{t('account.timezoneDescription', { defaultValue: '本机检测到的时区，仅用于本机显示。' })}</small>
              </div>
              <div>
                <label>
                  <span className="sr-only">{t('account.timezone', { defaultValue: '时区' })}</span>
                  <input
                    name="timezone"
                    aria-label={t('account.timezone', { defaultValue: '时区' })}
                    value={timezone}
                    autoComplete="off"
                    maxLength={64}
                    onChange={(event) => setTimezone(event.target.value)}
                  />
                </label>
                {detectedTimezone && detectedTimezone !== timezone ? (
                  <button type="button" className="text-button" onClick={() => setTimezone(detectedTimezone)}>
                    {t('account.useDetectedTimezone', {
                      timezone: detectedTimezone,
                      defaultValue: '使用检测到的时区：{{timezone}}',
                    })}
                  </button>
                ) : null}
              </div>
            </div>
          </div>
          <p className="field-help full-row">
            {t('account.timezoneNotSaved', {
              defaultValue: '时区不会上传：quill 尚未提供保存用户设置的接口。下一步：{{route}} 落地后再开放「保存时区」。',
              route: PROFILE_PATCH_ROUTE,
            })}
          </p>
        </Card>
        <Card title={t('account.agentFiles', { defaultValue: '智能体文件' })} description={t('account.agentFilesDescription', { defaultValue: '跳到工作区与资料库继续编辑。' })}>
          <div className="form-actions">
            <Link className="secondary-button" to="/workspace">{t('account.editWorkspace', { defaultValue: '打开工作区' })}</Link>
            <Link className="secondary-button" to="/memory">{t('account.editMemory', { defaultValue: '打开资料库' })}</Link>
          </div>
        </Card>
        <Card title={t('account.session', { defaultValue: '会话' })} description={t('account.sessionDescription', { defaultValue: '令牌保存在本机浏览器（localStorage quill-token）。' })}>
          <div className="form-actions">
            <button className="secondary-button" onClick={() => void logout()} disabled={loggingOut}>
              {loggingOut ? t('account.loggingOut', { defaultValue: '正在退出…' }) : t('account.logout', { defaultValue: '退出登录' })}
            </button>
          </div>
          {logoutError ? <ErrorNotice error={logoutError} /> : null}
          <p className="field-help full-row">
            {t('account.logoutHelp', {
              defaultValue: '退出会调用 POST /api/auth/logout 吊销服务端会话。注意：QUILL_TOKENS 的环境变量令牌不受登出影响，要收回得改配置并重启。',
            })}
          </p>
        </Card>
        <Card title={t('account.deleteTitle', { defaultValue: '删除账号' })} description={t('account.deleteDescription', { defaultValue: 'quill 的账号就是 QUILL_TOKENS 里的一条配置。' })} tone="danger">
          {confirmDelete ? (
            <div className="danger-actions">
              <span>{t('account.notDeletableHere', { defaultValue: '这里无法删除账号：请到服务端 QUILL_TOKENS 删除这一行后重启。' })}</span>
              <button className="secondary-button" onClick={() => setConfirmDelete(false)}>{t('common.cancel', { defaultValue: '取消' })}</button>
            </div>
          ) : (
            <button className="danger-button" onClick={() => setConfirmDelete(true)}>{t('account.deleteMine', { defaultValue: '删除我的账号' })}</button>
          )}
          <p className="field-help full-row">
            {t('account.deleteHelp', {
              defaultValue: '删除入口暂不可用：{{route}} 未在 quill 登记。下一步：若需要自助注销，再实现 {{route}}。',
              route: SELF_DELETE_ROUTE,
            })}
          </p>
        </Card>
      </div>
    </div>
  )
}

function detectTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}
