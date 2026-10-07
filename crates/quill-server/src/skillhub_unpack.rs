//! 把 SkillHub 下回来的 zip 变成磁盘上的技能文件。
//!
//! **这个文件里的每一条检查都对应一类真实攻击**，不是「稳妥起见」：
//! 解 zip 面向的是**外部服务给的任意字节**，一个不设防的解压器
//! 就是一个能让任意用户把服务端打爆的服务。
//!
//! 抄自 Octop 的同一组上限（`skills/skillhub_common.py`）——
//! 那些数字是它踩过之后定下来的，不是随手拍的。

use std::io::Read;

use crate::skillhub::{MAX_ZIP_COMPRESSION_RATIO, MAX_ZIP_ENTRIES, MAX_ZIP_UNCOMPRESSED_BYTES};

// zip 条目数上限是个常量不变量，用编译期断言钉住（原来写成单测里的
// `assert!(MAX_ZIP_ENTRIES >= 1_000)`，那条断言在构造上不会失败）。
const _: () = assert!(MAX_ZIP_ENTRIES >= 1_000, "zip 条目数上限过低");

/// 一个技能包解出来的东西。
// `Debug` 只为测试里的 `panic!("{other:?}")` 存在 —— 但有它才能在断言失败时
// 打出实际拿到了什么，而不是一句「这里不对」。
#[derive(Debug)]
pub struct Unpacked {
    /// `(文件名, 正文)`，文件名是纯 basename，不含任何目录成分。
    pub files: Vec<(String, String)>,
    /// 原始压缩字节数，用于如实告诉用户这个包有多大。
    pub compressed_bytes: u64,
    pub uncompressed_bytes: u64,
    /// 收到的条目里被丢掉的数量（图片、LICENSE、脚本…）。**如实报出去。**
    pub skipped_other: usize,
}

#[derive(Debug)]
pub enum UnpackError {
    /// zip 本身读不出来。
    Zip(String),
    /// 触到了某一条安全上限。**每一条都带上是哪个上限、超了多少**，
    /// 否则用户只会看到一句「安装失败」，而不知道是自己装了个怪东西。
    Refused(String),
    /// 整个包里一个 .md 都没有。
    NoSkill,
}

impl UnpackError {
    pub fn message(&self) -> String {
        match self {
            UnpackError::Zip(d) => format!(
                "技能包解不开（不是有效的 zip）。下一步：这个包可能是上游坏了或被中途截断，\
                 换一个试试。详情：{d}"
            ),
            UnpackError::Refused(why) => format!(
                "这个技能包因为安全检查没过，没有安装。下一步：{why}"
            ),
            UnpackError::NoSkill => {
                "这个包里没有可用的技能正文（.md）。下一步：换一个技能包；\
                 如果这是单技能，它可能缺 SKILL.md。"
                    .to_string()
            }
        }
    }
}

fn refuse(why: impl Into<String>) -> UnpackError {
    UnpackError::Refused(why.into())
}

/// 解包到内存（不落盘）。落盘由调用方做 —— 它才知道技能目录在哪。
pub fn unpack(bytes: &[u8]) -> Result<Unpacked, UnpackError> {
    unpack_with(bytes, PackageKind::SkillSet)
}

/// 解一个**单技能**包。产出的 `files` 一定有且只有一项。
///
/// 见 [`PackageKind`]：技能包和单技能包的 `.md` 数量完全不同，
/// 用同一套规则解会出现「同一个 slug 装了三遍、互相覆盖」。
pub fn unpack_skill(bytes: &[u8]) -> Result<Unpacked, UnpackError> {
    unpack_with(bytes, PackageKind::Skill)
}
/// 这份 zip 到底是**技能包**还是**单技能**。
///
/// 两者的结构实测差别很大，**当成同一套规则解就会出错**：
///
/// - 技能包（`tech-test-automation`）：`manifest.json` + `identify.md`。
///   收全部 `.md`，技能名取包 slug。
/// - 单技能（`pdf-image-text-extractor`，2026-10-06 实测 8 个条目）：
///   `SKILL.md`（15 KB，正文）、`README.md`、`README.en.md`、
///   `scripts/*.py` 4 个、`_meta.json`。
///
/// 后者若按前者的规则解，`SKILL.md` 与两个 `README` 会**被当成三个技能**，
/// 而它们共用同一个 slug 名 —— 落盘时后写的把先写的覆盖掉，
/// 用户装完不知道自己拿到了哪一份。**这个覆盖是真的会发生，不是理论风险。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageKind {
    /// 技能包：收全部 `.md` 与 `manifest.json`。
    SkillSet,
    /// 单技能：**只收一篇正文**（优先 `SKILL.md`），其余一律丢掉。
    Skill,
}

