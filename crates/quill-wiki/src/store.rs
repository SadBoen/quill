//! 用户作用域的文件存储层（raw / wiki / schema 三层）。
//!
//! # 隔离为什么必须在这一层收口
//!
//! 契约 `docs/PHASE2_CONTRACT.md` §七（data-engineer 第 1 条）明确：
//! **wiki 要目录级隔离，不要用单库 + `user_id` 列** —— 因为文件操作
//! 无法加 `where`。
//! 因此本 crate 的**每一个**路径 API 都强制带 `&UserId`，
//! 且没有任何一个 API 能拿到「不带用户」的裸路径。
//!
//! 落地上是三道防线：
//!
//! | 防线 | 手段 | 挡住什么 |
//! |---|---|---|
//! | 1 | 根目录 = `<base>/wiki/<user 紧凑 hex>/` | 用户 A 的根与 B 的根物理分离 |
//! | 2 | [`WikiStore::resolve`] 拒绝绝对路径 / `..` / 前导 `/` | `../../other-user/...` 逃逸 |
//! | 3 | [`WikiStore::resolve`] 后再校验结果确实在根内（含 `..` 已规范化） | 符号链接式的逻辑逃逸 |
//!
//! ⚠️ 防线 3 的必要性：只做 `starts_with` 词法判断时，
//! `a/../../b` 仍以 `a/` 开头但已逃出根 —— 必须先规范化再比对。
//!
//! # 文件是真相源
//!
//! `docs/XU_WIKI_SPEC.md` §6.2：**文件是真相源，数据库只做索引缓存**。
//! 本层只做文件系统读写，**不引入任何数据库依赖**，
//! 也不缓存解析结果到进程内（那会变成第二份真相源）。

use std::io;
use std::path::{Component, Path, PathBuf};

use quill_adapters::UserId;

use crate::page::{parse_page, Page};

/// 三层目录名（规格 §二）。
pub const DIR_RAW: &str = "raw";
/// 知识层目录名。
pub const DIR_WIKI: &str = "wiki";
/// 规则层目录名。
pub const DIR_SCHEMA: &str = "schema";
/// 内容目录名（`wiki/index.md`）。
pub const INDEX_FILE: &str = "index.md";
/// 追加日志名（`wiki/log.md`）。
pub const LOG_FILE: &str = "log.md";

/// wiki 存储错误。
#[derive(Debug)]
pub enum WikiError {
    /// 路径逃逸出用户根目录。
    ///
    /// ⚠️ 这是**安全事件**，不是普通 IO 错误：
    /// 它意味着调用方（通常是 LLM 驱动的工具调用）给出了越权路径。
    PathEscape {
        attempt: String,
        reason: &'static str,
    },
    /// 路径形态非法（绝对路径、空串等）。
    InvalidPath {
        attempt: String,
        reason: &'static str,
    },
    /// IO 失败（含文件不存在）。
    Io { path: PathBuf, source: io::Error },
    /// 内容不是合法 UTF-8。
    NotUtf8 { path: PathBuf },
    /// 反向引用未能被解析成受支持的形态。
    MalformedIndex(String),
    /// 模型/provider 侧失败。
    ///
    /// ⚠️ 单独一类，**不并进 [`WikiError::Io`]**：
    /// 归错类会把排查方向引向磁盘，而真因在 provider 配置与凭据。
    Backend(String),
    /// 目标不存在（页面被删、索引里有悬挂条目）。
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
    /// 按契约 §三「在 adapters 边界转换为 `AdapterError`」。
    fn from(e: WikiError) -> Self {
        match e {
            // 越权路径 → Forbidden（不是 Unauthorized）：
            // 身份已认证，是**权限范围**问题。
            WikiError::PathEscape { .. } | WikiError::InvalidPath { .. } => {
                Self::Forbidden(e.to_string())
            }
            WikiError::NotUtf8 { .. } | WikiError::MalformedIndex(_) => {
                Self::Internal(e.to_string())
            }
            // 契约 §三：provider 侧错误 → Provider，不是 Storage。
            WikiError::Backend(_) => Self::Provider(e.to_string()),
            WikiError::NotFound(_) => Self::NotFound(e.to_string()),
            WikiError::Io { .. } => Self::Storage(e.to_string()),
        }
    }
}

