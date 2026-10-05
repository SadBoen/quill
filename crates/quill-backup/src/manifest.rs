//! 备份清单（MANIFEST）：备份目录的真相源。
//!
//! # 为什么是手写文本而不是 JSON
//!
//! 1. **不引 serde/serde_json**：`Cargo.lock` 零新增外部 crate 是本仓库的硬不变式
//!    （`docs/FACTS.md` 第三节）。手写解析器换来的是「一条依赖边都不加」。
//! 2. **人要能读**：备份是给唯一验收者（用户）看的。`quill doctor` 报「这个文件
//!    摘要不符」时，用户需要能用 `cat` / 记事本直接打开清单核对。
//! 3. **行式结构天然可 diff**：两次备份的差异一眼可见，便于排查「为什么这次
//!    备份比上次大」。
//!
//! # 格式（v1）
//!
//! ```text
//! quill-backup-manifest v1
//! created.unix=1757030000
//! db.bytes=20480
//! db.sha256=<64 hex>
//! file.count=7
//! file\t<sha256>\t<bytes>\t<相对路径>
//! excluded.count=1
//! excluded\t<相对路径>\t<中文原因>
//! total.bytes=40960
//! ```
//!
//! # 严格解析：三条不可让步的红线
//!
//! | 情形 | 处置 | 理由 |
//! |---|---|---|
//! | 未知键 | [`BackupError::ManifestUnknownKey`] 判红 | 宽容解析会让「清单由更新版本产生」变成静默少还原文件 |
//! | 重复键 | [`BackupError::ManifestDupKey`] 判红 | 重复键说明清单被拼接/改坏，后写的会静默覆盖先写的 |
//! | 路径逃逸（`..` / 绝对路径 / 反斜杠） | [`BackupError::UnsafePath`] 判红 | 清单是外部输入，等同用户输入，不可当可信 |
//!
//! ⚠️ 「清单是外部输入」是本模块最重要的一条设计前提：备份目录可能来自
//! NAS、被 U 盘拷过、被人手工编辑过。**不把它当可信输入 = 恢复时存在
//! 任意文件写漏洞**（可覆盖 `~/.ssh/authorized_keys`）。

use crate::digest::{is_digest_hex, DIGEST_HEX_LEN};
use crate::error::BackupError;

/// 清单文件名（固定；备份目录的入口）。
pub const MANIFEST_NAME: &str = "MANIFEST";

/// 清单格式版本。版本串出现在首行，版本不符即拒绝恢复。
pub const MANIFEST_VERSION: &str = "v1";

/// 首行固定内容。
const HEADER: &str = "quill-backup-manifest v1";

/// 字段分隔符（`\t`）。
///
/// ⚠️ 用 TAB 而不是空格：文件名里可以有空格，用空格分隔会让
/// 「`file\t<sha>  \t<bytes>`」这类含空格的路径产生歧义解析。
/// TAB 在 POSIX 文件名里合法但极罕见，且本模块在
/// [`validate_rel_path`] 里**显式拒绝控制字符**——含 TAB 的文件名
/// 无法进备份（会被记为 excluded 并给出中文原因），而不是产生歧义。
const SEP: char = '\t';

/// 条目前缀（**含分隔符**）—— `render` 与 `parse` 共用。
///
/// ⚠️ 含 SEP 是必须的：只写 `"file"` 会让 `file.count=2` 被误当成条目行。
/// 两个函数共用同一个常量 → 前缀口径不可能各写一遍而漂移。
const FILE_PREFIX: &str = "file\t";
/// 同上，`excluded` 条目前缀。
const EXCLUDED_PREFIX: &str = "excluded\t";

/// 清单里记录的**必备**键。
///
/// ⚠️ 这里的集合是**封闭**的：清单里出现表外的键就是
/// [`BackupError::ManifestUnknownKey`]。加新字段必须同时改这里。
const KNOWN_KEYS: &[&str] = &[
    "created.unix",
    "db.bytes",
    "db.sha256",
    "file.count",
    "excluded.count",
    "total.bytes",
];

/// 一个已备份文件的记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// 备份目录内的相对路径（`/` 分隔）。
    pub rel: String,
    /// 字节数。
    pub bytes: u64,
    /// 内容 SHA-256（小写 hex，64 字符）。
    pub sha256: String,
}