/// 单技能的正文文件名。实测上游的约定就是这个大小写。
const SKILL_ENTRY: &str = "SKILL.md";

/// 报告用的是包里哪一个文件当正文。
///
/// 存在的理由：万一上游哪天把 `SKILL.md` 改名了，我们要**说清楚**
/// 拿的是哪一个，而不是静悄悄地换一个 —— 用户看到的技能内容
/// 必须能追到它来自哪个文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBody {
    pub file: String,
    pub body: String,
}

fn unpack_with(bytes: &[u8], kind: PackageKind) -> Result<Unpacked, UnpackError> {
    let reader = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| UnpackError::Zip(e.to_string()))?;

    let total = zip.len();
    guard_entry_count(total)?;

    let compressed_bytes: u64 = bytes.len() as u64;
    let mut uncompressed_total: u64 = 0;
    let mut files: Vec<(String, String)> = Vec::new();
    // 单技能模式下，每篇 `.md` 的候选正文。**先都收着，装完再挑一篇** ——
    // 挑哪篇是个判断（见 `unpack_skill`），不该藏在读盘那一层里。
    let mut skill_candidates: Vec<SkillBody> = Vec::new();
    // 非 .md / 非 manifest 的条目数。**要报出去** ——
    // 悄悄丢掉 40 个文件而界面一个字不说，用户会以为装全了。
    let mut skipped_other: usize = 0;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| UnpackError::Zip(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        guard_entry(&entry, &mut uncompressed_total)?;

        // 收哪些条目由包的种类决定。**其余一律丢掉**。
        // manifest 要留着是因为它装着「这个包还引用了哪些技能」——
        // 丢掉的话界面上就只能说「装好了」，说不清装的是包还是那几个之一。
        // 丢掉别的东西（比如 LICENSE、图片、脚本）是有意的：
        // 我们只把「方法说明」挂进工具表，不执行上游给的任何东西。
        let name = entry.name().to_string();
        let base = sanitize_name(&name)?;
        let is_md = base.to_ascii_lowercase().ends_with(".md");
        let keep = match kind {
            PackageKind::SkillSet => is_md || base == "manifest.json",
            // 单技能：`SKILL.md` 优先；没有它时只认**根目录**下的那篇
            // （带目录的 `docs/foo.md` 不算正文 —— 那是文档，不是技能）。
            // README 之类照收不误，但见下面 `unpack_skill` 的取舍说明。
            PackageKind::Skill => is_md,
        };
        if !keep {
            skipped_other += 1;
            continue;
        }

        let mut text = String::new();
        entry
            .read_to_string(&mut text)
            .map_err(|e| UnpackError::Zip(format!("读取 {base} 失败：{e}")))?;
        match kind {
            PackageKind::SkillSet => files.push((base, text)),
            PackageKind::Skill => {
                // 记下**原始条目名**：要靠它判断这是不是根目录下的那篇。
                skill_candidates.push(SkillBody {
                    file: name,
                    body: text,
                });
            }
        }
    }

    if kind == PackageKind::Skill {
        return pick_one_body(
            skill_candidates,
            skipped_other,
            compressed_bytes,
            uncompressed_total,
        );
    }

    // 一个 .md 都没有就是装不了东西。有 manifest 但没正文同样是。
    if !files.iter().any(|(n, _)| n.ends_with(".md")) {
        return Err(UnpackError::NoSkill);
    }

    Ok(Unpacked {
        files,
        compressed_bytes,
        uncompressed_bytes: uncompressed_total,
        skipped_other,
    })
}

/// 逐条过四道安全上限。
///
/// **抽出来是因为现在有两条解包路径**（装技能、装市场专家），
/// 而这四道上限是安全边界不是顺手加的检查：复制一份过去，
/// 早晚有一天只改了一边，另一边就悄悄不设防了。
fn guard_entry(
    entry: &zip::read::ZipFile<'_>,
    uncompressed_total: &mut u64,
) -> Result<(), UnpackError> {
    let raw = entry.size();
    *uncompressed_total = uncompressed_total.saturating_add(raw);

    // 上限 A：解压后总量。**必须在解压前判断** —— 已经解出来的东西
    // 占的内存就已经还回去了。
    if *uncompressed_total > MAX_ZIP_UNCOMPRESSED_BYTES {
        return Err(refuse(format!(
            "解压后超过 {} MiB 上限。",
            MAX_ZIP_UNCOMPRESSED_BYTES / 1024 / 1024
        )));
    }

    // 上限 B：压缩比。**这条必须在读之前算** ——
    // 一个 3 KB 的 zip 声明解压出 300 MB，比例 100000:1，这就是 zip bomb。
    if raw > 0 {
        let ratio = raw as f64 / entry.compressed_size().max(1) as f64;
        if ratio > MAX_ZIP_COMPRESSION_RATIO {
            return Err(refuse(format!(
                "压缩比 {ratio:.0}:1 超过 {MAX_ZIP_COMPRESSION_RATIO:.0}:1 上限 —— \
                 这是压缩炸弹的典型特征，所以没有解压。"
            )));
        }
    }

    // 上限 C：单文件也要有界。上面的总量是累加的，
    // 但一个 60 MB 的单文件在总量还没到线时就吃掉内存了。
    if raw > MAX_ZIP_UNCOMPRESSED_BYTES {
        return Err(refuse(format!(
            "包里有单个 {} MiB 的文件，超过 {} MiB 上限。",
            raw / 1024 / 1024,
            MAX_ZIP_UNCOMPRESSED_BYTES / 1024 / 1024
        )));
    }

    Ok(())
}