/// 单个用户的 wiki 根。
///
/// **不可跨用户复用**：构造后即绑定一个 [`UserId`]，
/// 且不提供任何返回「不带用户上下文的绝对路径」的公开方法
/// （除 [`WikiStore::root`]，它已被文档标注为仅供诊断/测试）。
#[derive(Debug, Clone)]
pub struct WikiStore {
    base: PathBuf,
    user: UserId,
}

impl WikiStore {
    /// 构造：`<base>/wiki/<user 紧凑 hex>/`。
    ///
    /// 用紧凑 32 位 hex 而非短码：可读、可手工定位、与日志一致。
    pub fn new(base: impl AsRef<Path>, user: UserId) -> Self {
        Self {
            base: base.as_ref().to_path_buf(),
            user,
        }
    }

    /// 归属用户。
    pub fn user(&self) -> UserId {
        self.user
    }

    /// 用户 wiki 根的绝对/相对路径。
    ///
    /// ⚠️ **只用于诊断输出与测试断言**。业务代码应一律用
    /// [`WikiStore::resolve`] 或 [`WikiStore::layer`] 拿到的受控路径。
    pub fn root(&self) -> PathBuf {
        self.base.join("wiki").join(self.user.to_compact_hex())
    }

    /// 三层之一（`raw` / `wiki` / `schema`）。
    pub fn layer(&self, layer: &str) -> Result<PathBuf, WikiError> {
        match layer {
            DIR_RAW | DIR_WIKI | DIR_SCHEMA => Ok(self.root().join(layer)),
            other => Err(WikiError::InvalidPath {
                attempt: other.to_string(),
                reason: "层名只允许 raw / wiki / schema",
            }),
        }
    }

