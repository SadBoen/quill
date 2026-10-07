import type { ReactNode } from 'react'
import { Navigate, Route, Routes } from 'react-router-dom'

import { AccountPage } from '../account/Account'
import { AdminInstancePage, AdminUsersPage } from '../admin/Admin'
import { AdminBackupPage } from '../admin/AdminBackup'
import { ChannelsPage } from '../channels/Channels'
import { MbtiPage } from '../mbti/MbtiPage'
import { PersonalizationPage } from '../personalization/PersonalizationPage'
import { AutomationsPage } from '../automations/Automations'
import { AuthPage, RequireAdmin, RequireAuth } from '../auth/auth'
import { ChatPage } from '../chat'
import { DeviceListPage } from '../devices/Devices'
import { ExpertsPage } from '../experts'
import { AppShell } from '../layout/AppShell'
import { MemoryPage } from '../memory/MemoryPage'
import { ModelsPage } from '../models'
import { SkillsPage } from '../skills/SkillsPage'
import { ThemeProvider } from '../theme/ThemeToggle'
import { UsagePage } from '../usage/UsagePage'
import { WorkspacePage } from '../workspace/WorkspacePage'

export function AppRoutes(): ReactNode {
  return (
    <ThemeProvider>
      <Routes>
        <Route path="/login" element={<AuthPage />} />
        <Route element={<RequireAuth />}>
          <Route element={<AppShell />}>
            <Route index element={<Navigate to="/chat" replace />} />
            <Route path="/chat" element={<ChatPage />} />
            <Route path="/chat/:sessionId" element={<ChatPage />} />
            <Route path="/experts" element={<ExpertsPage />} />
            <Route path="/workspace" element={<WorkspacePage />} />
            <Route path="/workspace/:workspaceRef" element={<WorkspacePage />} />            <Route path="/memory" element={<MemoryPage />} />
            <Route path="/usage" element={<UsagePage />} />
            <Route path="/devices" element={<DeviceListPage />} />
            {/* 设备名册后端压根没实现（/api/devices 返回 404），以前这一页摆着一段
                「quill 里没有设备名册」的死说明。**明说做不到是对的，留一页死页是错的** ——
                现在直接把人送到「MCP 服务」，那才是这里唯一真实存在的东西。 */}
            <Route path="/devices/:name" element={<Navigate to="/devices" replace />} />
            <Route path="/devices/:name/mcp" element={<Navigate to="/devices" replace />} />
            <Route path="/skills" element={<SkillsPage />} />
            <Route path="/automations" element={<AutomationsPage />} />
            <Route path="/channels" element={<ChannelsPage />} />
            <Route path="/personalization" element={<PersonalizationPage />} />
            {/* 人格测评与光谱。Octop 把它放在个性化页的一个页签里（MBTISelector.tsx），
                本项目给它独立一页：它有测 - 看 - 用三段流程，塞进索引页会挤。 */}
            <Route path="/mbti" element={<MbtiPage />} />
            <Route path="/account" element={<AccountPage />} />
            <Route element={<RequireAdmin />}>
              <Route path="/admin/models" element={<ModelsPage />} />
              <Route path="/admin/instance" element={<AdminInstancePage />} />
              {/* 备份与升级从工作区搬到这里（2026-10-07）。参照 octop：它的备份归
                  /admin/backend、升级归 /admin/advanced?tab=updates，都在 admin 区，
                  而它压根没有工作区页（routes/index.tsx:213 把 /workspace 重定向到专家页）。 */}
              <Route path="/admin/backup" element={<AdminBackupPage />} />
              <Route path="/admin/users" element={<AdminUsersPage />} />
              {/* 「共享 MCP」与「MCP 服务」是同一份数据，已合并到 /devices。
                  老链接仍然能用 —— 书签、外链、别人发给你的地址不该因为改了
                  信息架构就变成 404。 */}
              <Route path="/admin/mcp" element={<Navigate to="/devices" replace />} />
              <Route path="/admin/settings" element={<Navigate to="/admin/models" replace />} />
            </Route>
          </Route>
        </Route>
        <Route path="*" element={<Navigate to="/chat" replace />} />
      </Routes>
    </ThemeProvider>
  )
}