/// 一个**未**备份的条目及原因。
///
/// ⚠️ 排除项必须落进清单而不是静默丢弃：
/// 用户问「我的 `master.key` 怎么不在备份里」时，答案必须是可查的，
/// 而不是我记得有个排除规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludedEntry {
    /// 相对 `data/` 的路径（`/` 分隔）。
    pub rel: String,
    /// 中文原因（用户直接看这段话，不需要再查文档）。
    pub reason: String,
}

/// 一份完整清单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// 备份创建时刻（Unix 秒，来自 `SystemTime`，无外部时间库）。
    pub created_unix: u64,
    /// 快照数据库的字节数。
    pub db_bytes: u64,
    /// 快照数据库的 SHA-256。
    pub db_sha256: String,
    /// 已备份的用户数据文件。
    pub files: Vec<ManifestEntry>,
    /// 被排除的文件及原因。
    pub excluded: Vec<ExcludedEntry>,
    /// 全部字节合计（数据库 + 用户数据）。
    pub total_bytes: u64,
}

impl Manifest {
    /// 渲染为清单文本。
    ///
    /// ⚠️ 输出是**确定性**的（条目按 `rel` 排序），这样两次同内容备份的
    /// 清单可以直接 `diff`，且 `MANIFEST` 自身不引入无谓的时间戳差异。
    pub fn render(&self) -> String {
        let mut files = self.files.clone();
        files.sort_by(|a, b| a.rel.cmp(&b.rel));
        let mut excluded = self.excluded.clone();
        excluded.sort_by(|a, b| a.rel.cmp(&b.rel));

        let mut out = String::with_capacity(256 + files.len() * 96);
        out.push_str(HEADER);
        out.push('\n');
        out.push_str(&format!("created.unix={}\n", self.created_unix));
        out.push_str(&format!("db.bytes={}\n", self.db_bytes));
        out.push_str(&format!("db.sha256={}\n", self.db_sha256));
        out.push_str(&format!("file.count={}\n", files.len()));
        for f in &files {
            out.push_str(FILE_PREFIX);
            out.push_str(&format!("{}{SEP}{}{SEP}{}\n", f.sha256, f.bytes, f.rel));
        }
        out.push_str(&format!("excluded.count={}\n", excluded.len()));
        for e in &excluded {
            out.push_str(EXCLUDED_PREFIX);
            out.push_str(&format!("{}{SEP}{}\n", e.rel, e.reason));
        }
        out.push_str(&format!("total.bytes={}\n", self.total_bytes));
        out
    }

