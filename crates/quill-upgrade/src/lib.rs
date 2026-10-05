//! 手动受控升级（升级前强制备份 + 回滚）。
//!
//! # 这个 crate 解决什么问题
//!
//! schema 迁移失败会**留下一个无人能修的库**（铁律六）。所以「升级」这件事
//! 在本项目里不是「跑一下迁移」，而是**先拿到一份可验证的旧状态快照**，
//! 才允许动手。本 crate 只提供这一层**安全壳**，不实现具体迁移步骤：
//!
//! ```text
//!   take_pre_upgrade_backup() ──失败──▶ 升级入口根本不会被调用
//!            │成功
//!            ▼
//!     PreUpgradeGuard ──▶ 执行迁移 ──▶ 成功：落库
//!            │                        └─ 失败：guard.rollback()
//!            ▼
//!   回到与备份时**逐字节一致**的状态
//! ```
//!
//! # 为什么"升级前备份"必须是类型的一部分，而不是一句文档要求
//!
//! 「先备份再升级」如果只写在文档里，它就是**靠人记得的规则**——
//! 而本项目没有会记得的人。所以本 crate 让它**在类型上无法跳过**：
//! 想执行升级就必须先 `take_pre_upgrade_backup()` 拿到 `PreUpgradeGuard`，
//! 拿不到 guard 就说明备份**没成功**，此时升级入口压根不存在。
//!
//! 备份本身的正确性（`VACUUM INTO` 一致性、密钥排除、摘要校验、幂等恢复）
//! 由 `quill-backup` 负责并已由它的端到端测试钉住，本 crate 不重复实现。
//!
//! # 已知边界
//!
//! - **只管安全壳，不管迁移内容**：本 crate 不认识任何具体版本号的迁移，
//!   也不校验「迁移后的 schema 与该版本是否匹配」。
//! - **不保证"回滚"能兼容新版本代码**：本 crate 恢复的是**数据**；
//!   降级运行的二进制是否认得这份数据，由调用方负责（契约里对应"用旧二进制启动并冒烟"）。
//! - **原地被改过的文件不会被回滚覆盖**：数据库是**逐字节**恢复的，但用户数据文件
//!   只在目标处没有同名文件时才写回。升级期间被**修改**（而非删除）的文件会进
//!   `not_overwritten`。这是 `quill-backup` 的刻意取舍（那个文件可能是升级后
//!   用户新建的，盲目覆盖 = 静默丢数据）。它**必须**由调用方转告用户，
//!   否则就是"回滚成功但状态没回去"的静默失败。见 [`PreUpgradeGuard::unrestored_files`]。

use quill_backup::{
    create_backup, restore_backup, BackupError, BackupReport, BackupSource, RestoreReport,
};
use std::fmt;
use std::path::{Path, PathBuf};

/// 升级前备份成功后拿到的凭据。
///
/// **持有它 = 已经拥有一份可回滚的旧状态。** 它不可凭空构造：
/// 唯一的构造路径是 [`take_pre_upgrade_backup`]，而那条路径在备份失败时
/// 直接返回 `Err`（铁律二：不许兜底、不许吞掉失败）。
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
    /// 备份目录位置（诊断输出用；`quill doctor` 报错时应打印它）。
    pub fn backup_dir(&self) -> &Path {
        &self.backup_dir
    }

    /// 执行升级时所用的版本标识。
    pub fn app_version(&self) -> &str {
        &self.app_version
    }

    /// 这次备份的报告（清单、摘要、被排除的密钥条目）。
    pub fn report(&self) -> &BackupReport {
        &self.report
    }

    /// 回滚：把数据库与用户数据恢复到**备份那一刻**的状态。
    ///
    /// # 两条保证，强度不同（读返回值，不要想当然）
    ///
    /// | 范围 | 保证 | 依据 |
    /// |---|---|---|
    /// | **数据库** | **逐字节一致**，无条件 | 快照整库替换 |
    /// | **用户数据文件** | 仅当目标处**没有**同名文件时才写回 | 见下 |
    ///
    /// 升级期间**被原地修改**（而非删除）的文件**不会**被回滚覆盖 ——
    /// 它会出现在 [`RestoreReport::not_overwritten`] 里。
    /// 这是 `quill-backup` 的刻意设计：那个文件可能是升级**之后**用户新建的，
    /// 盲目覆盖等于静默丢数据。
    ///
    /// ## ⚠️ 因此调用方**必须**处理 `not_overwritten`
    ///
    /// 「回滚成功但某个文件没被回滚」如果不说出来，就是自查表第 3 类静默失败：
    /// 用户以为回到了升级前，实际那个文件停留在损坏状态且无人告诉他。
    /// 故 [`PreUpgradeGuard::unrestored_files`] 提供一键判据，
    /// 非空即必须转告用户。
    ///
    /// # 幂等
    ///
    /// 同一 guard 上重复调用等价于调用一次（铁律七：每步幂等）。
    /// 这是**刻意**用 `&mut self` 而不是 `self`：回滚失败时调用方
    /// 仍需要拿着这个 guard 继续处置（重试、或把现场路径报给人看），
    /// 一旦 consume 掉，失败路径上就什么都留不住了。
    ///
    /// # 原子性
    ///
    /// 摘要校验**先于**任何写入（由 `quill-backup` 保证），
    /// 所以校验不过时目标目录一个字节都没被改 —— 回滚失败 ≠ 半个损坏的库。
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

    /// 最近一次回滚中**没有被恢复**的文件（升级期间原地被改过的）。
    ///
    /// 非空 ⇒ 状态并未完全回到升级前，**必须**转告用户。
    ///
    /// ⚠️ 该列表**只增不减**：一旦某个文件被标记，它不会因为后续某次回滚
    /// 碰巧成功而自动消失。理由是"没人处理过它"这一事实不会因重试而改变 ——
    /// 让它自动清空等于让静默失败在第二次回滚后重新隐形。
    pub fn unrestored_files(&self) -> &[String] {
        &self.last_unrestored
    }
}

/// 升级流程的错误类型。
///
/// ⚠️ 错误信息里**一定**带可复制的路径：用户在这条路径上唯一需要执行的
/// 命令是 `quill doctor`，它要能直接定位到现场（铁律七）。
#[derive(Debug)]
pub enum UpgradeError {
    /// 升级前备份失败 —— 此时**升级必须没有开始**。
    PreUpgradeBackupFailed { dest: PathBuf, source: BackupError },
    /// 回滚失败。`backup_dir` 一并带出，供人工介入。
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

/// 执行升级前**强制**做一次一致性备份，成功才交出 [`PreUpgradeGuard`]。
///
/// # Errors
///
/// 备份失败（源路径不存在、目标非空、SQLite/IO 失败）时返回
/// [`UpgradeError::PreUpgradeBackupFailed`]，**且不构造任何 guard** ——
/// 即"没有可回滚快照"这个状态在类型上不可表示。
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