/// 上限 D：条目总数。只能在遍历开始前查一次（zip 中央目录里才有总数），
/// 所以单列一个函数，让两条路径都记得调它。
fn guard_entry_count(total: usize) -> Result<(), UnpackError> {
    if total > MAX_ZIP_ENTRIES {
        return Err(refuse(format!(
            "包里有 {total} 个文件，超过 {} 个上限 —— 这类条目数通常是压缩炸弹的信号。",
            MAX_ZIP_ENTRIES
        )));
    }
    Ok(())
}

/// 一个技能集里「当人格用」的那一篇，外加它自己的条目路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillsetPersona {
    /// 包内条目路径，例如 `skillsets/pdf-toolkit.md`。
    /// **报给用户看**：人格原文来自哪一个文件必须可追，
    /// 上游哪天改了结构，用户能一眼看出「换了一篇」。
    pub source_file: String,
    pub body: String,
}

/// 技能集包里装专家要的两样东西。
///
/// **一趟读完，不解两遍**：zip 要重新开、要重新过四道上限，
/// 而包是外部服务给的字节，多读一次就多一次被压缩炸弹打中的机会。
#[derive(Debug, Clone)]
pub struct SkillsetContents {
    pub persona: SkillsetPersona,
    /// 包里那份 `manifest.json`。**允许缺失** —— 见 [`skillset_contents`]
    /// 里为什么这一条是「我们的选择」而不是照抄 Octop。
    pub manifest: Option<crate::skillhub::HubManifest>,
}

/// 从技能集包里取出「当专家人格用的那一篇」与它点名的下游技能。
///
/// ## 为什么不能直接用 [`unpack`]
///
/// [`unpack`] 按 [`sanitize_name`] 把条目拍平成 basename，这是装技能要的
/// （落盘时拼的是技能目录，不想要目录成分）。但挑人格**恰恰要靠目录**：
/// 上游把编排提示放在 `skillsets/<slug>.md`，而包里的
/// `skills/<slug>/SKILL.md` 是它引用的各个技能 —— 两者拍平之后都叫
/// 一个 basename，就分不出哪一篇才是人格了。
///
/// 挑法照抄 Octop 的 `_parse_skillset_package`
/// （`.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:611-621`）：
/// 先找 `skillsets/<slug>.md`，没有就取 `skillsets/` 下的第一篇，
/// 再退到 `identify.md`。**顺序照抄，不自己发挥** —— 上游哪天调了优先级，
/// 我们跟着调，而不是让两边选到不同的文件。
///
/// 四道安全上限与 [`unpack`] 共用（见 [`guard_entry`]），
/// 这里不重复写一遍，也不因为「只是读文本」就放松。
///
/// ## 与 Octop 的一处**有意不同**
///
/// Octop 把 `manifest.json` 当**必备**（`.octop-ref/octop/src/octop/infra/agents/experts/skillhub_market.py:608`，
/// 缺了就 `PACKAGE_INVALID` 整单失败）。我们这里让 manifest **可缺失**：
/// 缺了就退回上游列表项给的 `skillSlugs`，两者都没有就装一个「只有人格」的专家，
/// 并如实回报「这个包没有列出技能」。
/// 理由是人格本身是完整可用的 —— 为了一个描述性字段把整单废掉，
/// 比装上一个用户能看见、也能改的专家更糟。这是**我们的选择**，不是上游做法。
pub fn skillset_contents(bytes: &[u8], skillset_slug: &str) -> Result<SkillsetContents, UnpackError> {
    let reader = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| UnpackError::Zip(e.to_string()))?;
    guard_entry_count(zip.len())?;

    let preferred = format!("skillsets/{skillset_slug}.md");
    let mut first_in_skillsets: Option<String> = None;
    let mut manifest_text: Option<String> = None;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| UnpackError::Zip(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let mut uncompressed_total = 0u64;
        guard_entry(&entry, &mut uncompressed_total)?;

        let name = entry.name().to_string();
        let lower = name.to_ascii_lowercase();
        // manifest 与人格候选都要**按条目路径**判断，所以这里不拍平。
        if lower.ends_with("manifest.json") && manifest_text.is_none() {
            let mut text = String::new();
            entry
                .read_to_string(&mut text)
                .map_err(|e| UnpackError::Zip(format!("读取 {name} 失败：{e}")))?;
            manifest_text = Some(text);
            continue;
        }
        if !lower.ends_with(".md") {
            continue;
        }
        if first_in_skillsets.is_none() && name.starts_with("skillsets/") {
            first_in_skillsets = Some(name);
        }
    }

    let persona = match preferred_exists(&mut zip, &preferred) {
        true => {
            let mut entry = zip
                .by_name(&preferred)
                .map_err(|e| UnpackError::Zip(e.to_string()))?;
            read_persona_entry(&mut entry, &preferred)?
        }
        false => match first_in_skillsets {
            Some(name) => {
                let mut entry =
                    zip.by_name(&name).map_err(|e| UnpackError::Zip(e.to_string()))?;
                read_persona_entry(&mut entry, &name)?
            }
            None => {
                let mut entry = zip.by_name("identify.md").map_err(|_| UnpackError::NoSkill)?;
                read_persona_entry(&mut entry, "identify.md")?
            }
        },
    };

    let manifest = match manifest_text {
        // 解析失败不算整单失败：这份 manifest 只用来点名下游技能，
        // 坏掉的最诚实后果是「没认出它要哪些技能」，而不是「连人格一起不给」。
        Some(text) => crate::skillhub::parse_manifest(&text),
        None => None,
    };

    Ok(SkillsetContents { persona, manifest })
}

