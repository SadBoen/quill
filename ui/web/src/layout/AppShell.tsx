import { useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { Link, NavLink, Outlet } from 'react-router-dom'
import { useTranslation } from 'react-i18next'

import { Brand } from '../auth/auth'
import { useAuthenticatedUser } from '../auth/context'
import { loadHealth } from '../chat/chatApi'

const NAV_ITEMS = [
  { to: '/chat', icon: 'C', labelKey: 'nav.chat', fallback: '对话' },
  { to: '/experts', icon: 'E', labelKey: 'nav.experts', fallback: '专家' },
  { to: '/workspace', icon: 'W', labelKey: 'nav.workspace', fallback: '工作区' },
  { to: '/memory', icon: 'M', labelKey: 'nav.memory', fallback: '资料库' },
  // 图标是单字母。刻意用 T 而不是 U：admin 区里的 U 已经是「用户」了，
  // 再占一个 U 会让侧栏里两个「U」长得一模一样。
  { to: '/usage', icon: 'T', labelKey: 'nav.usage', fallback: '用量统计' },
  // 这一项以前叫「设备」，点进去渲染的却是「MCP 服务」页
  // （devices/Devices.tsx 里 title 就是「MCP 服务」），而 /api/devices
  // 后端压根没注册、返回 404。名不副实的入口比没有入口更费时间 ——
  // 用户会以为找错了地方。
  { to: '/devices', icon: 'D', labelKey: 'nav.mcpServers', fallback: 'MCP 服务' },
  { to: '/skills', icon: 'K', labelKey: 'nav.skills', fallback: '技能包' },
  { to: '/automations', icon: 'A', labelKey: 'nav.automations', fallback: '自动化' },
] as const

const ADMIN_NAV_ITEMS = [
  { to: '/admin/models', icon: 'L', labelKey: 'nav.models', fallback: '模型' },
  { to: '/admin/instance', icon: 'S', labelKey: 'admin.settings', fallback: '实例设置' },
  { to: '/admin/users', icon: 'U', labelKey: 'admin.users', fallback: '用户' },
  // 这里原来有一项「共享 MCP」。**它和上面的「MCP 服务」操作的是同一份数据**：
  // 同一个 `/api/extensions/mcp`、同一个 `MCP_KEY` 查询缓存，只是两套长得
  // 不一样的表单。而 quill 只有实例级一套 MCP，「共享」这个词暗示的
  // 「多用户各自分享」并不存在。两处入口改一边另一边不跟着变，
  // 最终必然漂移，所以合并成一处。
] as const

export function AppShell(): ReactNode {
  const { t } = useTranslation()
  const user = useAuthenticatedUser()
  // 最近对话已经从左栏移走（Octop 的做法）：会话列表按角色放在对话页的第二侧栏。
  const health = useQuery({ queryKey: ['health'], queryFn: loadHealth, staleTime: 30_000, retry: false })

  return (
    <main className="stage">
      <div className="app-shell">
        <aside className="sidebar">
          <Brand />
          <Link className="new-session" to="/chat"><span aria-hidden="true">＋</span> {t('nav.newChat', { defaultValue: '新对话' })}</Link>
          <nav className="nav" aria-label={t('nav.mainLabel', { defaultValue: '主导航' })}>
            {NAV_ITEMS.map((item) => (
              <NavLink key={item.to} to={item.to} className="nav-item">
                <span className="nav-icon" aria-hidden="true">{item.icon}</span>
                {t(item.labelKey, { defaultValue: item.fallback })}
              </NavLink>
            ))}
            {user.is_admin ? (
              <>
                <div className="section-label">
                  <span>{t('nav.admin', { defaultValue: '管理' })}</span>
                </div>
                {ADMIN_NAV_ITEMS.map((item) => (
                  <NavLink key={item.to} to={item.to} className="nav-item">
                    <span className="nav-icon" aria-hidden="true">{item.icon}</span>
                    {t(item.labelKey, { defaultValue: item.fallback })}
                  </NavLink>
                ))}
              </>
            ) : null}
          </nav>
          <footer className="sidebar-footer">
            <Link
              className="profile-button"
              to="/account"
              aria-label={t('account.title', { defaultValue: '账户' })}
              title={t('account.title', { defaultValue: '账户' })}
            >
              <span className="avatar">{(user.username || user.id || '?').slice(0, 2).toUpperCase()}</span>
              <span className="profile-copy">
                <strong className="profile-name">{user.display_name || user.username}</strong>
                <small className="profile-role">
                  {user.is_admin ? t('nav.administrator', { defaultValue: '管理员' }) : t('nav.member', { defaultValue: '成员' })}
                </small>
              </span>
            </Link>
            {health.data ? (
              <p className={`chat-banner${health.data.storage.ready ? '' : ' chat-banner-error'}`}>
                {health.data.llm.configured
                  ? t('nav.llmReady', { model: health.data.llm.model, defaultValue: '模型 {{model}} 就绪' })
                  : t('nav.llmMissing', { defaultValue: '模型未配置' })}
              </p>
            ) : null}
          </footer>
        </aside>
        <section className="workspace"><Outlet /></section>
      </div>
    </main>
  )
}
