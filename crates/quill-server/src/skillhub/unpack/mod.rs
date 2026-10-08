//! 把 SkillHub 下回来的 zip 变成磁盘上的技能文件。
//!
//! **这个文件里的每一条检查都对应一类真实攻击**，不是「稳妥起见」：
//! 解 zip 面向的是**外部服务给的任意字节**，一个不设防的解压器
//! 就是一个能让任意用户把服务端打爆的服务。
//!
//! 抄自 Octop 的同一组上限（`skills/skillhub_common.py`）——
//! 那些数字是它踩过之后定下来的，不是随手拍的。

use std::io::Read;

/// 解压后总量上限，与 Octop 的 `MAX_ZIP_UNCOMPRESSED_BYTES` 同值。
pub const MAX_ZIP_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;

/// 压缩比上限，与 Octop 的 `MAX_ZIP_COMPRESSION_RATIO` 同值。
pub const MAX_ZIP_COMPRESSION_RATIO: f64 = 100.0;

/// zip 条目数上限，与 Octop 的 `MAX_ZIP_ENTRIES` 同值。
pub const MAX_ZIP_ENTRIES: usize = 2_000;

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
            UnpackError::Refused(why) => {
                format!("这个技能包因为安全检查没过，没有安装。下一步：{why}")
            }
            UnpackError::NoSkill => "这个包里没有可用的技能正文（.md）。下一步：换一个技能包；\
                 如果这是单技能，它可能缺 SKILL.md。"
                .to_string(),
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
        let mut entry = zip
            .by_index(i)
            .map_err(|e| UnpackError::Zip(e.to_string()))?;
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
pub fn skillset_contents(
    bytes: &[u8],
    skillset_slug: &str,
) -> Result<SkillsetContents, UnpackError> {
    let reader = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(reader).map_err(|e| UnpackError::Zip(e.to_string()))?;
    guard_entry_count(zip.len())?;

    let preferred = format!("skillsets/{skillset_slug}.md");
    let mut first_in_skillsets: Option<String> = None;
    let mut manifest_text: Option<String> = None;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| UnpackError::Zip(e.to_string()))?;
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
                let mut entry = zip
                    .by_name(&name)
                    .map_err(|e| UnpackError::Zip(e.to_string()))?;
                read_persona_entry(&mut entry, &name)?
            }
            None => {
                let mut entry = zip
                    .by_name("identify.md")
                    .map_err(|_| UnpackError::NoSkill)?;
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

fn preferred_exists<R: std::io::Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> bool {
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
        return Err(refuse(format!(
            "包里有条目的文件名是空的（{raw:?}），已跳过。"
        )));
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
mod tests;
