//! 错误类型：中文人话 + **可直接复制的修复命令**。
//!
//! # 铁律七：失败必须自诊断
//!
//! 唯一验收者（用户）不参与日常开发，所以任何失败都必须能**自己说出下一步**。
//! 本模块的每个变体都满足三条：
//! 1. [`std::fmt::Display`] 是中文人话，不把 `EACCES` / `SQLITE_BUSY` 抛给人；
//! 2. [`BackupError::fix_command`] 给出**一整条可直接复制的命令**（不是「请检查权限」）；
//! 3. 修复命令**幂等**（同一命令跑两次不会把事情弄得更糟）。
//!
//! # 为什么自己写而不引 `thiserror`
//!
//! `thiserror` 已在 `Cargo.lock` 里，但这里的每个变体都要**自定义 `Display`**——
//! 而「中文人话 + 修复命令」不是 derive 能生成的样板。
//! 手写 6 个 `impl` 比引一个 derive 宏更直白，也让 `fix_command` 与 `Display`
//! 写在同一个 `match` 里、不可能对不上。
//!
//! ⚠️ **size 纪律**：变体里超过一个字段的都用 `Box` 包。
//! `BackupError` 会出现在每个 `Result` 的错误位，若枚举超过 128 字节，
//! `clippy::result_large_err` 会让**每个调用点**的返回值都变大。
//! `sqlx::Error` 本身就是大枚举，故显式 `Box` 起来。

use std::fmt;
use std::io;
use std::path::Path;

