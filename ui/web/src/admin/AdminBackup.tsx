import { useMutation, useQuery } from '@tanstack/react-query'
import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Card, ErrorNotice, PageHeader, StatusBadge } from '../components/Page'
import { pickVisibleError, routeNotImplemented } from '../api/capability'
import {
  BACKUP_EXPORT_ROUTE,
  BACKUP_RESTORE_ROUTE,
  BACKUP_VERIFY_ROUTE,
  RESTORE_COMMAND,
  UPGRADE_CHECK_ROUTE,
  UPGRADE_HISTORY_ROUTE,
  UPGRADE_PREPARE_ROUTE,
  backupFailure,
  backupFailureLabel,
  checkUpgrade,
  exportBackup,
  isBackupExportResult,
  isBackupVerifyResult,
  listUpgradeHistory,
  prepareUpgrade,
  verifyBackup,
} from './backupApi'
import type { BackupExportResult, BackupVerifyResult } from './backupApi'
import { AdminTabs } from './AdminTabs'
import './backup.css'

/**
 * 实例的备份与升级。
 *
 * **为什么在 admin 而不是工作区**：这两个是实例运维能力。
 * 参照 octop —— 它的备份归 `/admin/backend`、升级归 `/admin/advanced?tab=updates`
 * （`.octop-ref/octop/dashboard/src/routes/index.tsx:229,236,244`），
 * 都在 admin 区。octop 压根没有工作区页（`:213` 把 `/workspace` 直接重定向到专家页），
 * 所以不存在「备份掉进工作区」这个问题。我们之前那样放，是因为工作区自己的
 * 功能是空的（`/api/workspace/*` 全部未登记），拿备份顶替 ——
 * 代价是让用户以为备份属于工作区。
 *
 * 这个页面上**没有一个点不动的按钮**：备份的导出/校验是真路由，
 * 升级的按钮在服务端还是 501 桩时**不渲染**（见下面的 `upgradeUnavailable`）。
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
    <div className="admin-backup-backup-result" data-testid="backup-export-result" role="status">
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
      <p className="admin-backup-excluded-title">
        {t('workspace.backupExcludedTitle', {
          defaultValue: '没进备份的密钥材料（{{count}} 项）：',
          count: result.excluded.length,
        })}
      </p>
      {result.excluded.length === 0 ? (
        <p className="admin-backup-excluded-none">
          {t('workspace.backupExcludedNone', { defaultValue: '这一次没有任何文件被排除。' })}
        </p>
      ) : (
        <ul className="admin-backup-excluded-list" data-testid="backup-excluded">
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
    <div className="admin-backup-backup-result" data-testid="backup-verify-result" role="status">
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

export function AdminBackupPage(): ReactNode {
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

  // 升级那三条路由**现在已经真实现**（Q038–Q040：check / prepare / history 都有真
  // handler，`routes.rs` 里不再是 `not_implemented` 桩），所以这段守卫平时不触发。
  // 它留着是**防御性**的：当初的缺陷是「桩上画按钮」—— 实点时 POST /api/upgrade/prepare
  // 回 501，界面零提示，用户以为升级在跑了。哪天这几条被改回桩，守卫要能自动把人挡住，
  // 而不是重新长出一个点了必失败还不报错的按钮（测试用模拟 501 钉住这一支）。
  const upgradeUnavailable =
    routeNotImplemented(upgrade.error) ||
    routeNotImplemented(history.error) ||
    routeNotImplemented(prepare.error)

  return (
    <div className="page-scroll">
      <PageHeader
        eyebrow={t('nav.admin', { defaultValue: '管理' })}
        title={t('admin.backupTitle', { defaultValue: '备份与升级' })}
        description={t('admin.backupPageDescription', {
          defaultValue:
            '备份与升级是实例运维能力。octop 把备份归「后台」、升级归「高级设置 → 更新」，都在管理区；它没有工作区页，所以这两个功能不会掉进工作区。',
        })}
        actions={<AdminTabs />}
      />
      <div className="settings-stack">
        <Card
          title={t('workspace.backupTitle', { defaultValue: '备份' })}
          description={t('workspace.backupDescription', { defaultValue: '把数据根目录打成一份可校验的归档。' })}
        >
          <div className="admin-backup-form">
            <label>
              {t('workspace.backupName', { defaultValue: '备份名' })}
              <input
                name="backupName"
                autoComplete="off"
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
            <p className="admin-backup-failure" data-testid={`backup-error-${exportFailure}`}>
              {backupFailureLabel(exportFailure, t)}
            </p>
          ) : null}
          {verifyFailure ? (
            <p className="admin-backup-failure" data-testid={`backup-error-${verifyFailure}`}>
              {backupFailureLabel(verifyFailure, t)}
            </p>
          ) : null}
          {/* 下面两条失败码不挂 role="alert"：它们的 error 已经交给紧邻的
              ErrorNotice 播报了（ErrorNotice 自带 role="alert"），再加一次
              屏幕阅读器会把同一次失败读两遍。下面 unreadable 那条不一样：
              服务端回的是成功、没有 error，ErrorNotice 什么都不渲染，
              只能由它自己播报。 */}
          {exportUnreadable || verifyUnreadable ? (
            <p className="admin-backup-failure" role="alert" data-testid="backup-error-unreadable">
              {t('workspace.backupResultUnreadable', {
                defaultValue:
                  '服务端回了成功状态，但备份结果里有字段缺失或类型不对，所以这里不显示「已完成」。这不是一次成功的备份。下一步：去看服务端日志，核对 export / verify 的响应字段。',
              })}
            </p>
          ) : null}
          <ErrorNotice error={pickVisibleError(backup.error, verify.error)} />
          {exportResult ? <BackupResult result={exportResult} /> : null}
          {verifyResult ? <VerifyResult result={verifyResult} /> : null}

          <p className="admin-backup-restore-note" data-testid="restore-note">
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
          {upgradeUnavailable ? (
            <p className="admin-backup-failure" data-testid="upgrade-unavailable">
              <StatusBadge tone="warning">
                {t('workspace.upgradeUnavailableBadge', { defaultValue: '尚未实现' })}
              </StatusBadge>{' '}
              {t('workspace.upgradeUnavailable', {
                defaultValue:
                  '服务端把升级这几条路由留成了 501 桩，所以这里没有「准备升级」和「刷新」：点下去只会拿到 501，且点了不会发生任何升级。等真做出来之后按下面的说明接上，这两个按钮会自动出现。',
              })}
            </p>
          ) : (
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
                disabled={upgrade.isFetching}
                onClick={() => void upgrade.refetch()}
              >
                {t('common.refresh', { defaultValue: '刷新' })}
              </button>
            </div>
          )}
          {/* 升级的错误端在这一张卡里，不端去备份卡：端过去它会被备份那两条
              错误按 `??` 顺序挡掉，用户在升级这块点了东西却看不到任何提示。 */}
          <ErrorNotice error={pickVisibleError(prepare.error, upgrade.error, history.error)} />
          <p className="field-help full-row">
            {upgradeUnavailable
              ? t('workspace.upgradeNotWired', {
                  defaultValue: '升级走 {{check}}、{{prepare}}、{{history}}：都已登记但尚未实现，所以版本号取不到（显示为「—」，不是占位版本号）。',
                  check: UPGRADE_CHECK_ROUTE,
                  prepare: UPGRADE_PREPARE_ROUTE,
                  history: UPGRADE_HISTORY_ROUTE,
                })
              : // 这句不能永远写死「尚未实现」：能力接上之后它就成了一句谎话，
                // 而用户正是靠它判断能不能点那两个按钮。
                t('workspace.upgradeRoutes', {
                  defaultValue: '版本号取自 {{check}}，准备升级走 {{prepare}}，升级历史走 {{history}}。',
                  check: UPGRADE_CHECK_ROUTE,
                  prepare: UPGRADE_PREPARE_ROUTE,
                  history: UPGRADE_HISTORY_ROUTE,
                })}
          </p>
        </Card>
      </div>
    </div>
  )
}
