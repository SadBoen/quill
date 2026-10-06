import { useMutation, useQuery } from '@tanstack/react-query'
import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice, StatusBadge } from '../components/Page'
import {
  BACKUP_EXPORT_ROUTE,
  BACKUP_RESTORE_ROUTE,
  BACKUP_VERIFY_ROUTE,
  RESTORE_COMMAND,
  UPGRADE_CHECK_ROUTE,
  UPGRADE_HISTORY_ROUTE,
  UPGRADE_PREPARE_ROUTE,
  WORKSPACE_FILE_ROUTE,
  backupFailure,
  backupFailureLabel,
  checkUpgrade,
  exportBackup,
  isBackupExportResult,
  isBackupVerifyResult,
  listUpgradeHistory,
  prepareUpgrade,
  verifyBackup,
} from './api'
import type { BackupExportResult, BackupVerifyResult } from './api'
import './workspace.css'

/**
 * 上游的「工作区」是一个跨设备文件浏览器（`/api/workspace/list`、`/api/workspace/files`），
 * quill 后端没有这套接口。这里保留上游的三栏骨架与 class 名，
 * 文件区改为明确的「尚未接通」说明，并接上 quill 真实存在的备份 / 升级路由。
 */

/** 备份名是一个**相对名**，服务端在自己的备份根目录（数据根目录的同级）下解析它。 */
function defaultBackupName(): string {
  const now = new Date()
  const pad = (n: number): string => String(n).padStart(2, '0')
  return `backup-${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`
}

/**
 * 导出结果只有逐字段对得上才渲染成「成功」。
 * 服务端回了 201 但摘要/计数是空的 —— 那不是一份可信的备份，界面必须说读不懂。
 */