/// 备份 / 恢复失败。
///
/// ⚠️ 变体里只放**定位所需**的信息，不放「已经算好的结论」——
/// 结论由 [`BackupError::fix_command`] 在渲染时算出，避免两处描述漂移。
#[derive(Debug)]
pub enum BackupError {
    /// 源库文件不存在。
    DbMissing {
        /// 期望的库文件路径。
        path: Box<Path>,
    },
    /// 用户数据目录不存在。
    DataRootMissing {
        /// 期望存在的数据目录路径。
        path: Box<Path>,
    },
    /// 备份目标目录已存在且非空（拒绝覆盖，避免毁掉上一份好备份）。
    DestNotEmpty {
        /// 已存在的目标目录。
        path: Box<Path>,
    },
    /// 文件系统操作失败（权限、磁盘满、路径不存在等）。
    Io {
        /// 正在做的事，中文（如「创建备份目录」）。
        what: String,
        /// 出问题的路径。
        path: Box<Path>,
        /// 底层原因。
        source: Box<io::Error>,
    },
    /// SQLite 操作失败。
    Sqlx {
        /// 触发失败的语句/操作，中文描述。
        stmt: String,
        /// 底层原因。
        source: Box<sqlx::Error>,
    },
    /// 备份目录里没有清单文件。
    ManifestMissing {
        /// 期望的清单路径。
        path: Box<Path>,
    },
    /// 清单某一行解析不出来。
    ManifestBadLine {
        /// 行号（1 起）。
        line_no: usize,
        /// 该行的内容（截断到 120 字符）。
        content: String,
        /// 原因，中文。
        reason: String,
    },
    /// 清单版本不是本程序认识的版本。
    ManifestBadVersion {
        /// 清单里写的版本串。
        found: String,
    },
    /// 清单里同一个键出现两次。
    ManifestDupKey {
        /// 重复的键名。
        key: String,
    },
    /// 清单里出现本程序不认识的键。
    ///
    /// ⚠️ **严格模式**：未知键一律判红。
    /// 宽容解析（忽略未知键）会让「清单被另一个版本改过」变成静默通过——
    /// 恢复时少还原一批文件而不报错，正是 `AGENTS.md` 自查表第 3 类静默失败。
    ManifestUnknownKey {
        /// 不认识的键名。
        key: String,
    },
    /// 清单里的相对路径不安全。
    UnsafePath {
        /// 清单里写的原始串。
        raw: String,
        /// 为什么危险，中文。
        why: &'static str,
    },
    /// 内容摘要与清单记录不符。
    DigestMismatch {
        /// 相对路径。
        rel: String,
        /// 清单里记的摘要。
        expect: String,
        /// 实际算出的摘要。
        actual: String,
    },
    /// 文件大小与清单记录不符。
    SizeMismatch {
        /// 相对路径。
        rel: String,
        /// 清单里记的字节数。
        expect: u64,
        /// 实际字节数。
        actual: u64,
    },
    /// 清单声明的文件数与实际列出的行数不符。
    FileCountMismatch {
        /// 清单头里声明的数量。
        declared: usize,
        /// 实际解析出的条目数。
        actual: usize,
    },
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DbMissing { path } => {
                write!(f, "找不到要备份的数据库文件：{}", path.display())
            }
            Self::DataRootMissing { path } => {
                write!(f, "找不到用户数据目录：{}", path.display())
            }
            Self::DestNotEmpty { path } => write!(
                f,
                "备份目标目录已存在且非空：{}（拒绝覆盖，否则会毁掉上一份好备份）",
                path.display()
            ),
            Self::Io { what, path, source } => {
                write!(f, "{what}失败：{}（{source}）", path.display())
            }
            Self::Sqlx { stmt, source } => write!(f, "{stmt}失败：{source}"),
            Self::ManifestMissing { path } => {
                write!(f, "备份目录里没有清单文件：{}", path.display())
            }
            Self::ManifestBadLine {
                line_no,
                content,
                reason,
            } => write!(f, "清单第 {line_no} 行无法解析：{reason}（该行内容：{content}）"),
            Self::ManifestBadVersion { found } => write!(
                f,
                "清单版本是「{found}」，本程序只认 v1 —— 备份可能由更新版本的 quill 产生"
            ),
            Self::ManifestDupKey { key } => {
                write!(f, "清单里的键「{key}」出现了两次（清单被重复拼接或手工改坏）")
            }
            Self::ManifestUnknownKey { key } => write!(
                f,
                "清单里有本程序不认识的键「{key}」—— 拒绝按未知格式恢复（否则会静默少还原文件）"
            ),
            Self::UnsafePath { raw, why } => {
                write!(f, "清单里的路径「{raw}」不安全：{why}")
            }
            Self::DigestMismatch {
                rel,
                expect,
                actual,
            } => write!(
                f,
                "文件「{rel}」的内容摘要与清单不符（清单记 {expect}，实际 {actual}）—— 文件已损坏或被改过"
            ),
            Self::SizeMismatch {
                rel,
                expect,
                actual,
            } => write!(
                f,
                "文件「{rel}」的大小与清单不符（清单记 {expect} 字节，实际 {actual} 字节）"
            ),
            Self::FileCountMismatch { declared, actual } => write!(
                f,
                "清单头声明要备份 {declared} 个文件，实际只列出 {actual} 个 —— 清单不完整"
            ),
        }
    }
}

