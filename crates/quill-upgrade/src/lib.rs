use quill_backup::{
    create_backup, restore_backup, BackupError, BackupReport, BackupSource, RestoreReport,
};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PreUpgradeGuard {
    backup_dir: PathBuf,
    db_path: PathBuf,
    data_root: PathBuf,
    app_version: String,
    report: BackupReport,
    last_unrestored: Vec<String>,
}

impl PreUpgradeGuard {
    pub fn backup_dir(&self) -> &Path {
        &self.backup_dir
    }

    pub fn app_version(&self) -> &str {
        &self.app_version
    }

    pub fn report(&self) -> &BackupReport {
        &self.report
    }

    pub async fn rollback(&mut self) -> Result<RestoreReport, UpgradeError> {
        let report = restore_backup(&self.backup_dir, &self.db_path, &self.data_root)
            .await
            .map_err(|source| UpgradeError::RollbackFailed {
                backup_dir: self.backup_dir.clone(),
                source,
            })?;
        if !report.not_overwritten.is_empty() {
            self.last_unrestored = report.not_overwritten.clone();
        }
        Ok(report)
    }

    pub fn unrestored_files(&self) -> &[String] {
        &self.last_unrestored
    }
}

#[derive(Debug)]
pub enum UpgradeError {
    PreUpgradeBackupFailed {
        dest: PathBuf,
        source: BackupError,
    },

    RollbackFailed {
        backup_dir: PathBuf,
        source: BackupError,
    },
}

impl fmt::Display for UpgradeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreUpgradeBackupFailed { dest, source } => write!(
                f,
                "升级前备份失败，升级已被阻断（目标目录：{}）。\n\
                 原因：{source}\n\
                 请先解决上述原因再重试；不要跳过备份直接升级 —— \
                 迁移失败会留下一个无人能修的库。\n\
                 请执行 `quill doctor` 获取诊断与修复动作。",
                dest.display()
            ),
            Self::RollbackFailed { backup_dir, source } => write!(
                f,
                "回滚失败（备份目录：{}）。\n\
                 原因：{source}\n\
                 该目录是升级前的原始状态，请勿删除。\n\
                 请执行 `quill doctor` 获取诊断与修复动作。",
                backup_dir.display()
            ),
        }
    }
}

impl std::error::Error for UpgradeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PreUpgradeBackupFailed { source, .. } | Self::RollbackFailed { source, .. } => {
                Some(source)
            }
        }
    }
}

pub async fn take_pre_upgrade_backup(
    src: &BackupSource,
    dest: impl AsRef<Path>,
    app_version: impl Into<String>,
) -> Result<PreUpgradeGuard, UpgradeError> {
    let dest = dest.as_ref().to_path_buf();
    let db_path = src.db_path.clone();
    let data_root = src.data_root.clone();

    let report =
        create_backup(src, &dest)
            .await
            .map_err(|source| UpgradeError::PreUpgradeBackupFailed {
                dest: dest.clone(),
                source,
            })?;

    Ok(PreUpgradeGuard {
        backup_dir: dest,
        db_path,
        data_root,
        app_version: app_version.into(),
        report,
        last_unrestored: Vec::new(),
    })
}
