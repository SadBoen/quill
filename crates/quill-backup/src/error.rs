use std::fmt;
use std::io;
use std::path::Path;

#[derive(Debug)]
pub enum BackupError {
    DbMissing {
        path: Box<Path>,
    },

    DataRootMissing {
        path: Box<Path>,
    },

    DestNotEmpty {
        path: Box<Path>,
    },

    Io {
        what: String,

        path: Box<Path>,

        source: Box<io::Error>,
    },

    Sqlx {
        stmt: String,

        source: Box<sqlx::Error>,
    },

    ManifestMissing {
        path: Box<Path>,
    },

    ManifestBadLine {
        line_no: usize,

        content: String,

        reason: String,
    },

    ManifestBadVersion {
        found: String,
    },

    ManifestDupKey {
        key: String,
    },

    ManifestUnknownKey {
        key: String,
    },

    UnsafePath {
        raw: String,

        why: &'static str,
    },

    DigestMismatch {
        rel: String,

        expect: String,

        actual: String,
    },

    SizeMismatch {
        rel: String,

        expect: u64,

        actual: u64,
    },

    FileCountMismatch {
        declared: usize,

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

    pub fn render_with_fix(&self) -> String {
        match self.fix_command() {
            Some(cmd) => format!("{self}\n\n请执行：\n  {cmd}"),
            None => self.to_string(),
        }
    }
}

pub(crate) fn io_err(what: &str, path: &Path, source: io::Error) -> BackupError {
    BackupError::Io {
        what: what.to_string(),

        path: path.to_path_buf().into(),
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn bp(p: &str) -> Box<Path> {
        PathBuf::from(p).into()
    }

    fn io(what: &str, p: &str, kind: io::ErrorKind) -> BackupError {
        io_err(what, Path::new(p), io::Error::new(kind, "boom"))
    }

    #[test]
    fn every_variant_yields_chinese_message() {
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

        let e = io(
            "写备份文件",
            "/srv/quill/bk",
            io::ErrorKind::PermissionDenied,
        );
        let cmd = e.fix_command().expect("应有修复命令");
        assert!(cmd.contains("chown"), "权限问题应给 chown 命令：{cmd}");

        let e = io("写备份文件", "/srv/quill/bk", io::ErrorKind::StorageFull);
        assert!(
            e.fix_command().expect("应有").contains("df"),
            "磁盘满应给 df 命令"
        );
    }

    #[test]
    fn security_violations_refuse_instead_of_offering_a_bypass() {
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
        let s = std::mem::size_of::<BackupError>();
        assert!(
            s <= 128,
            "BackupError 有 {s} 字节，超过 128 上限：把大字段 Box 起来"
        );
    }
}
