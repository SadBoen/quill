use crate::digest::{is_digest_hex, DIGEST_HEX_LEN};
use crate::error::BackupError;

pub const MANIFEST_NAME: &str = "MANIFEST";

pub const MANIFEST_VERSION: &str = "v1";

const HEADER: &str = "quill-backup-manifest v1";

const SEP: char = '\t';

const FILE_PREFIX: &str = "file\t";

const EXCLUDED_PREFIX: &str = "excluded\t";

const KNOWN_KEYS: &[&str] = &[
    "created.unix",
    "db.bytes",
    "db.sha256",
    "file.count",
    "excluded.count",
    "total.bytes",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub rel: String,

    pub bytes: u64,

    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcludedEntry {
    pub rel: String,

    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub created_unix: u64,

    pub db_bytes: u64,

    pub db_sha256: String,

    pub files: Vec<ManifestEntry>,

    pub excluded: Vec<ExcludedEntry>,

    pub total_bytes: u64,
}

impl Manifest {
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

            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| bad_line(line_no, line, "既不是条目行也不是 key=value 行"))?;
            if !KNOWN_KEYS.contains(&key) {
                return Err(BackupError::ManifestUnknownKey {
                    key: key.to_string(),
                });
            }

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

                other => unreachable!("KNOWN_KEYS 与 match 分支不一致：{other}"),
            }
        }

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

pub fn unsafe_reason(rel: &str) -> Option<&'static str> {
    if rel.is_empty() {
        return Some("路径为空");
    }
    if rel.starts_with('/') {
        return Some("以 / 开头，是绝对路径");
    }

    if rel.contains('\\') {
        return Some("含反斜杠（Windows 分隔符，会绕过上级目录检查）");
    }

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

    match lower.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && FORBIDDEN_EXT.contains(&ext),
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
        let mut a = sample();
        a.files.reverse();
        let b = sample();
        assert_eq!(a.render(), b.render());
    }

    #[test]
    fn secrets_enc_is_backed_up_but_master_key_is_not() {
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
        let mut lines: Vec<String> = sample().render().lines().map(str::to_string).collect();
        let pos = lines
            .iter()
            .position(|l| l.starts_with(&format!("file{SEP}")))
            .expect("清单里应有 file 行");
        lines.remove(pos);

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

    fn mutate(text: &str, from: &str, to: &str) -> String {
        assert!(
            text.contains(from),
            "测试输入构造失败：清单里找不到 {from:?} —— \
             这多半是清单格式改了，本用例需要跟着更新，**不是**格式有问题"
        );
        text.replacen(from, to, 1)
    }

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
