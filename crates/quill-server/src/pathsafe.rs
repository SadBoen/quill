//! 「这个路径在不在那个目录里」——全项目**唯一**的判定口径。
//!
//! ## 为什么单独一个模块，而不是谁用到谁写一份
//!
//! `tools.rs` 里写着这句话：「两处各写一份是这个项目最容易出的错」。
//! 它已经真的错过一次了：`api_extensions::skill_file` 把**原始** root
//! 跟**已 canonicalize** 的 root 放在一起比，于是 Windows 上每个技能
//! 都被判成「落在技能目录之外」而拒绝写入 —— 界面报「保存失败」，
//! 原因在代码里根本看不出来。
//!
//! ## 铁律：判定**不能**取决于某一侧碰巧在不在磁盘上
//!
//! `std::fs::canonicalize` 在 Windows 上返回**逐字路径**（verbatim
//! path），带 `\\?\` 前缀：`\\?\D:\tmp\x\skills`。不带前缀的
//! `D:\tmp\x\skills` 永远不会 `starts_with` 它 —— `Path::starts_with`
//! 按**分量**比，`Prefix(VerbatimDisk)` 就是不等于 `Prefix(Disk)`。
//!
//! 旧实现让两侧各走一次「canonicalize，失败就原样返回」，于是判定
//! 结果取决于**谁存在**：root 存在、candidate 不存在时，两边的前缀
//! 一个有、一个没有，结果恒为 false。而「我要写的这个新文件在不在
//! 技能目录里」正是最自然的问法，它在 Windows 上永远得到「不在」。
//! 方向是**误拒**不是漏放，但它是当初那个 bug 的镜像（原 bug 是
//! 漏放的另一极），症状同样是「每个技能保存失败」而原因在代码里
//! 根本看不出来；更糟的是「判定对不对取决于文件在不在盘上」这种
//! 模块会主动去掉的性质，又从后门回来了。
//!
//! 现在两侧都过同一个 `compare_form`，判定只取决于**拼法**。
//!
//! ## `compare_form` 的三步，每一步挡住什么
//!
//! 1. **canonicalize** —— 成功就用它。它顺带解掉符号链接、junction
//!    与 `..`。
//! 2. **整条失败就退到最长的那个能 canonicalize 的祖先**，再把剩下
//!    的分量接回去。还没落盘的文件走的就是这条路。
//! 3. **两侧统一**去掉 `\\?\` 逐字前缀（仅 Windows），并逐分量折叠
//!    大小写 —— Windows 上路径大小写不敏感，而 `Path::starts_with`
//!    是逐字节比的。
//!
//! 第 2 步不只是为了「让判定与存在性无关」，它同时是**唯一**能发现
//! 「中间目录是个链接」的手段：canonicalize 必须整条路径都存在才能
//! 成功，所以 `root/link/new.md`（`link` 指向 root 之外）里最后那
//! 一层不会让 canonicalize 成功，整条路径就此走兜底。实测（junction，
//! 无需管理员权限）：只做第 1、3 步的话，对**尚未落盘的文件**问它
//! 自己会得到「在根目录内」—— `canonicalize` 返回 `NotFound`，那一侧
//! 于是完全没解链接；问它的父目录才会解析出链接的真实目标并拒绝。
//! 退到最长存在祖先之后，这种情况直接被拒，不再依赖调用方记得
//! 「要问父目录」。
//!
//! ## 剩下的两条规矩
//!
//! 1. root 与 candidate 都过 `compare_form`（本模块的全部理由）。
//! 2. **残留 `..` 一律拒绝，但只在它真的残留时**：canonicalize 成功
//!    的话 `..` 已经被解掉。没成功的那一侧还留着 `..`，此时唯一
//!    的比法就是字面前缀，而那恰恰是最容易骗过 `starts_with` 的东西。
//!    （不能无条件拒绝 —— 那会把「同一个目录的不同拼法」也一起
//!    拒掉，而那正是要判对的东西。）

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf, Prefix};

