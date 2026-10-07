import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, PageHeader } from '../components/Page'

/**
 * 工作区 —— 文件浏览器。目前后端没有这套接口，所以**这里不画任何浏览器骨架**。
 *
 * 这一页以前长这样：上游的三栏文件浏览器骨架（位置 / 文件 / 详情），
 * 加上**备份与升级两张卡片**。备份是别处来的，理由写在当时的注释里：
 * 「工作区页没有文件接口（`/api/workspace/*` 全部未登记），改接真实存在的备份路由」。
 * 也就是说，**这一页自己的功能是空的，拿备份来顶替**。
 *
 * 代价是让用户以为备份属于工作区。参照 octop：它**压根没有工作区页** ——
 * `{ path: "/workspace", element: <Navigate to="/experts" replace /> }`
 * （`.octop-ref/octop/dashboard/src/routes/index.tsx:213`）直接把 `/workspace`
 * 重定向到专家页。备份归它的 `/admin/backend`，升级归 `/admin/advanced?tab=updates`。
 *
 * 2026-10-07 把备份与升级搬去了 admin 区（`/admin/backup`）。
 * 三栏骨架也一并撤掉：**空骨架比一句话更糟** —— 一个永远空的文件列表框
 * 看着像「文件加载不出来」，而真相是根本没有这个接口。
 */
export function WorkspacePage(): ReactNode {
  const { t } = useTranslation()
  return (
    <div className="page-scroll">
      <PageHeader
        title={t('workspace.title', { defaultValue: '工作区' })}
        description={t('workspace.notWiredDescription', {
          defaultValue: '浏览与编辑实例工作区里的文件。',
        })}
      />
      <div className="settings-stack">
        <Card
          title={t('workspace.notWiredTitle', { defaultValue: '工作区文件接口尚未接通' })}
          description={t('workspace.notWiredCardDescription', {
            defaultValue: '这一页没有文件列表、目录树或在线设备 —— 因为后端没有这些接口。',
          })}
        >
          <p className="field-help full-row" role="status">
            {t('workspace.filesNotWired', {
              defaultValue:
                'quill 后端没有工作区文件接口：{{route}} 与 {{list}} 都未在本实例登记，所以这一页没有文件、目录或在线设备（不做假数据）。下一步：实现 {{list}}（列目录）与 {{route}}（读写真实文本文件，含 ETag 乐观并发），文件浏览器就会自动接上。',
              route: WORKSPACE_FILE_ROUTE,
              list: WORKSPACE_LIST_ROUTE,
            })}
          </p>
          <p className="field-help full-row">
            {t('workspace.backupMoved', {
              defaultValue:
                '以前这一页还放着备份与升级。它们是实例运维能力，不属于工作区，已经搬到「{{link}}」。',
              link: t('admin.backupTitle', { defaultValue: '备份与升级' }),
            })}
          </p>
        </Card>
      </div>
    </div>
  )
}

const WORKSPACE_FILE_ROUTE = '/api/workspace/files'
const WORKSPACE_LIST_ROUTE = '/api/workspace/list'