impl std::error::Error for BackupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source.as_ref()),
            Self::Sqlx { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl BackupError {
    /// 返回**一整条可直接复制执行**的修复命令。
    ///
    /// 铁律七：不给「请检查权限」这种要用户自己想下一步的说法。
    /// 拿不到确定的修复路径时返回 `None` —— 那属于「必须停下来问人」，
    /// 而不是一个编出来的命令。**宁可少给，不给错命令**（错命令会让情况更糟）。
    pub fn fix_command(&self) -> Option<String> {
        match self {
            Self::DbMissing { path } => Some(format!(
                "ls -l '{}'   # 先确认库文件真的在这个路径；\
                 若在别处，改用正确路径重新执行 quill backup",
                path.display()
            )),
            Self::DataRootMissing { path } => Some(format!(
                "ls -la '{}'   # 确认数据目录；\
                 若尚未创建，先执行 quill doctor 走首次初始化",
                path.display()
            )),
            Self::DestNotEmpty { path } => Some(format!(
                "ls -la '{}'   # 确认里面没有需要保留的备份；\
                 确认无误后执行 mv '{}' '{}.old'   # 移开而不是删，删了不可逆",
                path.display(),
                path.display(),
                path.display()
            )),
            Self::Io { what, path, .. } => {
                let p = path.display();
                Some(match self {
                    Self::Io { source, .. } if source.kind() == io::ErrorKind::PermissionDenied => {
                        format!(
                            "ls -ld '{p}'   # 看清属主；\
                         若服务以 quill 用户运行，执行 sudo chown -R quill:quill '{p}'"
                        )
                    }
                    Self::Io { source, .. } if source.kind() == io::ErrorKind::NotFound => {
                        format!(
                            "ls -ld '{p}'   # 路径不存在；\
                                上一级目录也要在，执行 ls -ld \"$(dirname '{p}')\""
                        )
                    }
                    _ => format!(
                        "df -h '{p}'   # 先看磁盘；\
                                  空间不足时清理旧备份后重试"
                    ),
                })
                .map(|cmd| format!("{what} 失败时先跑：{cmd}"))
            }
            Self::Sqlx { .. } => Some(
                "systemctl status quill-server   # 确认数据库没被别的进程写坏；\
                 然后执行 quill doctor   # doctor 会检查库完整性并给出下一步"
                    .to_string(),
            ),
            Self::ManifestMissing { path } => Some(format!(
                "ls -la '{}'   # 目录里应有 MANIFEST 与 db/、data/；\
                 若被手工删过，这份备份已不可信，请改用另一份备份",
                path.display()
            )),
            Self::ManifestBadLine { .. }
            | Self::ManifestBadVersion { .. }
            | Self::ManifestDupKey { .. }
            | Self::ManifestUnknownKey { .. } => Some(
                "这份备份的清单已损坏，不要用它恢复。\
                 执行 ls -la <备份目录> 找一份完整的备份；若只有这一份，\
                 手工核对后用编辑器修好清单再重试"
                    .to_string(),
            ),
            Self::UnsafePath { .. } => Some(
                "拒绝恢复：清单里的路径会写到目标目录之外，这是安全红线，不可绕过。\
                 请改用由 quill 自己生成的备份"
                    .to_string(),
            ),
            Self::DigestMismatch { rel, .. } | Self::SizeMismatch { rel, .. } => Some(format!(
                "sha256sum '<备份目录>/{}'   # 确认文件确已损坏；\
                 损坏的备份无法恢复，请换一份完整备份。\
                 切勿跳过校验强行恢复 —— 那会装上一份内容不明的库",
                rel
            )),
            Self::FileCountMismatch { .. } => Some(
                "清单头与清单体不一致，说明备份过程中断或清单被截断。\
                 请换一份完整备份；不要手工补齐条目"
                    .to_string(),
            ),
        }
    }

    /// 渲染成「问题 + 修复命令」的完整人话输出，供 `quill doctor` / CLI 直接打印。
    pub fn render_with_fix(&self) -> String {
        match self.fix_command() {
            Some(cmd) => format!("{self}\n\n请执行：\n  {cmd}"),
            None => self.to_string(),
        }
    }
}

/// 便捷构造：带路径的 IO 错误。
pub(crate) fn io_err(what: &str, path: &Path, source: io::Error) -> BackupError {
    BackupError::Io {
        what: what.to_string(),
        // ⚠️ `PathBuf: Into<Box<Path>>` 是标准库提供的收缩转换，
        //    手工包一层 Box<Path> 反而会多出一次分配。
        path: path.to_path_buf().into(),
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 把 `&str` 变成变体里要的 `Box<Path>`。
    ///
    /// ⚠️ 之所以要有这个辅助函数：变体存 `Box<Path>` 是为了压枚举体积
    /// （见模块文档），代价是测试里构造变体啰嗦。
    /// 啰嗦只发生在测试里 —— 换来的是生产代码里每个 `Result` 都不背一个大枚举。
    fn bp(p: &str) -> Box<Path> {
        PathBuf::from(p).into()
    }

    /// 构造一个 `Io` 变体（`io_err` 是 `pub(crate)`，同 crate 的测试可直接调）。
    fn io(what: &str, p: &str, kind: io::ErrorKind) -> BackupError {
        io_err(what, Path::new(p), io::Error::new(kind, "boom"))
    }

    #[test]
    fn every_variant_yields_chinese_message() {
        // 逐个变体都要能渲染出中文人话 —— 漏一个就是「用户看到英文异常」。
        let cases: Vec<BackupError> = vec![
            BackupError::DbMissing {
                path: bp("/x/quill.db"),
            },
            BackupError::DataRootMissing {
                path: bp("/x/data"),
            },
            BackupError::DestNotEmpty { path: bp("/x/bk") },
            io("创建备份目录", "/x/bk", io::ErrorKind::Other),
            BackupError::ManifestMissing {
                path: bp("/x/bk/MANIFEST"),
            },
            BackupError::ManifestBadLine {
                line_no: 7,
                content: "garbage".into(),
                reason: "缺少分隔符".into(),
            },
            BackupError::ManifestBadVersion { found: "v2".into() },
            BackupError::ManifestDupKey {
                key: "db.sha256".into(),
            },
            BackupError::ManifestUnknownKey {
                key: "db.md5".into(),
            },
            BackupError::UnsafePath {
                raw: "../../etc/passwd".into(),
                why: "含上级目录",
            },
            BackupError::DigestMismatch {
                rel: "data/a/wiki/x.md".into(),
                expect: "a".into(),
                actual: "b".into(),
            },
            BackupError::SizeMismatch {
                rel: "data/a/wiki/x.md".into(),
                expect: 1,
                actual: 2,
            },
            BackupError::FileCountMismatch {
                declared: 3,
                actual: 2,
            },
        ];
        for e in &cases {
            let s = e.to_string();
            assert!(!s.is_empty(), "{e:?} 渲染成了空串");
            // 至少要含一个非 ASCII 字符 —— 即真的是中文人话而非透传底层英文。
            assert!(
                s.chars().any(|c| c as u32 > 0x2E80),
                "{e:?} 的消息不含中文：{s}"
            );
        }
    }

    #[test]
    fn fix_commands_are_present_and_absolute_for_diagnosable_cases() {
        let e = BackupError::DataRootMissing {
            path: bp("/srv/quill/data"),
        };
        let cmd = e.fix_command().expect("应有修复命令");
        assert!(
            cmd.contains("/srv/quill/data"),
            "命令里必须带出问题路径：{cmd}"
        );

        // 权限类要给 chown —— NAS 上最常见的一类
        let e = io(
            "写备份文件",
            "/srv/quill/bk",
            io::ErrorKind::PermissionDenied,
        );
        let cmd = e.fix_command().expect("应有修复命令");
        assert!(cmd.contains("chown"), "权限问题应给 chown 命令：{cmd}");

        // 磁盘满要给 df
        let e = io("写备份文件", "/srv/quill/bk", io::ErrorKind::StorageFull);
        assert!(
            e.fix_command().expect("应有").contains("df"),
            "磁盘满应给 df 命令"
        );
    }

    #[test]
    fn security_violations_refuse_instead_of_offering_a_bypass() {
        // 路径逃逸是安全红线：修复命令不得出现「继续」/「跳过」这类绕过措辞。
        let e = BackupError::UnsafePath {
            raw: "../../etc/passwd".into(),
            why: "含上级目录",
        };
        let rendered = e.render_with_fix();
        assert!(rendered.contains("拒绝"), "{rendered}");
        for bad in ["跳过", "继续", "--force", "忽略"] {
            assert!(
                !rendered.contains(bad),
                "修复命令不得提供绕过手法：{rendered}"
            );
        }
    }

    #[test]
    fn render_with_fix_includes_the_command() {
        let e = BackupError::DestNotEmpty { path: bp("/bk") };
        let s = e.render_with_fix();
        assert!(s.contains("请执行"), "{s}");
        assert!(s.contains("mv"), "移开而非删除（删了不可逆）：{s}");
    }

    #[test]
    fn error_is_small_enough_to_not_inflate_every_result() {
        // 铁律：BackupError 出现在每个 Result 的错误位。
        // 超过 128 字节会让 clippy::result_large_err 报红，也会让每个调用点的
        // 返回值变大 —— 这里钉住上限，将来加字段时会被立刻发现。
        let s = std::mem::size_of::<BackupError>();
        assert!(
            s <= 128,
            "BackupError 有 {s} 字节，超过 128 上限：把大字段 Box 起来"
        );
    }
}
