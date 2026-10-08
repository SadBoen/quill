use std::io;
use std::path::{Component, Path, PathBuf};

use quill_adapters::UserId;

use crate::page::{parse_page, Page};

pub const DIR_RAW: &str = "raw";

pub const DIR_WIKI: &str = "wiki";

pub const DIR_SCHEMA: &str = "schema";

pub const INDEX_FILE: &str = "index.md";

pub const LOG_FILE: &str = "log.md";

#[derive(Debug)]
pub enum WikiError {
    PathEscape {
        attempt: String,
        reason: &'static str,
    },

    InvalidPath {
        attempt: String,
        reason: &'static str,
    },

    Io {
        path: PathBuf,
        source: io::Error,
    },

    NotUtf8 {
        path: PathBuf,
    },

    MalformedIndex(String),

    Backend(String),

    NotFound(String),
}

impl std::fmt::Display for WikiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PathEscape { attempt, reason } => {
                write!(f, "路径 {attempt:?} 越出当前用户的 wiki 根目录（{reason}）")
            }
            Self::InvalidPath { attempt, reason } => {
                write!(f, "路径 {attempt:?} 形态非法（{reason}）")
            }
            Self::Io { path, source } => {
                write!(f, "文件操作失败：{}：{source}", path.display())
            }
            Self::NotUtf8 { path } => {
                write!(f, "文件不是合法 UTF-8：{}", path.display())
            }
            Self::MalformedIndex(s) => write!(f, "index.md 形态非法：{s}"),
            Self::Backend(s) => write!(f, "知识后端失败：{s}"),
            Self::NotFound(s) => write!(f, "目标不存在：{s}"),
        }
    }
}