    /// 从清单文本解析。
    ///
    /// # Errors
    /// 任一行不合规即返回 [`BackupError`]；**不会**「尽力解析出部分内容」。
    /// 部分清单比没有清单更危险：它会让恢复看起来成功。
    pub fn parse(text: &str) -> Result<Self, BackupError> {
        let mut lines = text.lines().enumerate();

        let Some((_, first)) = lines.next() else {
            return Err(BackupError::ManifestBadLine {
                line_no: 1,
                content: String::new(),
                reason: "清单是空文件".into(),
            });
        };
        if first.trim() != HEADER {
            return Err(BackupError::ManifestBadVersion {
                found: first.trim().to_string(),
            });
        }

        let mut created_unix: Option<u64> = None;
        let mut db_bytes: Option<u64> = None;
        let mut db_sha256: Option<String> = None;
        // ⚠️ 一律先收成 Option<u64>：键值行的值都是「非负整数」，
        //    统一类型才能共用一个 `need` 缺字段检查，也免掉 `as usize` 转换
        //    （转换在超大清单上会静默截断）。
        let mut declared_file_count: Option<u64> = None;
        let mut declared_excluded_count: Option<u64> = None;
        let mut total_bytes: Option<u64> = None;
        let mut files: Vec<ManifestEntry> = Vec::new();
        let mut excluded: Vec<ExcludedEntry> = Vec::new();

        for (idx, raw) in lines {
            let line_no = idx + 1;
            let line = raw.trim_end_matches(['\r']);
            if line.trim().is_empty() {
                continue;
            }

            // 条目行（多字段 + TAB 分隔）
            //
            // ⚠️ 前缀必须是 `"file" + SEP` / `"excluded" + SEP`（**含分隔符**），
            //    不能只写 `strip_prefix("file")`：那样 `file.count=2` 这行
            //    也会被当成条目行 → 自己写的清单读不回来。
            //    （这个 bug 是被 `render_then_parse_round_trips` 抓到的 ——
            //     没有往返用例，实现侧与清单格式就会各写各的。）
            if let Some(rest) = line.strip_prefix(FILE_PREFIX) {
                let parts: Vec<&str> = rest.split(SEP).collect();
                if parts.len() != 3 {
                    return Err(bad_line(
                        line_no,
                        line,
                        &format!(
                            "file 行应有 3 个字段（摘要/字节数/路径），实际 {}",
                            parts.len()
                        ),
                    ));
                }
                let sha = parts[0];
                check_digest(line_no, line, sha)?;
                let bytes = parse_u64(line_no, line, parts[1])?;
                let rel = parts[2];
                validate_rel_path(line_no, line, rel)?;
                files.push(ManifestEntry {
                    rel: rel.to_string(),
                    bytes,
                    sha256: sha.to_string(),
                });
                continue;
            }
            if let Some(rest) = line.strip_prefix(EXCLUDED_PREFIX) {
                let parts: Vec<&str> = rest.split(SEP).collect();
                if parts.len() != 2 {
                    return Err(bad_line(
                        line_no,
                        line,
                        &format!(
                            "excluded 行应有 2 个字段（路径/原因），实际 {}",
                            parts.len()
                        ),
                    ));
                }
                validate_rel_path(line_no, line, parts[0])?;
                excluded.push(ExcludedEntry {
                    rel: parts[0].to_string(),
                    reason: parts[1].to_string(),
                });
                continue;
            }

            // 键值行
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| bad_line(line_no, line, "既不是条目行也不是 key=value 行"))?;
            if !KNOWN_KEYS.contains(&key) {
                return Err(BackupError::ManifestUnknownKey {
                    key: key.to_string(),
                });
            }
            // ⚠️ `db.sha256` 存 String、其余存 u64。这里**不**借用 slot
            //    （`&mut Option<u64>` 与 `&mut Option<String>` 无法统一成同一类型），
            //    而是逐键显式赋值 + 各自的「重复即判红」检查。
            //    重复检查放在各分支里：集中在一处反而需要一个统一的 slot 类型，
            //    而那个统一类型只能靠枚举/装箱引入，反而更绕。
            let dup = |k: &str| BackupError::ManifestDupKey { key: k.to_string() };
            match key {
                "created.unix" => {
                    if created_unix.is_some() {
                        return Err(dup(key));
                    }
                    created_unix = Some(parse_u64(line_no, line, value)?);
                }
                "db.bytes" => {
                    if db_bytes.is_some() {
                        return Err(dup(key));
                    }
                    db_bytes = Some(parse_u64(line_no, line, value)?);
                }
                "db.sha256" => {
                    if db_sha256.is_some() {
                        return Err(dup(key));
                    }
                    check_digest(line_no, line, value)?;
                    db_sha256 = Some(value.to_string());
                }
                "file.count" => {
                    if declared_file_count.is_some() {
                        return Err(dup(key));
                    }
                    declared_file_count = Some(parse_u64(line_no, line, value)?);
                }
                "excluded.count" => {
                    if declared_excluded_count.is_some() {
                        return Err(dup(key));
                    }
                    declared_excluded_count = Some(parse_u64(line_no, line, value)?);
                }
                "total.bytes" => {
                    if total_bytes.is_some() {
                        return Err(dup(key));
                    }
                    total_bytes = Some(parse_u64(line_no, line, value)?);
                }
                // 上面的 KNOWN_KEYS.contains 已挡住未知键，这里是穷尽匹配。
                other => unreachable!("KNOWN_KEYS 与 match 分支不一致：{other}"),
            }
        }

        // 缺键即判红：少一个字段就意味着「我知道备份里有什么」这件事不成立。
        let need = |name: &str, v: Option<u64>| -> Result<u64, BackupError> {
            v.ok_or_else(|| bad_line(0, name, "清单缺少必填字段"))
        };
        let need_str = |name: &str, v: Option<String>| -> Result<String, BackupError> {
            v.ok_or_else(|| bad_line(0, name, "清单缺少必填字段"))
        };
        let created_unix = need("created.unix", created_unix)?;
        let db_bytes = need("db.bytes", db_bytes)?;
        let db_sha256 = need_str("db.sha256", db_sha256)?;
        let declared_file_count = need("file.count", declared_file_count)? as usize;
        let declared_excluded_count = need("excluded.count", declared_excluded_count)? as usize;
        let total_bytes = need("total.bytes", total_bytes)?;