    /// 把用户给的相对路径解析成根内路径。
    ///
    /// 三道防线见模块文档。此函数是**唯一**能把「用户/LLM 给的字符串」
    /// 变成 `PathBuf` 的地方 —— 别处不得自行 `join`。
    pub fn resolve(&self, layer: &str, rel: &str) -> Result<PathBuf, WikiError> {
        let root = self.layer(layer)?;
        if rel.trim().is_empty() {
            return Err(WikiError::InvalidPath {
                attempt: rel.to_string(),
                reason: "路径为空",
            });
        }
        // 绝对路径（含 Windows 盘符与 UNC）直接拒 —— 不做「剥掉根再拼」。
        let p = Path::new(rel);
        if p.is_absolute() || rel.starts_with('/') || rel.starts_with('\\') {
            return Err(WikiError::PathEscape {
                attempt: rel.to_string(),
                reason: "不接受绝对路径",
            });
        }
        // 逐段检查：任何 `..` / 前导 `.` 的怪异组合 / Windows 前缀都拒。
        let mut depth: i32 = 0;
        for c in p.components() {
            match c {
                Component::Normal(seg) => {
                    let s = seg.to_string_lossy();
                    // Windows 盘符即使拼在相对路径里（如 `C:foo`）也不是普通段。
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
        // 防线 3：规范化后必须仍在根内（防 `a/../..` 这类词法骗法）。
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

    /// 创建三层目录（幂等 —— 铁律七：每步幂等）。
    pub fn ensure_layout(&self) -> Result<(), WikiError> {
        for l in [DIR_RAW, DIR_WIKI, DIR_SCHEMA] {
            std::fs::create_dir_all(self.layer(l)?).map_err(|source| WikiError::Io {
                path: self.layer(l).unwrap_or_else(|_| self.root()),
                source,
            })?;
        }
        Ok(())
    }

    /// 读一个 wiki 页面（相对 wiki 层）。
    pub fn read_page(&self, rel: &str) -> Result<Page, WikiError> {
        let p = self.resolve(DIR_WIKI, rel)?;
        let text = read_utf8(&p)?;
        Ok(parse_page(rel, &text))
    }

    /// 写一个 wiki 页面（相对 wiki 层），父目录自动创建。
    ///
    /// ⚠️ 这是**唯一的页面写入口**：`ingest` 编排只经它落盘，
    /// LLM 拿不到文件系统句柄（规格 §6.4 约束 1）。
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

    /// 写 raw 层文件（摄入的原始来源落盘处）。
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

    /// 读 raw 层文件。
    pub fn read_raw(&self, rel: &str) -> Result<String, WikiError> {
        let p = self.resolve(DIR_RAW, rel)?;
        read_utf8(&p)
    }

    /// 读 schema 层的规则文件全文（层 3）。
    ///
    /// 缺文件返回 `Ok(None)` 而非 `Err`：**schema 是可选的**
    /// （用户可能还没写），但「没写」与「读失败」必须可区分 ——
    /// 所以是 `Option`，不是把 IO 错误吞成空串。
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

    /// 列出 wiki 层全部页面（递归，按路径排序保证确定性）。
    ///
    /// ⚠️ **返回相对路径**（不含用户根）—— 避免把绝对路径泄给 LLM：
    /// 绝对路径一旦外泄，它就成了绕过 [`WikiStore::resolve`] 的旁路
    /// （LLM 可以直接拿它去拼别的路径）。
    pub fn list_pages(&self) -> Result<Vec<String>, WikiError> {
        let root = self.layer(DIR_WIKI)?;
        let mut out = Vec::new();
        collect_md(&root, &root, &mut out)?;
        out.sort();
        Ok(out)
    }

    /// 列出并解析全部页面。
    pub fn load_all_pages(&self) -> Result<Vec<Page>, WikiError> {
        let mut pages = Vec::new();
        for rel in self.list_pages()? {
            pages.push(self.read_page(&rel)?);
        }
        Ok(pages)
    }

    /// 读 `wiki/index.md`；不存在返回 `Ok(None)`。
    pub fn read_index(&self) -> Result<Option<String>, WikiError> {
        self.read_page_optional(INDEX_FILE)
    }

    /// 写 `wiki/index.md`。
    pub fn write_index(&self, content: &str) -> Result<(), WikiError> {
        self.write_page(INDEX_FILE, content)
    }

    /// 读 `wiki/log.md`；不存在返回 `Ok(None)`。
    pub fn read_log(&self) -> Result<Option<String>, WikiError> {
        self.read_page_optional(LOG_FILE)
    }

    /// 追加到 `wiki/log.md`（不存在则创建）。
    ///
    /// 日志是**追加不修改**（规格 §四）。
    pub fn append_log(&self, addition: &str) -> Result<(), WikiError> {
        let p = self.resolve(DIR_WIKI, LOG_FILE)?;
        let existing = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(source) => return Err(WikiError::Io { path: p, source }),
        };
        // 保证恰好一个空行分隔：`log.md` 是给 `grep "^## \["` 解析的，
        // 多余空行不影响解析，但**粘连**会让上一条目的正文吞掉下一个标题。
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

/// 词法规范化（消 `.` / `..`，不解引用符号链接）。
///
/// ⚠️ **不解引用符号链接是刻意的**：解引用需要在目标不存在时报错，
/// 而本函数用于「写入前的路径校验」，此时目标通常还不存在。
/// 因此防线 3 只保证**词法**上不出根 —— 词法逃逸（`..`、绝对路径、
/// 盘符）是 LLM 唯一能构造出来的越权手法，已被完全挡住。
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

/// 原子写：先写临时文件再 rename。
///
/// ⚠️ wiki 是**长期资产**（"知识只编译一次，然后保持持续更新"）。
/// 中途断电留下半截页面，比写失败严重得多 —— 半截 frontmatter
/// 会让下一页解析降级，而降级是静默的。
fn write_atomic(p: &Path, content: &str) -> Result<(), WikiError> {
    let tmp = p.with_extension("md.tmp");
    std::fs::write(&tmp, content).map_err(|source| WikiError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, p).map_err(|source| {
        // rename 失败时清掉临时文件，否则每次失败留一个垃圾文件。
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

    #[test]
    fn two_users_get_disjoint_roots() {
        let a = WikiStore::new("/base", user(1));
        let b = WikiStore::new("/base", user(2));
        assert_ne!(a.root(), b.root());
        // A 的任何相对路径解析结果都不得落在 B 的根下
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
        // 非 .md 不该进页面列表
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
        // 两行标题都必须能被 `grep "^## \["` 命中 —— 追加不得粘连
        let heads: Vec<&str> = text.lines().filter(|l| l.starts_with("## [")).collect();
        assert_eq!(heads.len(), 2, "标题行数不对：\n{text}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_schema_is_none_not_error() {
        let base = tmp_root("schema");
        let s = WikiStore::new(&base, user(5));
        s.ensure_layout().expect("建三层");
        // 「没写 schema」与「读失败」必须可区分
        assert!(s.read_schema("AGENTS.md").expect("读 schema").is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