/// canonicalize 一次；失败就是 `None`，**不**在这里兜底 ——
/// 「归一成功没有」这件事本身是判定所需的信息，见 `is_within`。
fn try_canonical(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

/// 把一侧变成**可比较的拼法**。root 与 candidate 必须都过它：
/// 只有同一个函数同时处理了「存在 / 不存在」「有无 `\\?\`」「大小写」，
/// 判定才与两侧碰巧在不在盘上无关（见模块文档）。
fn compare_form(canonical: Option<PathBuf>, raw: &Path) -> PathBuf {
    // canonicalize 成功的那一侧：它已经解掉了符号链接与 `..`，直接用。
    // 失败的那一侧：退到最长的那个能 canonicalize 的祖先，把剩下的分量
    // 接回去 —— 「要写入的新文件」以及「还没建的 root」都走这条路。
    let base = match canonical {
        Some(c) => c,
        None => resolve_existing_ancestor(raw),
    };
    fold_case(&strip_verbatim(&base))
}

/// 整条路径 canonicalize 失败时的退路：退到**最长的那个能 canonicalize
/// 的祖先**，把被退掉的分量原样接回去。
///
/// 为什么要退到祖先而不是原样返回：canonicalize 需要整条路径存在，
/// 所以「还没落盘的文件」必然失败。若原样返回，那一侧就完全没解符号
/// 链接 —— 实测中 `root/link/new.md`（`link` 是指向 root 之外的
/// junction）会被判成「在根目录内」。退到最长存在祖先之后，
/// `root/link` 会被解析成真实目标，判定随之正确。
///
/// `..` 留给 `is_within` 的守卫拒绝：这里不解析它，也不该解析 ——
/// 解析了就等于替调用方猜意图。
fn resolve_existing_ancestor(p: &Path) -> PathBuf {
    let mut tail: Vec<OsString> = Vec::new();
    let mut head = p;
    loop {
        if let Some(c) = try_canonical(head) {
            let mut out = c;
            // 退出来的分量要按原序接回去
            for name in tail.iter().rev() {
                out.push(name);
            }
            return out;
        }
        let (Some(parent), Some(name)) = (head.parent(), head.file_name()) else {
            // 没有更短的前缀可试了（或分量已经不能再拆，如盘符根）：
            // 原样返回，交给调用方的比较与守卫去处理。
            return p.to_path_buf();
        };
        tail.push(name.to_os_string());
        head = parent;
    }
}

/// 去掉 `\\?\` 逐字前缀。canonicalize 的产物带这个前缀，调用方给的
/// 路径不带；不去掉的话**两边必不等**，而这正是本模块要杀的 bug。
///
/// 非 Windows 上不存在逐字前缀，`has_verbatim_prefix` 恒为 false，
/// 于是这段完全不做字符串转换 —— 也就不存在把非 UTF-8 路径
/// `to_string_lossy` 打坏的风险。
fn strip_verbatim(p: &Path) -> PathBuf {
    if !has_verbatim_prefix(p) {
        return p.to_path_buf();
    }
    let s = p.as_os_str().to_string_lossy();
    // `\\?\UNC\server\share` 去掉前缀后会退化成 `UNC\server\share`，
    // 那不是合法 UNC 拼法（盘符会被当成普通分量名），所以要还原成
    // `\\server\share`。
    let plain = match s.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => match s.strip_prefix(r"\\?\") {
            Some(rest) => rest.to_string(),
            None => return p.to_path_buf(),
        },
    };
    PathBuf::from(plain)
}

fn has_verbatim_prefix(p: &Path) -> bool {
    p.components().any(|c| match c {
        Component::Prefix(pc) => {
            matches!(
                pc.kind(),
                Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _)
            )
        }
        _ => false,
    })
}