        // 头里声明的条数必须等于实际列出的条数。
        // 只对 file 计数做这一条：清单被截断时，尾部条目连同 total.bytes 一起消失，
        // 此时 file.count 仍对不上 —— 这是「截断」唯一可自证的信号。
        if declared_file_count != files.len() {
            return Err(BackupError::FileCountMismatch {
                declared: declared_file_count,
                actual: files.len(),
            });
        }
        if declared_excluded_count != excluded.len() {
            return Err(BackupError::FileCountMismatch {
                declared: declared_excluded_count,
                actual: excluded.len(),
            });
        }

        // 重复路径会让「恢复两次」时后者覆盖前者，且清单只记一条 → 静默少还原。
        let mut seen = std::collections::BTreeSet::new();
        for f in &files {
            if !seen.insert(f.rel.clone()) {
                return Err(BackupError::ManifestBadLine {
                    line_no: 0,
                    content: f.rel.clone(),
                    reason: "同一个相对路径在清单里出现了两次".into(),
                });
            }
        }

        Ok(Self {
            created_unix,
            db_bytes,
            db_sha256,
            files,
            excluded,
            total_bytes,
        })
    }
}

fn bad_line(line_no: usize, content: &str, reason: &str) -> BackupError {
    // 行内容截断到 120 字符：错误消息本身不该变成一兆的转储。
    let short: String = content.chars().take(120).collect();
    BackupError::ManifestBadLine {
        line_no,
        content: short,
        reason: reason.to_string(),
    }
}

fn check_digest(line_no: usize, line: &str, v: &str) -> Result<(), BackupError> {
    if is_digest_hex(v) {
        return Ok(());
    }
    Err(bad_line(
        line_no,
        line,
        &format!("摘要必须是 {DIGEST_HEX_LEN} 位小写 hex，实际 {:?}", v),
    ))
}

fn parse_u64(line_no: usize, line: &str, v: &str) -> Result<u64, BackupError> {
    v.parse::<u64>()
        .map_err(|_| bad_line(line_no, line, &format!("应是非负整数，实际 {v:?}")))
}

/// 校验清单里的相对路径安全。
///
/// # 为什么这是安全边界而不是洁癖
///
/// 恢复要按清单里的路径往目标目录写文件。若清单是
/// `file\t<sha>\t<10>\t../../.ssh/authorized_keys`，
/// 一个「校验摘要通过」的恢复就能往 SSH 目录写东西 ——
/// 摘要只证明**内容**没变，不证明**写到哪里**是安全的。
/// 所以路径必须独立校验，且拒绝规则要覆盖：
/// 绝对路径、`..` 组件、空组件、反斜杠（Windows 分隔符）、
/// 控制字符（含 TAB/换行）、`.`/`..` 本身、以及盘符前缀。
fn validate_rel_path(line_no: usize, line: &str, rel: &str) -> Result<(), BackupError> {
    if let Some(why) = unsafe_reason(rel) {
        return Err(BackupError::UnsafePath {
            raw: rel.to_string(),
            why,
        });
    }
    let _ = (line_no, line);
    Ok(())
}

/// 返回路径不安全的原因；安全则 `None`。
///
/// 拆成独立函数（不绑行号/行内容）是为了让**生成侧**（备份时决定要不要
/// 收录某个文件）能复用同一套判据 —— 写不进清单的路径，恢复时也必然被拒，
/// 两侧口径不会各写一遍而漂移。
pub fn unsafe_reason(rel: &str) -> Option<&'static str> {
    if rel.is_empty() {
        return Some("路径为空");
    }
    if rel.starts_with('/') {
        return Some("以 / 开头，是绝对路径");
    }
    // 反斜杠：Windows 上会被当分隔符，绕过 `..` 检查（`..\\x` 在 Unix 上是合法文件名，
    // 在 Windows 上却能逃逸）。两种语义混在一起 = 逃逸。
    if rel.contains('\\') {
        return Some("含反斜杠（Windows 分隔符，会绕过上级目录检查）");
    }
    // 盘符前缀（如 `C:`）在 Windows 上被 `Path` 当根目录。
    let b = rel.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return Some("以盘符开头（如 C:），是绝对路径");
    }
    if rel.chars().any(|c| c.is_control()) {
        return Some("含控制字符（换行/制表符等，无法安全解析）");
    }
    if rel.ends_with('/') {
        return Some("以 / 结尾，指向目录而非文件");
    }
    for comp in rel.split('/') {
        if comp.is_empty() {
            return Some("含空路径段（双斜杠或以 / 开头）");
        }
        if comp == ".." {
            return Some("含上级目录 ..，会写到目标目录之外");
        }
        if comp == "." {
            return Some("含当前目录 .，路径不规范");
        }
    }
    None
}

