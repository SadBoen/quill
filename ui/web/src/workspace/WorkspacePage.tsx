import { useMutation, useQuery } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice } from '../components/Page'
import {
  BACKUP_EXPORT_ROUTE,
  BACKUP_VERIFY_ROUTE,
  UPGRADE_CHECK_ROUTE,
  UPGRADE_HISTORY_ROUTE,
  UPGRADE_PREPARE_ROUTE,
  WORKSPACE_FILE_ROUTE,
  checkUpgrade,
  exportBackup,
  listUpgradeHistory,
  prepareUpgrade,
  verifyBackup,
} from './api'
import './workspace.css'

/**
 * 上游的「工作区」是一个跨设备文件浏览器（`/api/workspace/list`、`/api/workspace/files`），
 * quill 后端没有这套接口。这里保留上游的三栏骨架与 class 名，
 * 文件区改为明确的「尚未接通」说明，并接上 quill 真实存在的备份 / 升级路由。
 */
export function WorkspacePage(): ReactNode {
  const { t } = useTranslation()
  const upgrade = useQuery({ queryKey: ['upgrade-check'], queryFn: checkUpgrade, retry: false })
  const history = useQuery({ queryKey: ['upgrade-history'], queryFn: listUpgradeHistory, retry: false })
  const backup = useMutation({ mutationFn: exportBackup })
  const verify = useMutation({
    mutationFn: (path: string) => verifyBackup(path ? { path } : {}),
  })
  const prepare = useMutation({ mutationFn: prepareUpgrade })

  return (
    <>
      <header className="workspace-header">
        <div className="breadcrumbs">
          <span>{t('workspace.title', { defaultValue: '工作区' })}</span>
          <span aria-hidden="true">/</span>
          <strong>{t('workspace.dataRoot', { defaultValue: '数据根目录' })}</strong>
        </div>
        <button
          className="primary-button"
          type="button"
          disabled={backup.isPending}
          onClick={() => backup.mutate()}
        >
          {t('workspace.exportBackup', { defaultValue: '导出备份' })}
        </button>
      </header>

      <div className="oo-workspace-page">
        <h1 className="sr-only">{t('workspace.title', { defaultValue: '工作区' })}</h1>

        <p className="oo-workspace-notice" role="status">
          {t('workspace.filesNotWired', {
            defaultValue: 'quill 后端没有工作区文件接口：{{route}} 未在本实例登记，所以下面三栏里没有任何文件、目录或在线设备（不做假数据）。下一步：实现 {{list}}（列目录）与 {{route}}（读写真实文本文件，含 ETag 乐观并发），本浏览器的功能就会自动接上。',
            route: WORKSPACE_FILE_ROUTE,
            list: '/api/workspace/list',
          })}
        </p>

        <div className="oo-workspace-browser">
          <aside className="oo-workspace-locations" aria-label={t('workspace.locations', { defaultValue: '工作区位置' })}>
            <h2>{t('workspace.serverLocations', { defaultValue: '服务端位置' })}</h2>
            <p>{t('workspace.noLocations', { defaultValue: '没有可用的位置：quill 尚未提供工作区列表。' })}</p>
          </aside>

          <section className="oo-workspace-files" aria-label={t('workspace.filesRegion', { defaultValue: '工作区文件' })}>
            <div className="oo-workspace-toolbar">
              <div>
                <button type="button" disabled>← {t('workspace.up', { defaultValue: '上一级' })}</button>
                <code>.</code>
              </div>
            </div>
            <div className="oo-workspace-file-header">
              <span>{t('workspace.nameColumn', { defaultValue: '名称' })}</span>
              <span>{t('workspace.type', { defaultValue: '类型' })}</span>
              <span>{t('workspace.size', { defaultValue: '大小' })}</span>
            </div>
            <p className="oo-workspace-empty">
              {t('workspace.emptyDirectory', { defaultValue: '文件列表为空：读取接口尚未接通。' })}
            </p>
            <ul className="oo-workspace-file-list" aria-label={t('workspace.fileList', { defaultValue: '工作区文件' })} />
          </section>

          <aside className="oo-workspace-inspector" aria-label={t('workspace.fileDetails', { defaultValue: '文件详情' })}>
            <p>{t('workspace.selectFile', { defaultValue: '选中文件后在这里查看详情。' })}</p>
          </aside>
        </div>

        <div className="settings-stack">
          <Card
            title={t('workspace.backupTitle', { defaultValue: '备份' })}
            description={t('workspace.backupDescription', { defaultValue: '把数据根目录打成一份可校验的归档。' })}
          >
            <ErrorNotice error={backup.error ?? verify.error ?? upgrade.error ?? history.error} />
            <div className="form-actions">
              <button
                className="primary-button"
                type="button"
                disabled={backup.isPending}
                onClick={() => backup.mutate()}
              >
                {t('workspace.exportBackup', { defaultValue: '导出备份' })}
              </button>
              <button
                className="secondary-button"
                type="button"
                disabled={verify.isPending}
                onClick={() => verify.mutate('')}
              >
                {t('workspace.verifyBackup', { defaultValue: '校验备份' })}
              </button>
            </div>
            <p className="field-help full-row">
              {t('workspace.backupNotWired', {
                defaultValue: '备份与校验走 {{export}} 与 {{verify}}：这两个路由在 quill 里已登记但尚未实现，失败时上面显示的是服务端原文与下一步提示。',
                export: BACKUP_EXPORT_ROUTE,
                verify: BACKUP_VERIFY_ROUTE,
              })}
            </p>
          </Card>

          <Card
            title={t('workspace.upgradeTitle', { defaultValue: '升级' })}
            description={t('workspace.upgradeDescription', { defaultValue: '查看是否有新版本并准备升级。' })}
          >
            <dl className="detail-grid">
              <div>
                <dt>{t('workspace.currentVersion', { defaultValue: '当前版本' })}</dt>
                <dd>{upgrade.data?.current_version ?? '—'}</dd>
              </div>
              <div>
                <dt>{t('workspace.latestVersion', { defaultValue: '最新版本' })}</dt>
                <dd>{upgrade.data?.latest_version ?? '—'}</dd>
              </div>
            </dl>
            <div className="form-actions">
              <button
                className="primary-button"
                type="button"
                disabled={prepare.isPending}
                onClick={() => prepare.mutate()}
              >
                {t('workspace.prepareUpgrade', { defaultValue: '准备升级' })}
              </button>
              <button
                className="secondary-button"
                type="button"
                onClick={() => void upgrade.refetch()}
              >
                {t('common.refresh', { defaultValue: '刷新' })}
              </button>
            </div>
            <p className="field-help full-row">
              {t('workspace.upgradeNotWired', {
                defaultValue: '升级走 {{check}}、{{prepare}}、{{history}}：都已登记但尚未实现，所以版本号取不到（显示为「—」，不是占位版本号）。',
                check: UPGRADE_CHECK_ROUTE,
                prepare: UPGRADE_PREPARE_ROUTE,
                history: UPGRADE_HISTORY_ROUTE,
              })}
            </p>
          </Card>
        </div>
      </div>
    </>
  )
}