/// Windows 上路径大小写不敏感，而 `Path::starts_with` 是逐字节比的：
/// 实测 `\\?\C:\USERS\...\QUILL-PROBE-DIR` 与同目录的小写拼法互相
/// `starts_with` 都为 false。同一侧要么两种拼法都归到 canonicalize、
/// 要么都不成功，所以不折叠大小写就会把「大小写不同的同一目录」
/// 判成两个地方。
///
/// 只折叠**分量**，不折叠整个字符串：折叠后仍然靠 `Path::starts_with`
/// 按分量比较，`skills` 与 `skills-evil`、`skills.md` 因此仍然判为
/// 不在内（见 `a_sibling_sharing_a_name_prefix_is_not_within`）。
///
/// 非 Windows 上文件系统大小写敏感，折叠会把两个真实不同的目录
/// 混为一谈，所以那里什么都不做。
///
/// 只折叠**能安全折叠的分量**：不是合法 UTF-8 的分量原样保留，否则
/// 两个不同的目录名会被 lossy 替换折叠成同一个（见 `push_folded`）。
#[cfg(windows)]
fn fold_case(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Prefix(pc) => push_folded(&mut out, pc.as_os_str()),
            Component::Normal(name) => push_folded(&mut out, name),
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 折叠**一个**分量的大小写；不是合法 UTF-8 的分量原样保留。
///
/// 为什么不走 `to_string_lossy().to_lowercase()`：lossy 会把每一个非法
/// 字节替换成**同一个** U+FFFD，于是 `0xD800` 与 `0xD801` 这样两个
/// 不同的目录名折叠成同一个 —— 而「两个不同的路径被判成同一个」正是
/// 本模块唯一不能犯的错（漏放，比误拒严重一个方向）。
///
/// 不折叠的最坏后果只是「同一个目录的大小写拼法被判成两个」，那是
/// 误拒，方向是安全的，交给调用方看到「保存失败」也比放进别人的
/// 目录里好。
#[cfg(windows)]
fn push_folded(out: &mut PathBuf, part: &std::ffi::OsStr) {
    match part.to_str() {
        Some(s) => out.push(std::ffi::OsStr::new(&s.to_lowercase())),
        None => out.push(part),
    }
}

#[cfg(not(windows))]
fn fold_case(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// `candidate` 是否落在 `root` 之内（含 root 自身）。
///
/// **两侧都过同一个 `compare_form`**，这是本模块存在的全部理由。
/// 判定因此只取决于路径**怎么拼**，不取决于它在不在磁盘上：
/// 「root 存在、要写入的新文件不存在」这种最常见的组合，
/// 在 Windows 上同样判得对（旧实现在这里恒为 false）。
///
/// **残留 `..` 一律拒绝**，但只在它真的残留时：canonicalize 成功
/// 的话 `..` 已经被解掉；没成功的那一侧还留着 `..`，此时唯一的
/// 比法就是字面前缀，而那恰恰是最容易骗过 `starts_with` 的东西。
/// （注意不能无条件拒绝 —— 那会把「同一个目录的不同拼法」也一起
/// 拒掉，而那正是要判对的东西。）
///
/// 语义上判的是**路径**，可以直接传尚未落盘的文件：没落盘的那一侧
/// 会退到最长的存在祖先，符号链接与 junction 照样解得开（见模块
/// 文档里 junction 的实测）。但**判目录通常更省事**：目录存在时
/// 一步 canonicalize 就到位，不必退祖先。
pub fn is_within(root: &Path, candidate: &Path) -> bool {
    let r = try_canonical(root);
    let c = try_canonical(candidate);
    if r.is_none() && has_parent_dir(root) {
        return false;
    }
    if c.is_none() && has_parent_dir(candidate) {
        return false;
    }
    let r = compare_form(r, root);
    let c = compare_form(c, candidate);
    c.starts_with(&r)
}

fn has_parent_dir(p: &Path) -> bool {
    p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个真实存在的临时目录，测完删掉。
    ///
    /// **存在的意义是让 `canonicalize` 成功** —— 而那正是原 bug 的触发条件。
    /// 用不存在的目录测，canonicalize 会走兜底，bug 自己就藏起来了，
    /// 这正是它当初能混过 Linux/WSL 上大部分路径的原因。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let p = std::env::temp_dir().join(format!(
                "quill-pathsafe-{tag}-{}-{}",
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&p).expect("建临时目录");
            TempDir(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn sub(&self, name: &str) -> PathBuf {
            let p = self.0.join(name);
            std::fs::create_dir_all(&p).expect("建子目录");
            p
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_plain_child_is_within() {
        let root = Path::new("/a/skills");
        assert!(is_within(root, &root.join("code-review.md")));
        assert!(is_within(root, Path::new("/a/skills")));
    }

    #[test]
    fn parent_dir_escape_is_not_within() {
        let root = Path::new("/a/skills");
        for bad in [
            "/a/skills/../evil/x.md",
            "/a/skills/../../etc/passwd",
            "/a/other",
            "/a",
        ] {
            assert!(
                !is_within(root, Path::new(bad)),
                "{bad:?} 不该被判成在根目录内"
            );
        }
    }

    /// 字符串前缀陷阱：`skills-evil` 与 `skills` 共享开头，朴素的前缀
    /// 比较会说「在里面」。`Path::starts_with` 是按**路径分量**比的，
    /// 所以这是对的 —— 但它是本模块最该钉死的一条，因为一旦有人
    /// 「简化」成 `to_string_lossy().starts_with()`，这里立刻静默失效。
    #[test]
    fn a_sibling_sharing_a_name_prefix_is_not_within() {
        let root = Path::new("/a/skills");
        assert!(!is_within(root, Path::new("/a/skills-evil/x.md")));
        assert!(!is_within(root, Path::new("/a/skills2")));
        assert!(!is_within(root, Path::new("/a/skills.md")));
    }

    #[test]
    fn root_that_does_not_exist_yet_still_works() {
        // 目录不存在 → 两侧都走兜底 → 判定仍然成立。
        // 这一条是原 bug 的「藏身处」，不是它的修复；留着是为了
        // 明确记录「不存在时不会误拒」，别让人以为修复只对存在时有效。
        let root = Path::new("/a/does-not-exist-yet/skills");
        assert!(is_within(root, &root.join("x.md")));
    }

    /// 复现原 bug 的那一条：root 与 candidate 是**同一个真实目录**，
    /// 只是拼法不同（一个带 `..`、一个不带）。
    ///
    /// 旧代码拿**原始** root 去比**已 canonicalize** 的 candidate，
    /// 在 Windows 上这里恒为 false（`\\?\` 前缀），于是每个技能都被
    /// 判成「落在技能目录之外」。现在两侧同归一化，必然为 true。
    #[test]
    fn an_existing_root_is_judged_the_same_however_it_is_spelled() {
        let tmp = TempDir::new("spelling");
        let skills = tmp.sub("skills");

        // 带 `..` 与 `.` 的同一目录：canonicalize 会把它抹平。
        let spelled = tmp.path().join("sub").join("..").join("skills");
        std::fs::create_dir_all(tmp.path().join("sub")).expect("建 sub");

        assert!(
            is_within(&skills, &skills.join("code-review.md").parent().unwrap()),
            "存在的 root 判自己的子目录必须为真"
        );
        assert!(
            is_within(&spelled, &spelled.join("code-review.md").parent().unwrap()),
            "同一目录的不同拼法必须判出同一个结果（Windows 的 \\\\?\\ 前缀陷阱）"
        );
        assert!(
            is_within(&skills, &spelled),
            "拼法不同不该改变判定：这是被原 bug 打破的不变量"
        );
        assert!(is_within(&spelled, &skills));
    }

    #[test]
    fn an_existing_root_does_not_admit_its_prefix_sibling() {
        let tmp = TempDir::new("sibling");
        let skills = tmp.sub("skills");
        let evil = tmp.sub("skills-evil");
        assert!(!is_within(&skills, &evil));
        assert!(!is_within(&skills, &evil.join("x.md").parent().unwrap()));
    }

    /// **缺陷本身**：root 存在、candidate 不存在。
    ///
    /// 这是「我要写的这个新文件在不在技能目录里」这个最自然的问法。
    /// 旧实现里 root 被 canonicalize 成 `\\?\C:\...\skills`，
    /// candidate 原样留在 `C:\...\skills\new.md`，
    /// `Prefix(VerbatimDisk)` 不等于 `Prefix(Disk)`，于是恒为 false ——
    /// 也就是说**每一个新技能的保存都被判成「落在技能目录之外」**。
    /// 方向是误拒不是漏放，但它就是当初那个 bug 的镜像。
    #[test]
    fn an_existing_root_admits_a_file_that_does_not_exist_yet() {
        let tmp = TempDir::new("mixed-new");
        let skills = tmp.sub("skills");
        let new_file = skills.join("code-review.md");
        assert!(!new_file.exists(), "这个测试要的就是 candidate 真的不存在");

        assert!(
            is_within(&skills, &new_file),
            "存在的 root 必须认下还没落盘的新文件（\\?\\ 前缀陷阱的镜像 bug）"
        );
        assert!(is_within(&skills, &skills));

        // 同一条判定仍然要挡住真正的越界：同前缀兄弟目录里的新文件。
        let evil = tmp.sub("skills-evil");
        assert!(!is_within(&skills, &evil.join("code-review.md")));
        assert!(!is_within(&skills, &skills.join("../skills-evil/x.md")));
    }

    /// 上一条的反向：root 不存在、candidate 存在。
    ///
    /// 旧实现里这一侧是「藏身处」：两侧都不存在时拼法恰好一致，
    /// 判定碰巧正确，于是 bug 的一半路径自己把自己藏住了。
    /// 现在 candidate 侧被 canonicalize、带上了 `\\?\`，root 侧却没有，
    /// 所以这一格必须显式钉住。
    ///
    /// **为什么正反不对称**：root 不存在、candidate 存在、而 candidate
    /// 又在 root 之下，这在真实文件系统上是**不可能**的（子路径存在
    /// 就意味着它每一级祖先都存在）。所以这一格能断言的只有拒绝方向
    /// —— 存在的 candidate 也不在 ghost 之内，以及同一个 ghost 对
    /// 「存在的候选」与「不存在的候选」给出同一个答案。
    #[test]
    fn a_root_that_does_not_exist_yet_still_judges_an_existing_candidate() {
        let tmp = TempDir::new("mixed-root");
        let real = tmp.sub("real");
        let existing = real.join("已有.md");
        std::fs::write(&existing, "内容".as_bytes()).expect("写文件");
        let ghost = real.join("not-created-yet");
        assert!(!ghost.exists());

        // 存在的 candidate，但不在 ghost 之下：必须拒。
        assert!(
            !is_within(&ghost, &existing),
            "存在的 candidate 落在不存在的 root 之外，必须拒"
        );
        // 不存在的 candidate 在 ghost 之内：必须收。
        assert!(
            is_within(&ghost, &ghost.join("x.md")),
            "不存在的 root 仍要判出里面的路径"
        );
        // ghost 就在 real 之内，所以反过来问是「收」——
        // 这同时又钉了一次「存在的 root + 不存在的 candidate」。
        assert!(is_within(&real, &ghost));
        // real 显然不在 ghost 之内（方向反了就该拒）。
        assert!(!is_within(&ghost, &real));
        // 平级目录仍然被拒。
        let sibling = tmp.sub("real-evil");
        assert!(!is_within(&ghost, &sibling.join("x.md")));
    }

    /// 判定只认拼法，不认存在性 —— 同一个 root、同一批候选，
    /// 存在的与不存在的必须给出**同一个**答案。
    #[test]
    fn the_verdict_does_not_depend_on_whether_either_side_exists() {
        let tmp = TempDir::new("independent");
        let skills = tmp.sub("skills");
        let listed = skills.join("listed.md");
        let fresh = skills.join("fresh.md");
        std::fs::write(&listed, b"x").expect("写文件");
        assert!(listed.exists() && !fresh.exists());

        for candidate in [&listed, &fresh] {
            assert!(
                is_within(&skills, candidate),
                "{candidate:?} 的判定不该因为它在不在盘上而变化"
            );
        }
    }

    /// 非 ASCII 路径：canonicalize 不该把它们判成目录之外。
    /// 顺带钉住「逐字前缀不能变成 `starts_with` 的隐形墙」。
    #[test]
    fn non_ascii_names_with_spaces_work_in_both_existence_states() {
        let tmp = TempDir::new("cjk");
        let cjk = tmp.sub("技能 文档");
        assert!(is_within(&cjk, &cjk.join("新建 笔记.md")));
        assert!(is_within(&cjk, &cjk.join("技能 文档.md")));
        let written = cjk.join("已存在.md");
        std::fs::write(&written, "内容".as_bytes()).expect("写文件");
        assert!(is_within(&cjk, &written));
        assert!(!is_within(&cjk, &cjk.join("子目录/../别的/x.md")));
    }

    /// 符号链接 / junction 指向 root 之外时必须被拒，**哪怕要判的那个
    /// 文件还没落盘**。
    ///
    /// 这条是「判定与存在性无关」这件事的代价，必须一起钉死：
    /// canonicalize 要求整条路径都存在，所以没落盘的那一层必然失败，
    /// 如果失败就原样返回，那一侧根本没解链接。实测（junction，
    /// 无需管理员权限）`canonicalize(root/link/new.md)` 返回
    /// `NotFound`，旧写法在这里会放行。现在退到最长存在祖先，
    /// `root/link` 被解析成真实目标，判定随之正确。
    ///
    /// **只跑 Windows**：链接的建法本身就分平台（`symlink_dir` vs
    /// `mklink /J`），而且这正是 Windows 上才踩到的坑 ——
    /// 非 Windows 上文件系统大小写敏感、没有逐字路径前缀，这条路径的
    /// 价值完全不同，硬跑只会把「跨平台能编译」这件事弄坏。
    #[cfg(windows)]
    #[test]
    fn a_not_yet_written_file_behind_a_link_outside_root_is_rejected() {
        let tmp = TempDir::new("link");
        let root = tmp.sub("root");
        let outside = tmp.sub("outside");
        let link = root.join("link");

        // symlink_dir 在没开开发者模式的机器上要管理员权限；
        // 退到 junction（mklink /J），两者语义在这里够用。
        if std::os::windows::fs::symlink_dir(&outside, &link).is_err() {
            let status = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .status();
            assert!(status.map(|s| s.success()).unwrap_or(false), "建 junction");
        }
        assert!(link.exists(), "链接没建成，这条测试就失去意义");

        let not_written = link.join("escape.md");
        assert!(!not_written.exists());
        assert!(
            !is_within(&root, &not_written),
            "没落盘的文件也不能绕过指向外部的链接"
        );
        assert!(!is_within(&root, &link), "链接本身就不在 root 内");
        // 链接之内但仍落在 root 内的那一侧要放行，避免把判定做死。
        assert!(is_within(&root, &root.join("ok.md")));
    }

    /// 大小写不同的同一目录，在 Windows 上必须判成同一个地方 ——
    /// 实测 `\\?\C:\USERS\...` 与小写拼法互相 `starts_with` 都为 false，
    /// 不折叠大小写就会把一个目录当成两个。
    ///
    /// 非 Windows 上文件系统大小写敏感，折叠会把两个真实不同的目录
    /// 混为一谈，所以那里没有这条断言。
    #[cfg(windows)]
    #[test]
    fn spelling_case_does_not_change_the_verdict() {
        let tmp = TempDir::new("case");
        let skills = tmp.sub("skills");
        let shouted = PathBuf::from(skills.to_string_lossy().to_uppercase());
        assert_ne!(skills, shouted, "拼法必须真的不同，这条测试才有意义");
        assert!(is_within(&skills, &shouted));
        assert!(is_within(&shouted, &skills));
        assert!(is_within(&shouted, &shouted.join("x.md")));
        // 两边**都不存在**、只有大小写不同的同一个目录名：
        // ancestor 回退对两边各自解析不出任何东西能比，只有折叠
        // 大小写之后它们才是同一个目录。这是 fold_case 唯一不可省
        // 的一格，也是「判定只取决于拼法」的最后一格。
        let ghost_upper = tmp.path().join("GHOST-DIR");
        let ghost_lower = tmp.path().join("ghost-dir");
        assert!(!ghost_upper.exists() && !ghost_lower.exists());
        assert!(is_within(&ghost_upper, &ghost_lower.join("x.md")));
        assert!(is_within(&ghost_lower, &ghost_upper.join("x.md")));
        // 大小写折叠不得把同前缀兄弟也一起放进来。
        let evil = tmp.sub("skills-evil");
        assert!(!is_within(
            &shouted,
            &PathBuf::from(evil.to_string_lossy().to_uppercase())
        ));
    }

    /// 大小写折叠**不得**把两个不同的目录名折叠成同一个。
    ///
    /// 这条钉的是折叠本身的实现选择：`to_string_lossy` 会把每一个
    /// 非法字节替换成同一个 U+FFFD，于是 `0xD800` 与 `0xD801` 两个
    /// 不同的目录名折叠后相等 —— 而「两个不同路径被判成同一个」正是
    /// 本模块唯一不能犯的方向（漏放）。所以不是合法 UTF-8 的分量
    /// 一律**不折叠**：最坏后果只是同一目录的大小写拼法被判成两个
    /// （误拒），而误拒会在界面上变成一句「保存失败」。
    #[cfg(windows)]
    #[test]
    fn lossy_conversion_must_not_merge_two_distinct_components() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        // 两个都不成对代理：各自都不是合法 UTF-8，但互不相同。
        let a = OsString::from_wide(&[0xD800]);
        let b = OsString::from_wide(&[0xD801]);
        assert_ne!(a, b);

        let folded_a = fold_case(Path::new(&a));
        let folded_b = fold_case(Path::new(&b));
        assert_ne!(
            folded_a, folded_b,
            "两个不同分量被折叠成同一个：非法字节被替换成同一个字符就会制造漏放"
        );

        // 合法 UTF-8 的分量仍然要折叠（否则 Windows 上同一目录会被
        // 当成两个地方，见 spelling_case_does_not_change_the_verdict）。
        assert_eq!(
            fold_case(Path::new("SKILLS")),
            fold_case(Path::new("skills")),
            "合法 UTF-8 的分量必须照常折叠大小写"
        );
    }
}