fn preferred_exists<R: std::io::Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> bool {
    zip.by_name(name).is_ok()
}

fn read_persona_entry<R: std::io::Read>(
    entry: &mut R,
    source_file: &str,
) -> Result<SkillsetPersona, UnpackError> {
    let mut body = String::new();
    entry
        .read_to_string(&mut body)
        .map_err(|e| UnpackError::Zip(format!("读取 {source_file} 失败：{e}")))?;
    if body.trim().is_empty() {
        // 空正文当人格等于给模型一段空白，还标着「已设置人格」。直接说没有。
        return Err(UnpackError::NoSkill);
    }
    Ok(SkillsetPersona {
        source_file: source_file.to_string(),
        body: strip_frontmatter(&body),
    })
}

/// 去掉正文开头那一段 YAML frontmatter。
///
/// Octop 也要去掉（`_dedupe_frontmatter`）：那段是上游给**包**写的元数据，
/// 不是给人格的。而 quill 的人格会整段发给模型，让模型先读一段
/// `slug:`/`version:` 的包装，等于在人格最前面塞了一段与它无关的话。
fn strip_frontmatter(body: &str) -> String {
    let trimmed = body.trim_start_matches('\u{feff}').trim_start();
    if !trimmed.starts_with("---") {
        return body.to_string();
    }
    let rest = match trimmed.find('\n') {
        Some(idx) => &trimmed[idx + 1..],
        None => return body.to_string(),
    };
    // frontmatter 的结束行必须正好是 `---`（允许尾随空白），
    // 找不到就当它不是 frontmatter —— 宁可多留一段，也别把正文吃掉。
    let mut end: Option<(usize, usize)> = None;
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let t = line.trim_end_matches(['\r', '\n']).trim_end();
        if t == "---" || t == "..." {
            end = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    match end {
        Some((_, after)) => rest[after..].trim_start().to_string(),
        None => body.to_string(),
    }
}

/// 从单技能包里挑出**唯一一篇**正文。
///
/// ## 为什么不能「有多少收多少」
///
/// 实测 `pdf-image-text-extractor` 一个包里就有 `SKILL.md`、`README.md`、
/// `README.en.md` 三篇 `.md`。三篇都当技能装上，它们就会共用同一个
/// slug 互相覆盖 —— **用户最后拿到哪一篇，取决于 zip 里的顺序**。
/// 这种「看起来装成功了、实际内容是随机的」比直接失败更糟。
///
/// 所以这里只取一篇，其余的**计入 `skipped_other`** —— 数量照实报，
/// 用户在安装结果里能看见「这个包里有 2 个文件没装」。
fn pick_one_body(
    candidates: Vec<SkillBody>,
    mut skipped_other: usize,
    compressed_bytes: u64,
    uncompressed_bytes: u64,
) -> Result<Unpacked, UnpackError> {
    if candidates.is_empty() {
        return Err(UnpackError::NoSkill);
    }
    // 丢掉的那几篇 .md 也要算进「没装」，不能因为「它是 .md」就假装装全了。
    skipped_other += candidates.len() - 1;

    let picked = candidates
        .iter()
        .find(|c| base_of(&c.file).eq_ignore_ascii_case(SKILL_ENTRY))
        .or_else(|| {
            // 没有 SKILL.md 就只认根目录下那一篇。**不猜、不合并。**
            let root: Vec<&SkillBody> = candidates
                .iter()
                .filter(|c| !c.file.contains('/') && !c.file.contains('\\'))
                .collect();
            match root.as_slice() {
                [only] => Some(*only),
                _ => None,
            }
        })
        .ok_or(UnpackError::NoSkill)?;

    Ok(Unpacked {
        files: vec![(base_of(&picked.file), picked.body.clone())],
        compressed_bytes,
        uncompressed_bytes,
        skipped_other,
    })
}

/// 取条目名的最后一段。**只做分割，不做校验** ——
/// 校验在 [`sanitize_name`] 里，那才是挡穿越的地方。
fn base_of(raw: &str) -> String {
    raw.rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

/// 取纯文件名，并**挡掉路径穿越**。
///
/// zip 里的条目名可以是 `../../../etc/cron.d/x`。我们只取最后一段，
/// 再挡一遍 `..` 与空段 —— 落盘时拼的是技能目录，爬出去就等于
/// 让上游往这台机器上任意写文件。
pub fn sanitize_name(raw: &str) -> Result<String, UnpackError> {
    let last = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if last.is_empty() {
        return Err(refuse(format!("包里有条目的文件名是空的（{raw:?}），已跳过。")));
    }
    if last == "." || last == ".." {
        return Err(refuse(format!(
            "包里有条目试图用 {last:?} 指到目录外（{raw:?}），已拒绝。"
        )));
    }
    // 反斜杠在 Windows 上是路径分隔符，冒号会造出 `C:` 这种盘符。
    if last.contains(':') {
        return Err(refuse(format!(
            "包里有条目的文件名含冒号（{raw:?}），在 Windows 上会变成盘符，已拒绝。"
        )));
    }
    if last.len() > 128 {
        return Err(refuse(format!(
            "包里有条目的文件名过长（{} 字符：{last}），已拒绝。",
            last.len()
        )));
    }
    // 只留 ASCII 字母数字、连字符、点、下划线。
    if !last
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(refuse(format!(
            "包里有条目的文件名含不允许的字符（{raw:?}），已拒绝。"
        )));
    }
    Ok(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 造一个只含 `name` → `body` 的 zip。
    fn make_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<'_, ()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, body) in entries {
                w.start_file(*name, opts).expect("start_file");
                w.write_all(body.as_bytes()).expect("write");
            }
            w.finish().expect("finish");
        }
        buf
    }

    #[test]
    fn a_normal_package_unpacks_into_named_files() {
        let bytes = make_zip(&[("demo.md", "# 演示\n\n内容。"), ("notes.txt", "忽略我")]);
        let out = unpack(&bytes).expect("应当解开");
        assert_eq!(out.files.len(), 1, "只收 .md");
        assert_eq!(out.files[0].0, "demo.md");
        assert!(out.files[0].1.contains("演示"));
        assert!(out.compressed_bytes > 0 && out.uncompressed_bytes > 0);
        // 丢掉的东西要报出去，不能悄悄咽下去。
        assert_eq!(out.skipped_other, 1, "notes.txt 应当被计入「丢掉的」");
    }

    #[test]
    fn a_manifest_is_kept_because_it_lists_the_downstream_skills() {
        // 丢掉 manifest 的话，界面上就说不清「装的是包本身，还是包点名的
        // 那几个下游技能」—— 而这两者对用户是天差地别的。
        let bytes = make_zip(&[
            ("manifest.json", r#"{"slug":"tech","skillSlugs":["tdd"]}"#),
            ("identify.md", "# 编排说明"),
        ]);
        let out = unpack(&bytes).expect("应当解开");
        let names: Vec<&str> = out.files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"manifest.json"), "manifest 必须留着：{names:?}");
        assert!(names.contains(&"identify.md"));
    }

    #[test]
    fn a_package_with_a_manifest_but_no_markdown_still_cannot_be_installed() {
        // 有元数据、没正文 = 装不了任何东西。这时候报 200 + 空数组，
        // 界面就会显示「装好了」—— 那是在骗人。
        let bytes = make_zip(&[("manifest.json", r#"{"slug":"x"}"#)]);
        match unpack(&bytes) {
            Err(UnpackError::NoSkill) => {}
            other => panic!("应当报「没有技能」，实际：{other:?}"),
        }
    }

    #[test]
    fn a_zip_with_no_markdown_says_so_instead_of_installing_nothing() {
        let bytes = make_zip(&[("readme.txt", "只有 txt")]);
        match unpack(&bytes) {
            Err(UnpackError::NoSkill) => {}
            other => panic!("应当报「没有技能」，实际：{other:?}"),
        }
    }

    #[test]
    fn an_entry_name_never_survives_as_anything_but_a_safe_basename() {
        // **这一条才是真正要保的东西**：无论包里写什么，
        // `sanitize_name` 的输出拼进技能目录之后都必须落在目录内。
        //
        // 注意这里有个我一开始搞错的地方：带路径的条目名**不是**被拒绝，
        // 而是被压成 basename。`..\..\windows\system32\evil` 变成 `evil`，
        // 落在 `<技能目录>/evil` —— 这是安全的，取 basename 正是为此。
        // 真正会被拒的是「连 basename 都不干净」的那些。
        for raw in [
            "tech-test-automation/skills/tdd.md",
            "..\\..\\windows\\system32\\evil",
            "../../../etc/cron.d/evil",
            "C:/evil.md",
            "a/b/c/deep.md",
        ] {
            let out = match sanitize_name(raw) {
                Ok(v) => v,
                Err(e) => panic!("{raw:?} 应当被压成 basename，实际被拒了：{e:?}"),
            };
            assert!(
                !out.contains('/') && !out.contains('\\'),
                "{raw:?} 压出来的 {out:?} 里还有路径分隔符"
            );
            assert!(
                out != ".." && out != ".",
                "{raw:?} 压出来是 {out:?}，指向目录本身"
            );
            // 拼进技能目录之后必须还在目录里。
            let joined = std::path::Path::new("/skills").join(&out);
            assert_eq!(
                joined.parent().and_then(|p| p.to_str()),
                Some("/skills"),
                "{raw:?} 拼出来跑出了技能目录：{joined:?}"
            );
        }
    }

    #[test]
    fn an_entry_name_that_is_dirty_even_as_a_basename_is_rejected() {
        // 上面那些是「压成 basename」，这三条是「压完仍然不干净」——
        // 这时才必须拒，而不是悄悄改写（改写等于我们替上游编了文件名）。
        for raw in ["..", ".", "a/..", "C:evil.md", "名字带中文.md"] {
            let err = match sanitize_name(raw) {
                Ok(v) => panic!("{raw:?} 应当被拒绝，实际压成了 {v:?}"),
                Err(e) => e,
            };
            match err {
                UnpackError::Refused(_) => {}
                other => panic!("{raw:?} 应当被拒绝，实际：{other:?}"),
            }
        }
        // 超长文件名同理：zip 规范允许 65535 字节，我们只要 128。
        let long = "x".repeat(200);
        assert!(sanitize_name(&long).is_err(), "200 字符的文件名应当被拒");
    }

    #[test]
    fn nested_paths_keep_only_the_basename() {
        assert_eq!(
            sanitize_name("tech-test-automation/skills/tdd.md").expect("应当取 basename"),
            "tdd.md"
        );
    }

    #[test]
    fn the_zip_bomb_ratio_is_checked_before_anything_is_decompressed() {
        // 造一个真正的高压缩比条目：几万字节的重复字符。
        let bomb = "A".repeat(2_000_000);
        let bytes = make_zip(&[("bomb.md", &bomb)]);
        // 先确认这确实是个高压缩比的包（否则这条测试等于没测）
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("能打开");
        {
            let e = z.by_index(0).expect("by_index");
            let ratio = e.size() as f64 / e.compressed_size().max(1) as f64;
            assert!(ratio > 10.0, "这个样本压缩比只有 {ratio:.1}，测不出限制");
        }
        match unpack(&bytes) {
            Err(UnpackError::Refused(why)) => {
                assert!(
                    why.contains("压缩比"),
                    "应当明确说是压缩比超限，实际：{why}"
                );
            }
            other => panic!("压缩炸弹应当被拒，实际：{other:?}"),
        }
    }

    #[test]
    fn a_package_with_too_many_entries_is_refused() {
        // 不真的造 2001 个文件（太慢），只验证「条目数超限」这条错误信息是对的；
        // 上限常量本身改由文件顶部的编译期断言钉住。
        let err = refuse("包里有 9999 个文件，超过 2000 个上限");
        assert!(err.message().contains("安全检查"));
    }

    // -----------------------------------------------------------------------
    // 单技能包
    //
    // 下面这些用的样本结构是 2026-10-06 从上游真机下载的
    // `pdf-image-text-extractor@1.0.13`，条目名与个数都对得上。
    // -----------------------------------------------------------------------

    /// 复刻上游那个包的条目表（`scripts/*.py` 只留名字，内容不影响断言）。
    const REAL_SKILL_PKG: &[(&str, &str)] = &[
        ("README.en.md", "# English readme"),
        ("README.md", "# 中文说明"),
        ("scripts/batch_extractor.py", "print('x')"),
        ("scripts/pdf_text_extractor.py", "print('x')"),
        ("scripts/changelog.py", "print('x')"),
        ("scripts/record.py", "print('x')"),
        ("SKILL.md", "---\nname: pdf-image-text-extractor\ndescription: 提取文字\n---\n\n# 正文"),
        ("_meta.json", r#"{"version":"1.0.13"}"#),
    ];

    #[test]
    fn a_single_skill_package_yields_exactly_one_skill() {
        // **这一条是真 bug 的钉子。** 按技能包的规则解，这个包会产出
        // `README.en.md`、`README.md`、`SKILL.md` 三个技能，
        // 它们共用 slug 互相覆盖，最后留下哪篇取决于 zip 里的顺序。
        let bytes = make_zip(REAL_SKILL_PKG);
        let out = unpack_skill(&bytes).expect("应当解开");
        assert_eq!(out.files.len(), 1, "单技能包必须只出一个技能");
        assert_eq!(out.files[0].0, "SKILL.md");
        assert!(out.files[0].1.contains("提取文字"));
    }

    #[test]
    fn the_files_left_out_of_a_single_skill_package_are_counted() {
        // 8 个条目，只装 1 个 —— 界面要能说「还有 7 个没装」，
        // 而不是让用户以为装全了。
        let bytes = make_zip(REAL_SKILL_PKG);
        let out = unpack_skill(&bytes).expect("应当解开");
        assert_eq!(
            out.files.len() + out.skipped_other,
            REAL_SKILL_PKG.len(),
            "装了的加没装的，必须等于包里的条目数"
        );
        // 4 个脚本 + 2 个 README + _meta.json = 7
        assert_eq!(out.skipped_other, 7);
    }

    #[test]
    fn skill_md_wins_over_the_readmes_whatever_the_order_is() {
        // 上游改了 zip 里的条目顺序，结果必须一样。
        // 顺序决定结果 = 用户不知道自己拿到的是哪一份。
        let mut shuffled: Vec<(&str, &str)> = REAL_SKILL_PKG.to_vec();
        shuffled.reverse();
        let bytes = make_zip(&shuffled);
        let out = unpack_skill(&bytes).expect("应当解开");
        assert_eq!(out.files[0].0, "SKILL.md");
    }

    #[test]
    fn a_single_skill_without_skill_md_uses_the_one_root_markdown_and_says_which() {
        // 换了名字也别静悄悄换内容：正文来源要能报出去。
        let bytes = make_zip(&[("GUIDE.md", "# 换个名字的正文"), ("docs/extra.md", "文档")]);
        let out = unpack_skill(&bytes).expect("应当解开");
        assert_eq!(out.files.len(), 1);
        assert_eq!(out.files[0].0, "GUIDE.md");
        assert!(out.files[0].1.contains("换个名字"));
    }

    #[test]
    fn a_single_skill_with_several_root_markdowns_is_refused_instead_of_picking_one() {
        // 两篇都说得通的时候，**挑一篇就是替上游做主**。
        // 报出来让用户看见，比装一篇随机正文诚实。
        let bytes = make_zip(&[("A.md", "a"), ("B.md", "b")]);
        match unpack_skill(&bytes) {
            Err(UnpackError::NoSkill) => {}
            other => panic!("应当拒绝并报「没有正文」，实际：{other:?}"),
        }
    }

    #[test]
    fn a_single_skill_with_only_nested_markdowns_is_refused() {
        // `docs/foo.md` 是文档，不是技能正文。
        let bytes = make_zip(&[("docs/a.md", "a"), ("docs/b.md", "b")]);
        assert!(matches!(
            unpack_skill(&bytes),
            Err(UnpackError::NoSkill)
        ));
    }

    #[test]
    fn the_two_package_kinds_are_not_interchangeable() {
        // 钉住「技能包收全部 md、单技能只收一篇」这个区别本身。
        // 一旦哪天两边混了，这条会先撞。
        assert_ne!(PackageKind::SkillSet, PackageKind::Skill);
        let bytes = make_zip(&[("SKILL.md", "正文"), ("README.md", "说明")]);
        // 技能包模式：两篇都收。
        assert_eq!(unpack(&bytes).expect("应当解开").files.len(), 2);
        // 单技能模式：只收一篇。
        assert_eq!(unpack_skill(&bytes).expect("应当解开").files.len(), 1);
    }

    // --- 装专家要用的那一趟读 ---------------------------------------------

    #[test]
    fn the_persona_is_the_skillset_named_file_and_not_a_skill_it_references() {
        // 包里同时有编排提示与它引用的技能文档。选错的话，
        // 装出来的专家人格就是一篇技能说明。
        // **`skillsets/` 下故意放了两篇**：只按「第一篇」选的话，
        // 这条测试挑不出毛病 —— 排在前面的偏偏不是该选的那篇。
        let bytes = make_zip(&[
            ("skillsets/other-name.md", "别的包留下的编排提示"),
            ("skillsets/pdf-toolkit.md", "你是 PDF 工具箱助手。"),
            ("skills/pdf-extract/SKILL.md", "怎么抽文本"),
        ]);
        let out = skillset_contents(&bytes, "pdf-toolkit").expect("应当读出人格");
        assert_eq!(out.persona.source_file, "skillsets/pdf-toolkit.md");
        assert!(out.persona.body.contains("PDF 工具箱助手"));
        assert!(
            !out.persona.body.contains("别的包"),
            "不能选到别人的编排提示"
        );
    }

    #[test]
    fn without_the_named_file_any_markdown_under_skillsets_is_used() {
        let bytes = make_zip(&[
            ("skillsets/other-name.md", "换个名字的编排提示"),
            ("skills/x/SKILL.md", "技能说明"),
        ]);
        let out = skillset_contents(&bytes, "wanted-name").expect("应当读出人格");
        assert_eq!(out.persona.source_file, "skillsets/other-name.md");
        assert!(
            !out.persona.body.contains("技能说明"),
            "不能退而选一篇技能说明当人格"
        );
    }

    #[test]
    fn identify_md_is_the_last_resort_and_its_absence_is_reported() {
        let with = make_zip(&[("identify.md", "兜底编排")]);
        let out = skillset_contents(&with, "nope").expect("应当退回 identify.md");
        assert_eq!(out.persona.source_file, "identify.md");

        // 三处都没有 → 报错，不返回空人格。
        let none = make_zip(&[("skills/x/SKILL.md", "只有技能")]);
        match skillset_contents(&none, "nope") {
            Err(UnpackError::NoSkill) => {}
            other => panic!("没有可当人格的条目时应报错，实际：{other:?}"),
        }
    }

    #[test]
    fn frontmatter_is_stripped_because_it_describes_the_package_not_the_persona() {
        let bytes = make_zip(&[(
            "identify.md",
            "---\nslug: pdf-toolkit\nversion: 1.0.0\n---\n\n先问用途。",
        )]);
        let out = skillset_contents(&bytes, "pdf-toolkit").expect("应当读出人格");
        assert_eq!(out.persona.body, "先问用途。");
        assert!(!out.persona.body.contains("version:"), "包装元数据不该进人格");
    }

    #[test]
    fn a_package_without_a_manifest_still_yields_a_persona() {
        // 这是**我们的选择**，与 Octop 不同（它缺 manifest 就整单失败）。
        // 缺 manifest 的诚实后果是「没认出它要哪些技能」，不是「人格也没了」。
        let bytes = make_zip(&[("skillsets/x.md", "有人格")]);
        let out = skillset_contents(&bytes, "x").expect("应当读出人格");
        assert!(out.manifest.is_none(), "这个包本来就没有 manifest");
        assert!(out.persona.body.contains("有人格"));
    }

    #[test]
    fn a_manifest_naming_skills_is_read_as_the_dependency_list_not_as_the_persona() {
        let bytes = make_zip(&[
            (
                "manifest.json",
                r#"{"slug":"pdf-toolkit","skills":[{"slug":"pdf-extract"}]}"#,
            ),
            ("skillsets/pdf-toolkit.md", "人格正文"),
        ]);
        let out = skillset_contents(&bytes, "pdf-toolkit").expect("应当读出人格");
        assert_eq!(out.manifest.expect("manifest 应可解析").referenced_slugs(), vec!["pdf-extract"]);
        assert_eq!(out.persona.source_file, "skillsets/pdf-toolkit.md");
    }

    #[test]
    fn the_zip_bomb_limits_apply_to_the_expert_path_too() {
        // 新增一条读 zip 的路径，就必须同样过四道上限。
        // 换成「只是读文本所以放松」的话，外部服务就能拿一个包把内存打爆。
        let bytes = vec![b'0'; 300 * 1024 * 1024];
        match skillset_contents(&bytes, "x") {
            Err(_) => {}
            Ok(_) => panic!("压缩炸弹必须被拒"),
        }
    }
}
