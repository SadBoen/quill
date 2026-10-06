import type { ReactNode } from 'react'
import { Navigate, Route, Routes } from 'react-router-dom'

import { AccountPage } from '../account/Account'
import { AdminInstancePage, AdminMcpPage, AdminUsersPage } from '../admin/Admin'
import { AutomationsPage } from '../automations/Automations'
import { AuthPage, RequireAdmin, RequireAuth } from '../auth/auth'
import { ChatPage } from '../chat'
import { DeviceDetailPage, DeviceListPage, DeviceMcpPage } from '../devices/Devices'
import { ExpertsPage } from '../experts'
import { AppShell } from '../layout/AppShell'
import { MemoryPage } from '../memory/MemoryPage'
import { ModelsPage } from '../models'
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
            <Route path="/workspace/:workspaceRef" element={<WorkspacePage />} />
            <Route path="/memory" element={<MemoryPage />} />
            <Route path="/usage" element={<UsagePage />} />
            <Route path="/devices" element={<DeviceListPage />} />
            <Route path="/devices/:name" element={<DeviceDetailPage />} />
            <Route path="/devices/:name/mcp" element={<DeviceMcpPage />} />
            <Route path="/automations" element={<AutomationsPage />} />
            <Route path="/account" element={<AccountPage />} />
            <Route element={<RequireAdmin />}>
              <Route path="/admin/models" element={<ModelsPage />} />
              <Route path="/admin/instance" element={<AdminInstancePage />} />
              <Route path="/admin/users" element={<AdminUsersPage />} />
              <Route path="/admin/mcp" element={<AdminMcpPage />} />
              <Route path="/admin/settings" element={<Navigate to="/admin/models" replace />} />
            </Route>
          </Route>
        </Route>
        <Route path="*" element={<Navigate to="/chat" replace />} />
      </Routes>
    </ThemeProvider>
  )
}
