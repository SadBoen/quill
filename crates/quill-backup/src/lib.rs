//! `quill-backup` —— 一致性备份与恢复。
//!
//! # 解决的问题
//!
//! V1 需求 6（`docs/V1_SCOPE_CONSTRAINTS.md` 第一节）：
//! **一键完整导出 + 从备份完整恢复（自动化测试覆盖）**。
//!
//! # 三条不可让步的约束
//!
//! ## 1. 一致性来自 `VACUUM INTO`，不是 `cp`（铁律三）
//!
//! WAL 模式下 `cp quill.db` 会丢掉 `-wal` 里**已提交**的数据，
//! **且不报任何错** —— 恢复出来的是一个「看着正常、少了数据」的库。
//! `VACUUM INTO` 由 SQLite 在只读事务里把主库 + WAL 合成一个自洽文件。
//!
//! ## 2. 备份产物不得含凭据明文（R4）
//!
//! `secrets.enc` **是密文，原样搬运**（无 master key 时不可解），
//! 且它是「完整恢复」的前提 —— 不带它恢复出来的实例没有对端凭据。
//! 真正被排除的是**密钥材料**（`master.key` / `*.pem` / `*.key` …），
//! 见 [`manifest::is_forbidden_in_backup`]。
//! ⚠️ 注意排除清单**不含** `secrets.enc`：把它排除会让
//! `scripts/quill-doctor-roaming.sh` 的 R4 判红，同时让恢复不完整。
//!
//! ## 3. 恢复必须幂等且不留半开状态（铁律七）
//!
//! - 同一份备份恢复两次 ≡ 恢复一次；
//! - 摘要校验**先于**任何写入 —— 校验不过时目标目录一个字节都没被改；
//! - 每个文件先写 `.part` 再 `rename`（原子）；
//! - 恢复数据库前先删 `-wal`/`-shm`（旧 WAL 配新主库 = 静默错乱）。
//!
//! # 已知边界：空目录不进清单
//!
//! 清单记录的是**文件**，不记录空目录；恢复时按文件路径重建其父目录，
//! 因此**空目录不会被重建**。有文件内容的目录全部保留。
//! 该边界由 `tests/end_to_end.rs` 的
//! `空目录不进清单恢复后不重建但有文件内容的目录都在` 钉住 ——
//! 若将来改成保留空目录，那条用例会红，提示同步更新本段。
//!
//! # 压缩降级（重要）
//!
//! **本 crate 不做压缩。** 铁律三要求用 `zstd` crate，而 `zstd`
//! **不在 `Cargo.lock` 里**，引入它会新增 `[[package]]` 条目，
//! 违反 `docs/FACTS.md` 第三节的「`Cargo.lock` 零新增外部 crate」不变式。
//! 故按任务裁决降级为**不压缩**。影响：备份体积等于原始数据量。
//! 补齐条件：`zstd` 进入依赖树后，在 [`backup::create_backup`]
//! 落一份 `.tar.zst` 即可，清单格式不必改。
//!
//! # 依赖说明
//!
//! 允许依赖 `quill-adapters` + `quill-domain`（契约 §一）；
//! 本实现额外直接依赖 `quill-store`（拿 `SqlitePool` 跑 `VACUUM INTO`）
//! 与 `sqlx`（发 SQL）与 `sha2`（SHA-256）。
//! 后两者**都已在 `Cargo.lock` 里**（`sqlx` 由 `quill-store` 引入、
//! `sha2` 由 `sqlx-core` 引入），此处只是把已在编译单元里的依赖写成直接依赖
//! —— 判据：`git diff -- Cargo.lock` 不会新增 `[[package]]`。
//! **未新增任何外部 crate**。

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

pub mod backup;
pub mod digest;
pub mod error;
pub mod manifest;

pub use backup::{create_backup, restore_backup, BackupReport, BackupSource, RestoreReport};
pub use digest::{is_digest_hex, sha256_bytes, sha256_file, DIGEST_HEX_LEN};
pub use error::BackupError;
pub use manifest::{ExcludedEntry, Manifest, ManifestEntry, MANIFEST_NAME, MANIFEST_VERSION};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_compiles_and_test_runs() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn public_surface_is_reachable() {
        // 防「模块建了但没导出」—— 外部 crate 只能看到 re-export 的东西。
        assert_eq!(MANIFEST_NAME, "MANIFEST");
        assert_eq!(MANIFEST_VERSION, "v1");
        assert!(is_digest_hex(&sha256_bytes(b"quill")));
    }
}
