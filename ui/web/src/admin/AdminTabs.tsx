import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { Link } from 'react-router-dom'

/**
 * 管理区的页签。
 *
 * **单独一个文件而不是每个页各自写一份**：备份页搬进 admin 之后，
 * 立刻出现了两份 `AdminTabs`。两处入口以后加一项必然只改一处，
 * 于是漂移 —— 之前「共享 MCP」就是这么消失的（见下面那条注释）。
 */
export function AdminTabs(): ReactNode {
  const { t } = useTranslation()
  return (
    <nav className="inline-tabs" aria-label={t('admin.adminPages', { defaultValue: '管理页面' })}>
      <Link to="/admin/models">{t('nav.models', { defaultValue: '模型' })}</Link>
      <Link to="/admin/instance">{t('admin.settings', { defaultValue: '实例设置' })}</Link>
      <Link to="/admin/backup">{t('admin.backupTitle', { defaultValue: '备份与升级' })}</Link>
      <Link to="/admin/users">{t('admin.users', { defaultValue: '用户' })}</Link>
      {/* 原来这里链到 /admin/mcp「共享 MCP」。它与左侧的「MCP 服务」是同一份数据、
          同一个接口，只是另一套表单 —— 而 quill 只有实例级一套，「共享」暗示的
          分用户分享并不存在。两处改一边另一边不跟着变，最终必然漂移，所以指向同一处。 */}
      <Link to="/devices">{t('nav.mcpServers', { defaultValue: 'MCP 服务' })}</Link>
    </nav>
  )
}