function BackupResult({ result }: { result: BackupExportResult }): ReactNode {
  const { t } = useTranslation()
  return (
    <div className="oo-workspace-backup-result" data-testid="backup-export-result" role="status">
      <StatusBadge tone="success">{t('workspace.backupResultTitle', { defaultValue: '导出完成' })}</StatusBadge>
      <dl className="detail-grid">
        <div>
          <dt>{t('workspace.backupDest', { defaultValue: '目标目录' })}</dt>
          <dd data-testid="backup-dest">{result.dest}</dd>
        </div>
        <div>
          <dt>{t('workspace.backupDbDigest', { defaultValue: '数据库摘要' })}</dt>
          <dd>
            <code>{result.db_sha256}</code>
          </dd>
        </div>
        <div>
          <dt>{t('workspace.backupFiles', { defaultValue: '文件数' })}</dt>
          <dd data-testid="backup-files">{result.files}</dd>
        </div>
        <div>
          <dt>{t('workspace.backupTotalBytes', { defaultValue: '总字节' })}</dt>
          <dd data-testid="backup-total-bytes">{result.total_bytes}</dd>
        </div>
      </dl>
      <p className="oo-workspace-excluded-title">
        {t('workspace.backupExcludedTitle', {
          defaultValue: '没进备份的密钥材料（{{count}} 项）：',
          count: result.excluded.length,
        })}
      </p>
      {result.excluded.length === 0 ? (
        <p className="oo-workspace-excluded-none">
          {t('workspace.backupExcludedNone', { defaultValue: '这一次没有任何文件被排除。' })}
        </p>
      ) : (
        <ul className="oo-workspace-excluded-list" data-testid="backup-excluded">
          {result.excluded.map((entry) => (
            <li key={entry.rel}>
              <code>{entry.rel}</code> —— {entry.reason}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function VerifyResult({ result }: { result: BackupVerifyResult }): ReactNode {
  const { t } = useTranslation()
  return (
    <div className="oo-workspace-backup-result" data-testid="backup-verify-result" role="status">
      <StatusBadge tone="success">{t('workspace.verifyOk', { defaultValue: '校验通过' })}</StatusBadge>
      <dl className="detail-grid">
        <div>
          <dt>ok</dt>
          <dd data-testid="verify-ok">{String(result.ok)}</dd>
        </div>
        <div>
          <dt>{t('workspace.backupFiles', { defaultValue: '文件数' })}</dt>
          <dd data-testid="verify-files">{result.files}</dd>
        </div>
        <div>
          <dt>{t('workspace.backupTotalBytes', { defaultValue: '总字节' })}</dt>
          <dd data-testid="verify-total-bytes">{result.total_bytes}</dd>
        </div>
        <div>
          <dt>{t('workspace.backupDbDigest', { defaultValue: '数据库摘要' })}</dt>
          <dd>
            <code>{result.db_sha256}</code>
          </dd>
        </div>
      </dl>
    </div>
  )
}

export function WorkspacePage(): ReactNode {
  const { t } = useTranslation()
  const [name, setName] = useState(defaultBackupName)
  const trimmed = name.trim()
  const upgrade = useQuery({ queryKey: ['upgrade-check'], queryFn: checkUpgrade, retry: false })
  const history = useQuery({ queryKey: ['upgrade-history'], queryFn: listUpgradeHistory, retry: false })
  const backup = useMutation({ mutationFn: exportBackup })
  const verify = useMutation({ mutationFn: verifyBackup })
  const prepare = useMutation({ mutationFn: prepareUpgrade })

  const exportFailure = backupFailure(backup.error)
  const verifyFailure = backupFailure(verify.error)
  // 只有响应逐字段对得上才算成功；201/200 但字段缺失或类型不对一律按「读不懂」处理。
  const exportResult = backup.isSuccess && isBackupExportResult(backup.data) ? backup.data : null
  const verifyResult = verify.isSuccess && isBackupVerifyResult(verify.data) ? verify.data : null
  const exportUnreadable = backup.isSuccess && backup.data !== undefined && !exportResult
  const verifyUnreadable = verify.isSuccess && verify.data !== undefined && !verifyResult

  return (
    <>
      <header className="workspace-header">
        <div className="breadcrumbs">
          <span>{t('workspace.title', { defaultValue: '工作区' })}</span>
          <span aria-hidden="true">/</span>
          <strong>{t('workspace.dataRoot', { defaultValue: '数据根目录' })}</strong>
        </div>
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
            <div className="oo-workspace-backup-form">
              <label>
                {t('workspace.backupName', { defaultValue: '备份名' })}
                <input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="backup-2026-10-06"
                  spellCheck={false}
                />
              </label>
              <div className="form-actions">
                <button
                  className="primary-button"
                  type="button"
                  disabled={backup.isPending || trimmed === ''}
                  onClick={() => backup.mutate(trimmed)}
                >
                  {t('workspace.exportBackup', { defaultValue: '导出备份' })}
                </button>
                <button
                  className="secondary-button"
                  type="button"
                  disabled={verify.isPending || trimmed === ''}
                  onClick={() => verify.mutate(trimmed)}
                >
                  {t('workspace.verifyBackup', { defaultValue: '校验备份' })}
                </button>
              </div>
            </div>
            <p className="field-help full-row">
              {t('workspace.backupRoutes', {
                defaultValue:
                  '备份名是一个相对名，不是路径：服务端把它解析到自己备份根目录（数据根目录的同级）下，绝对路径、盘符与 .. 都会被拒。导出走 {{export}}，校验走 {{verify}}。',
                export: BACKUP_EXPORT_ROUTE,
                verify: BACKUP_VERIFY_ROUTE,
              })}
            </p>
            {exportFailure ? (
              <p className="oo-workspace-failure" data-testid={`backup-error-${exportFailure}`}>
                {backupFailureLabel(exportFailure, t)}
              </p>
            ) : null}
            {verifyFailure ? (
              <p className="oo-workspace-failure" data-testid={`backup-error-${verifyFailure}`}>
                {backupFailureLabel(verifyFailure, t)}
              </p>
            ) : null}
            {exportUnreadable || verifyUnreadable ? (
              <p className="oo-workspace-failure" data-testid="backup-error-unreadable">
                {t('workspace.backupResultUnreadable', {
                  defaultValue:
                    '服务端回了成功状态，但备份结果里有字段缺失或类型不对，所以这里不显示「已完成」。这不是一次成功的备份。下一步：去看服务端日志，核对 export / verify 的响应字段。',
                })}
              </p>
            ) : null}
            <ErrorNotice error={backup.error ?? verify.error ?? upgrade.error ?? history.error} />
            {exportResult ? <BackupResult result={exportResult} /> : null}
            {verifyResult ? <VerifyResult result={verifyResult} /> : null}

            <p className="oo-workspace-restore-note" data-testid="restore-note">
              <strong>{t('workspace.restoreNoteTitle', { defaultValue: '还原要先停掉服务端' })}</strong>
              {t('workspace.restoreNoteBody', {
                defaultValue:
                  '服务端运行期间数据库文件被它自己占着，{{route}} 在页面上永远不会报成功；还原又是破坏性的操作，所以没有还原按钮。下一步：停掉服务端后执行 {{command}}（不带 --yes 时它只打印预告，一个字节都不写）。',
                route: BACKUP_RESTORE_ROUTE,
                command: RESTORE_COMMAND,
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