impl std::error::Error for WikiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<WikiError> for quill_adapters::AdapterError {
    fn from(e: WikiError) -> Self {
        match e {
            WikiError::PathEscape { .. } | WikiError::InvalidPath { .. } => {
                Self::Forbidden(e.to_string())
            }
            WikiError::NotUtf8 { .. } | WikiError::MalformedIndex(_) => {
                Self::Internal(e.to_string())
            }

            WikiError::Backend(_) => Self::Provider(e.to_string()),
            WikiError::NotFound(_) => Self::NotFound(e.to_string()),
            WikiError::Io { .. } => Self::Storage(e.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WikiStore {
    base: PathBuf,
    user: UserId,
}

impl WikiStore {
    pub fn new(base: impl AsRef<Path>, user: UserId) -> Self {
        Self {
            base: base.as_ref().to_path_buf(),
            user,
        }
    }

    pub fn user(&self) -> UserId {
        self.user
    }

    pub fn root(&self) -> PathBuf {
        self.base.join("wiki").join(self.user.to_compact_hex())
    }

    pub fn layer(&self, layer: &str) -> Result<PathBuf, WikiError> {
        match layer {
            DIR_RAW | DIR_WIKI | DIR_SCHEMA => Ok(self.root().join(layer)),
            other => Err(WikiError::InvalidPath {
                attempt: other.to_string(),
                reason: "层名只允许 raw / wiki / schema",
            }),
        }
    }

    pub fn resolve(&self, layer: &str, rel: &str) -> Result<PathBuf, WikiError> {
        let root = self.layer(layer)?;
        if rel.trim().is_empty() {
            return Err(WikiError::InvalidPath {
                attempt: rel.to_string(),
                reason: "路径为空",
            });
        }

        let p = Path::new(rel);
        if p.is_absolute() || rel.starts_with('/') || rel.starts_with('\\') {
            return Err(WikiError::PathEscape {
                attempt: rel.to_string(),
                reason: "不接受绝对路径",
            });
        }

        let mut depth: i32 = 0;
        for c in p.components() {
            match c {
                Component::Normal(seg) => {
                    let s = seg.to_string_lossy();

                    if s.contains(':') {
                        return Err(WikiError::InvalidPath {
                            attempt: rel.to_string(),
                            reason: "路径段含冒号（盘符或 ADS）",
                        });
                    }
                    depth += 1;
                }
                Component::ParentDir => {
                    depth -= 1;
                    if depth < 0 {
                        return Err(WikiError::PathEscape {
                            attempt: rel.to_string(),
                            reason: "含 `..` 上跳",
                        });
                    }
                }
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => {
                    return Err(WikiError::PathEscape {
                        attempt: rel.to_string(),
                        reason: "不接受根/前缀",
                    })
                }
            }
        }
        let joined = root.join(p);

        let norm_root = normalize(&root);
        let norm_joined = normalize(&joined);
        if !norm_joined.starts_with(&norm_root) {
            return Err(WikiError::PathEscape {
                attempt: rel.to_string(),
                reason: "规范化后越出根目录",
            });
        }
        Ok(joined)
    }

    pub fn ensure_layout(&self) -> Result<(), WikiError> {
        for l in [DIR_RAW, DIR_WIKI, DIR_SCHEMA] {
            std::fs::create_dir_all(self.layer(l)?).map_err(|source| WikiError::Io {
                path: self.layer(l).unwrap_or_else(|_| self.root()),
                source,
            })?;
        }
        Ok(())
    }

    pub fn read_page(&self, rel: &str) -> Result<Page, WikiError> {
        let p = self.resolve(DIR_WIKI, rel)?;
        // 缺页是「资源不存在」，不是 I/O 故障：混进 Io 会让调用方把 404 报成 500。
        let text = match read_utf8(&p) {
            Ok(t) => t,
            Err(WikiError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                return Err(WikiError::NotFound(rel.to_string()))
            }
            Err(other) => return Err(other),
        };
        Ok(parse_page(rel, &text))
    }

    pub fn write_page(&self, rel: &str, content: &str) -> Result<(), WikiError> {
        let p = self.resolve(DIR_WIKI, rel)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|source| WikiError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        write_atomic(&p, content)
    }

    pub fn write_raw(&self, rel: &str, content: &str) -> Result<(), WikiError> {
        let p = self.resolve(DIR_RAW, rel)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|source| WikiError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        write_atomic(&p, content)
    }

    pub fn read_raw(&self, rel: &str) -> Result<String, WikiError> {
        let p = self.resolve(DIR_RAW, rel)?;
        read_utf8(&p)
    }

    pub fn read_schema(&self, rel: &str) -> Result<Option<String>, WikiError> {
        let p = self.resolve(DIR_SCHEMA, rel)?;
        match std::fs::read(&p) {
            Ok(b) => String::from_utf8(b)
                .map(Some)
                .map_err(|_| WikiError::NotUtf8 { path: p }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(WikiError::Io { path: p, source }),
        }
    }

    pub fn list_pages(&self) -> Result<Vec<String>, WikiError> {
        let root = self.layer(DIR_WIKI)?;
        let mut out = Vec::new();
        collect_md(&root, &root, &mut out)?;
        out.sort();
        Ok(out)
    }

    pub fn load_all_pages(&self) -> Result<Vec<Page>, WikiError> {
        let mut pages = Vec::new();
        for rel in self.list_pages()? {
            pages.push(self.read_page(&rel)?);
        }
        Ok(pages)
    }

    pub fn read_index(&self) -> Result<Option<String>, WikiError> {
        self.read_page_optional(INDEX_FILE)
    }

    pub fn write_index(&self, content: &str) -> Result<(), WikiError> {
        self.write_page(INDEX_FILE, content)
    }

    pub fn read_log(&self) -> Result<Option<String>, WikiError> {
        self.read_page_optional(LOG_FILE)
    }

    pub fn append_log(&self, addition: &str) -> Result<(), WikiError> {
        let p = self.resolve(DIR_WIKI, LOG_FILE)?;
        let existing = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(source) => return Err(WikiError::Io { path: p, source }),
        };

        let mut buf = existing;
        if !buf.is_empty() && !buf.ends_with("\n\n") {
            if buf.ends_with('\n') {
                buf.push('\n');
            } else {
                buf.push_str("\n\n");
            }
        }
        buf.push_str(addition);
        if !buf.ends_with('\n') {
            buf.push('\n');
        }
        write_atomic(&p, &buf)
    }

    fn read_page_optional(&self, rel: &str) -> Result<Option<String>, WikiError> {
        let p = self.resolve(DIR_WIKI, rel)?;
        match std::fs::read(&p) {
            Ok(b) => String::from_utf8(b)
                .map(Some)
                .map_err(|_| WikiError::NotUtf8 { path: p }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(WikiError::Io { path: p, source }),
        }
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn read_utf8(p: &Path) -> Result<String, WikiError> {
    let b = std::fs::read(p).map_err(|source| WikiError::Io {
        path: p.to_path_buf(),
        source,
    })?;
    String::from_utf8(b).map_err(|_| WikiError::NotUtf8 {
        path: p.to_path_buf(),
    })
}

fn write_atomic(p: &Path, content: &str) -> Result<(), WikiError> {
    let tmp = p.with_extension("md.tmp");
    std::fs::write(&tmp, content).map_err(|source| WikiError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, p).map_err(|source| {
        let _ = std::fs::remove_file(&tmp);
        WikiError::Io {
            path: p.to_path_buf(),
            source,
        }
    })
}

fn collect_md(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), WikiError> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(WikiError::Io {
                path: dir.to_path_buf(),
                source,
            })
        }
    };
    for entry in rd {
        let entry = entry.map_err(|source| WikiError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let p = entry.path();
        let ft = entry.file_type().map_err(|source| WikiError::Io {
            path: p.clone(),
            source,
        })?;
        if ft.is_dir() {
            collect_md(root, &p, out)?;
        } else if ft.is_file() && p.extension().is_some_and(|e| e == "md") {
            let rel = p
                .strip_prefix(root)
                .map_err(|_| WikiError::PathEscape {
                    attempt: p.display().to_string(),
                    reason: "枚举结果不在 wiki 根内（不应发生）",
                })?
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(n: u8) -> UserId {
        let mut b = [0u8; 16];
        b[15] = n;
        UserId::from_bytes(b)
    }

    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "quill-wiki-store-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("建临时根");
        p
    }

    #[test]
    fn resolve_accepts_nested_relative_paths() {
        let s = WikiStore::new("/base", user(1));
        let p = s.resolve(DIR_WIKI, "concepts/a/b.md").expect("应放行");
        assert!(p.ends_with("concepts/a/b.md"));
    }

    #[test]
    fn resolve_rejects_traversal_and_absolute() {
        let s = WikiStore::new("/base", user(1));
        for bad in [
            "../other-user/secret.md",
            "concepts/../../other/secret.md",
            "/etc/passwd",
            "\\\\server\\share\\x.md",
            "",
            "   ",
        ] {
            let r = s.resolve(DIR_WIKI, bad);
            assert!(r.is_err(), "越权路径 {bad:?} 未被拒绝");
        }
    }

    #[test]
    fn resolve_rejects_unknown_layer() {
        let s = WikiStore::new("/base", user(1));
        assert!(s.layer("etc").is_err());
        assert!(s.resolve("etc", "a.md").is_err());
    }

    /// 路径安全的**边界批量**（queue Q060）。
    ///
    /// 两条互补的断言，合起来才是「越界一律拒」的完整判据：
    /// 1. **每个输入都不得落到根外面** —— 对 `Ok` 也要把返回路径规范化后比对根前缀，
    ///    而不是只看 `is_err()`。只断言 `is_err` 的写法会漏掉「放行了但放到了别处」
    ///    这种更糟的错；
    /// 2. 点名的那几个必须**直接拒**（不靠「恰好没跑出去」）。
    ///
    /// **为什么跨平台分开写**：`..\other` 在 Unix 是一个普通文件名（反斜杠不是分隔符），
    /// 在 Windows 是两级上跳。所以这里不假设某一侧的具体结果，只测两边都该成立的不变量；
    /// Windows 侧另有一条 `#[cfg(windows)]` 钉住它确实拒。
    #[test]
    fn resolve_never_lands_outside_the_root() {
        let s = WikiStore::new("/base", user(1));
        // 判据的基准是**层目录**：`resolve(layer, rel)` 是相对 `root/<layer>` 解析的，
        // 拿 `root()` 去比会宽出一层，即便真跑到了 `raw/` 里也照样绿。
        let layer_root = normalize(&s.layer(DIR_WIKI).expect("wiki 层"));
        let cases = [
            // 老测试覆盖过的：必须直接拒。
            "../other-user/secret.md",
            "concepts/../../other/secret.md",
            "/etc/passwd",
            "\\\\server\\share\\x.md",
            "",
            "   ",
            // Q060 补的边界。
            "C:x",                      // 盘符相对（冒号）
            "c:/windows/system32/x.md", // 盘符绝对
            "dir/C:x.md",               // 非首段的冒号（ADS / 盘符）
            "/",                        // 只给分隔符
            "//",                       // 双分隔符
            "a/../../b",                // 中途越出
            "..",                       // 就是父目录
            "../..",
            "sub/../..",   // 深度恰好归零后再上跳
            "concepts/..", // 合法：归零仍在根内（见下方断言）
            "a//b.md",     // 重复分隔符
            "./a.md",      // 当前目录前缀
            "a/./b.md",
            "\\etc\\passwd",        // 反斜杠开头（Windows 绝对路径）
            "..\\other\\secret.md", // 反斜杠上跳（Windows 才是上跳）
        ];

        for case in cases {
            match s.resolve(DIR_WIKI, case) {
                Err(_) => {}
                Ok(path) => {
                    let resolved = normalize(&path);
                    assert!(
                        resolved.starts_with(&layer_root),
                        "路径 {case:?} 被放行，但规范化后落到了层目录外面：{resolved:?}"
                    );
                }
            }
        }

        // 点名必须直接拒的那几个。
        for must_reject in [
            "../other-user/secret.md",
            "concepts/../../other/secret.md",
            "a/../../b",
            "..",
            "../..",
            "/etc/passwd",
            "C:x",
            "c:/windows/system32/x.md",
            "dir/C:x.md",
            "/",
            "//",
            "\\etc\\passwd",
        ] {
            assert!(
                s.resolve(DIR_WIKI, must_reject).is_err(),
                "{must_reject:?} 必须直接拒"
            );
        }

        // 合法的那种也要钉住：`concepts/..` 归零后仍在根内 —— 拒掉它属于过度拦截，
        // 会让「列出目录再按相对路径回读」这类正常用法失效。
        let ok = s
            .resolve(DIR_WIKI, "concepts/..")
            .expect("归零后仍在根内，应放行");
        assert_eq!(normalize(&ok), layer_root, "concepts/.. 应当就是 wiki 层根");
    }

    /// Windows 侧：反斜杠真的是分隔符，`..\other` 就是上跳，必须拒。
    ///
    /// **本机（WSL/Linux）跑不到，未验证**；与 `pathsafe.rs` 里既有的几条
    /// `#[cfg(windows)]` 测试同一纪律。
    #[cfg(windows)]
    #[test]
    fn windows_treats_backslash_traversal_as_escape() {
        let s = WikiStore::new("C:\\base", user(1));
        assert!(
            s.resolve(DIR_WIKI, "..\\other\\secret.md").is_err(),
            "Windows 上 `..\\\\other` 是上跳，必须拒"
        );
        assert!(s.resolve(DIR_WIKI, "sub\\..\\..\\x.md").is_err());
    }

    /// 写路径也要拒（不只是 `resolve` 拒）：`write_page` / `write_raw` 会
    /// `create_dir_all(parent)`，放行一次就越权建目录。判据落在**真没落盘**上。
    #[test]
    fn write_apis_refuse_escapes_and_write_nothing() {
        let base = tmp_root("escape");
        let s = WikiStore::new(&base, user(3));
        s.ensure_layout().expect("建三层");

        let outside = base.join("outside-marker.md");
        for bad in [
            "../outside-marker.md",
            "sub/../../outside-marker.md",
            "/tmp/quill-wiki-abs.md",
        ] {
            assert!(
                s.write_page(bad, "越权").is_err(),
                "write_page 放行了 {bad:?}"
            );
            assert!(
                s.write_raw(bad, "越权").is_err(),
                "write_raw 放行了 {bad:?}"
            );
        }
        assert!(!outside.exists(), "越权写入竟然真的落了盘：{outside:?}");

        // 合法路径照常能写 —— 否则上面的「拒」可能只是因为整个 API 坏了。
        s.write_page("ok/page.md", "---\ntitle: OK\n---\n\n正文\n")
            .expect("合法路径必须能写");
        assert!(s.read_page("ok/page.md").is_ok());
    }

    #[test]
    fn two_users_get_disjoint_roots() {
        let a = WikiStore::new("/base", user(1));
        let b = WikiStore::new("/base", user(2));
        assert_ne!(a.root(), b.root());

        for rel in ["a.md", "x/y/z.md"] {
            let pa = normalize(&a.resolve(DIR_WIKI, rel).unwrap());
            let b_root = normalize(&b.root());
            assert!(!pa.starts_with(&b_root), "A 的路径落进了 B 的根：{pa:?}");
        }
    }

    #[test]
    fn write_read_round_trip_and_listing() {
        let base = tmp_root("rt");
        let s = WikiStore::new(&base, user(7));
        s.ensure_layout().expect("建三层");
        s.write_page("a.md", "---\ntitle: A\n---\n\n正文\n")
            .expect("写 a");
        s.write_page("sub/b.md", "---\ntitle: B\n---\n\n正文\n")
            .expect("写 b");

        s.write_raw("note.txt", "原始").expect("写 raw");
        assert!(base.join("wiki").exists());

        let p = s.read_page("a.md").expect("读 a");
        assert_eq!(p.frontmatter.title.as_deref(), Some("A"));

        let mut list = s.list_pages().expect("列表");
        list.sort();
        assert!(list.contains(&"a.md".to_string()));
        assert!(list.contains(&"sub/b.md".to_string()));
        assert!(!list.contains(&"note.txt".to_string()));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ensure_layout_is_idempotent() {
        let base = tmp_root("idem");
        let s = WikiStore::new(&base, user(3));
        s.ensure_layout().expect("第一次");
        s.ensure_layout().expect("第二次必须同样成功（幂等）");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn append_log_is_append_only() {
        let base = tmp_root("log");
        let s = WikiStore::new(&base, user(4));
        s.ensure_layout().expect("建三层");
        assert!(s.read_log().expect("读 log").is_none(), "初始应不存在");
        s.append_log("## [2026-10-04] ingest | A\n\n第一条\n")
            .expect("追加一");
        s.append_log("## [2026-10-04] ingest | B\n\n第二条\n")
            .expect("追加二");
        let text = s.read_log().expect("读 log").expect("有内容");
        assert!(text.contains("第一条"));
        assert!(text.contains("第二条"));

        let heads: Vec<&str> = text.lines().filter(|l| l.starts_with("## [")).collect();
        assert_eq!(heads.len(), 2, "标题行数不对：\n{text}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_schema_is_none_not_error() {
        let base = tmp_root("schema");
        let s = WikiStore::new(&base, user(5));
        s.ensure_layout().expect("建三层");

        assert!(s.read_schema("AGENTS.md").expect("读 schema").is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