/// 某个文件名是否命中**绝不进备份**的规则。
///
/// # 规则来源
///
/// 铁律三 + `scripts/quill-doctor-roaming.sh` R4：备份里出现
/// 密钥材料会让「密文与钥匙同处」，把一次备份泄漏变成凭据泄漏。
/// 本函数是这条规则在**实现侧**的落点（脚本只在部署侧查产物目录）。
///
/// ⚠️ 匹配口径是「文件名或扩展名」，大小写不敏感 ——
/// `Master.Key` 与 `master.key` 对攻击者是同一回事。
/// 只匹配精确名会让人改个大小写就把密钥文件送进备份。
pub fn is_forbidden_in_backup(file_name: &str) -> bool {
    const FORBIDDEN_EXACT: &[&str] = &["master.key", "masterkey", "id_rsa", "id_dsa", "id_ecdsa"];
    const FORBIDDEN_EXT: &[&str] = &["key", "pem", "p12", "pfx", "jks", "keystore"];
    const FORBIDDEN_SUFFIX: &[&str] = &[
        "master.key",
        "master_key",
        "private.key",
        "private_key",
        ".npmrc",
        ".netrc",
        ".env",
    ];

    let lower = file_name.to_ascii_lowercase();
    if FORBIDDEN_EXACT.contains(&lower.as_str()) {
        return true;
    }
    if FORBIDDEN_SUFFIX.iter().any(|s| lower.ends_with(s)) {
        return true;
    }
    // 扩展名：`.key` / `.pem` 等。`secrets.enc` **不在此列** ——
    // 它是密文（无钥匙时不可解），且备份它正是「完整恢复」的前提。
    match lower.rsplit_once('.') {
        Some((stem, ext)) => {
            // `x.key` 的 stem 为 `x`；`x` 无点时 rsplit 不返回，落到 false。
            !stem.is_empty() && FORBIDDEN_EXT.contains(&ext)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            created_unix: 1_757_030_000,
            db_bytes: 20480,
            db_sha256: "a".repeat(64),
            // ⚠️ 条目**按 rel 升序**给：`render` 会排序，往返测试比的是
            //    「render 后的结构 == 原结构」。样本自己就有序，
            //    才让「往返后不等」真正意味着解析器坏了，而不是排序造成的。
            files: vec![
                ManifestEntry {
                    rel: "u1/secrets.enc".into(),
                    bytes: 64,
                    sha256: "c".repeat(64),
                },
                ManifestEntry {
                    rel: "u1/wiki/index.md".into(),
                    bytes: 12,
                    sha256: "b".repeat(64),
                },
            ],
            excluded: vec![ExcludedEntry {
                rel: "u1/master.key".into(),
                reason: "密钥材料不进备份".into(),
            }],
            total_bytes: 40960,
        }
    }

    #[test]
    fn render_then_parse_round_trips() {
        let m = sample();
        let text = m.render();
        let back = Manifest::parse(&text).expect("自己写的清单必须能读回来");
        assert_eq!(m, back);
    }

    #[test]
    fn render_is_deterministic_regardless_of_input_order() {
        // 两次同内容备份的清单要能直接 diff —— 故排序必须发生在 render 里。
        let mut a = sample();
        a.files.reverse();
        let b = sample();
        assert_eq!(a.render(), b.render());
    }

    #[test]
    fn secrets_enc_is_backed_up_but_master_key_is_not() {
        // 需求 6「完整恢复」要求 secrets.enc 进备份；
        // R4 要求密钥材料不进备份。两者必须同时成立。
        assert!(
            !is_forbidden_in_backup("secrets.enc"),
            "secrets.enc 是密文，必须能进备份"
        );
        assert!(is_forbidden_in_backup("master.key"));
        assert!(is_forbidden_in_backup("Master.Key"), "大小写变体同样要拦");
        assert!(is_forbidden_in_backup("id_rsa"));
        assert!(is_forbidden_in_backup("tls.key"), "扩展名 .key 要拦");
        assert!(is_forbidden_in_backup("tls.pem"), "扩展名 .pem 要拦");
        assert!(is_forbidden_in_backup(".env"));
        // 边界：普通文件不能被误伤（误报闸门与假闸门同等有害）
        for ok in [
            "index.md",
            "wiki.md",
            "session.json",
            "monkey.md",
            "keyboard.txt",
            "secrets.enc.bak",
        ] {
            assert!(!is_forbidden_in_backup(ok), "误伤合法文件：{ok}");
        }
    }

    #[test]
    fn rejects_unknown_key() {
        let text = sample().render() + "db.md5=deadbeef\n";
        match Manifest::parse(&text) {
            Err(BackupError::ManifestUnknownKey { key }) => assert_eq!(key, "db.md5"),
            other => panic!("应判未知键，实际 {other:?}"),
        }
    }

    #[test]
    fn keyval_keys_are_not_mistaken_for_entry_lines() {
        // 回归测试：`file.count=2` 以 "file" 开头。
        // 早期实现用 strip_prefix("file")，于是这一行被当成条目行 →
        // 「自己写的清单读不回来」。往返用例抓到了它，这里再钉一条更直白的。
        let text = sample().render();
        for k in [
            "file.count",
            "excluded.count",
            "db.bytes",
            "total.bytes",
            "created.unix",
        ] {
            assert!(text.contains(&format!("{k}=")), "渲染结果里应含键 {k}");
        }
        // 能读回来就证明这些行没被误判成条目行。
        assert!(Manifest::parse(&text).is_ok());
        assert!(FILE_PREFIX.ends_with(SEP), "条目前缀必须含分隔符");
        assert!(EXCLUDED_PREFIX.ends_with(SEP), "条目前缀必须含分隔符");
    }

    #[test]
    fn rejects_duplicate_key() {
        let text = sample().render() + "db.bytes=1\n";
        match Manifest::parse(&text) {
            Err(BackupError::ManifestDupKey { key }) => assert_eq!(key, "db.bytes"),
            other => panic!("应判重复键，实际 {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_required_key() {
        // 去掉 db.sha256 —— 「我知道备份里有什么」这件事就不成立了。
        let text: String = sample()
            .render()
            .lines()
            .filter(|l| !l.starts_with("db.sha256="))
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(matches!(
            Manifest::parse(&text),
            Err(BackupError::ManifestBadLine { .. })
        ));
    }

    #[test]
    fn rejects_truncated_manifest_via_count_mismatch() {
        // 模拟「备份写到一半断电」：删掉最后一条 file 与 total.bytes。
        let mut lines: Vec<String> = sample().render().lines().map(str::to_string).collect();
        let pos = lines
            .iter()
            .position(|l| l.starts_with(&format!("file{SEP}")))
            .expect("清单里应有 file 行");
        lines.remove(pos);
        // 补回 total.bytes，模拟「尾部只丢了一条」——最难的截断形态。
        let text: String = lines
            .into_iter()
            .filter(|l| !l.starts_with("total.bytes=") && !l.starts_with("excluded.count="))
            .chain(std::iter::once("excluded.count=1".to_string()))
            .chain(std::iter::once("total.bytes=1".to_string()))
            .map(|l| format!("{l}\n"))
            .collect();
        match Manifest::parse(&text) {
            Err(BackupError::FileCountMismatch { declared, actual }) => {
                assert_eq!(declared, 2);
                assert_eq!(actual, 1);
            }
            other => panic!("应判条数不符，实际 {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_rel_path() {
        // 直接手写一份「同一路径出现两次」的清单。
        // ⚠️ 不从 render() 出发做字符串替换 —— 那是**用被测对象生成输入**，
        //    前缀一改替换就悄悄失效（本次就是这么假绿的：断言一直成立，
        //    但它匹配的根本不是重复路径）。
        let dup = "u1/wiki/index.md";
        let sha = "b".repeat(64);
        let text = format!(
            "{HEADER}\ncreated.unix=1\ndb.bytes=1\ndb.sha256={}\nfile.count=2\n\
             file{SEP}{sha}{SEP}12{SEP}{dup}\nfile{SEP}{sha}{SEP}12{SEP}{dup}\n\
             excluded.count=0\ntotal.bytes=24\n",
            "a".repeat(64)
        );
        match Manifest::parse(&text) {
            Err(BackupError::ManifestBadLine { reason, .. }) => {
                assert!(reason.contains("两次"), "原因应说明重复：{reason}");
            }
            other => panic!("应判重复路径，实际 {other:?}"),
        }
    }

    /// 造测试输入：**先确认 needle 真的存在**，再替换。
    ///
    /// ⚠️ 为什么不用裸 `replacen`：清单格式一改（比如条目前缀从 `file`
    ///    变成 `file\t`），裸 `replacen` 找不到就**原样返回**，
    ///    而「解析这份未改动的清单」往往会**通过**断言 ——
    ///    用例变成恒绿，却还在报告「已覆盖」。
    ///    （`rejects_duplicate_rel_path` 就真实假绿过一次。）
    fn mutate(text: &str, from: &str, to: &str) -> String {
        assert!(
            text.contains(from),
            "测试输入构造失败：清单里找不到 {from:?} —— \
             这多半是清单格式改了，本用例需要跟着更新，**不是**格式有问题"
        );
        text.replacen(from, to, 1)
    }

    /// 一份格式良好的清单文本（`sample()` 渲染结果）。
    fn sample_text() -> String {
        sample().render()
    }

    #[test]
    fn rejects_bad_version_header() {
        let text = mutate(&sample_text(), HEADER, "quill-backup-manifest v2");
        match Manifest::parse(&text) {
            Err(BackupError::ManifestBadVersion { found }) => {
                assert_eq!(found, "quill-backup-manifest v2")
            }
            other => panic!("应判版本不符，实际 {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_and_garbage() {
        assert!(matches!(
            Manifest::parse(""),
            Err(BackupError::ManifestBadLine { .. })
        ));
        assert!(matches!(
            Manifest::parse("随便一行\n"),
            Err(BackupError::ManifestBadVersion { .. })
        ));
    }

    #[test]
    fn rejects_bad_digest_shape() {
        let text = mutate(&sample_text(), &"a".repeat(64), "abc");
        assert!(matches!(
            Manifest::parse(&text),
            Err(BackupError::ManifestBadLine { .. })
        ));
    }

    #[test]
    fn rejects_non_integer_counts() {
        let text = mutate(&sample_text(), "file.count=2", "file.count=abc");
        assert!(matches!(
            Manifest::parse(&text),
            Err(BackupError::ManifestBadLine { .. })
        ));
    }

    #[test]
    fn path_traversal_variants_are_all_rejected() {
        // 每一行都是一次「清单被改坏 → 任意文件写」的尝试。
        // 只测 `../` 是不够的：绝对路径、反斜杠、盘符同样能逃逸。
        for bad in [
            "../etc/passwd",
            "u1/../../etc/passwd",
            "/etc/passwd",
            "u1/../../../../root/.ssh/authorized_keys",
            "u1\\..\\..\\windows\\system32",
            "C:/windows/system32",
            "u1//wiki.md",
            "u1/./wiki.md",
            "u1/",
            "",
            "u1/wi\nki.md",
            "u1/wi\tki.md",
        ] {
            assert!(unsafe_reason(bad).is_some(), "路径逃逸未被拦：{bad:?}");
        }
        for good in [
            "u1/wiki/index.md",
            "u1/secrets.enc",
            "a.md",
            "u1/a b/c d.md",
        ] {
            assert_eq!(unsafe_reason(good), None, "合法路径被误伤：{good:?}");
        }
    }

    #[test]
    fn manifest_traversal_entry_is_rejected_at_parse_time() {
        // 不只测 unsafe_reason —— 必须证明**解析器真的调用了它**。
        // 否则「校验函数存在但没接线」就是一道永不生效的闸门。
        let text = mutate(
            &sample_text(),
            "u1/wiki/index.md",
            "../../.ssh/authorized_keys",
        );
        match Manifest::parse(&text) {
            Err(BackupError::UnsafePath { raw, .. }) => {
                assert!(raw.contains(".."));
            }
            other => panic!("清单里的逃逸路径应被判不安全，实际 {other:?}"),
        }
    }
}
