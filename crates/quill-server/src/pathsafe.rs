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
//! ## 铁律：两侧必须走**同一个**归一化
//!
//! `std::fs::canonicalize` 在 Windows 上返回**逐字路径**（verbatim
//! path），带 `\\?\` 前缀：`\\?\D:\tmp\x\skills`。不带前缀的
//! `D:\tmp\x\skills` 永远不会 `starts_with` 它。
//!
//! 所以「把候选路径 canonicalize 一下再跟 root 比」这种写法在 Windows
//! 上是**系统性失败**：只要目录真的存在，canonicalize 就成功，
//! 前缀就出现，比较就恒假。目录不存在时 canonicalize 失败、走
//! 「保持原样」的兜底，两边反而相等 —— 于是这个 bug 在一部分路径上
//! 自己藏起来，只在「目录已存在」（也就是正常使用的那条路）上炸。
//!
//! 两条规矩，缺一不可：
//! 1. root 与 candidate 都过 `canonical_or_self`；
//! 2. 要问的必须是**能 canonicalize 的路径**。还没落盘的文件
//!    canonicalize 一定失败，所以对「要写入的新文件」要问它的
//!    **父目录**，不是问文件自己。

use std::path::{Path, PathBuf};

/// canonicalize 一次；失败就是 `None`，**不**在这里兜底 ——
/// 「归一成功没有」这件事本身是判定所需的信息，见 `is_within`。
fn try_canonical(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

/// 归一化一个路径；canonicalize 失败就原样返回。
fn canonical_or_self(p: &Path) -> PathBuf {
    try_canonical(p).unwrap_or_else(|| p.to_path_buf())
}

/// `candidate` 是否落在 `root` 之内（含 root 自身）。
///
/// **两侧都过同一套归一化**，这是本模块存在的全部理由。
/// 只归一化一侧在 Windows 上恒假（见模块文档）。
///
/// **残留 `..` 一律拒绝**，但只在它真的残留时：canonicalize 成功
/// 的话 `..` 已经被解掉；没成功的那一侧还留着 `..`，此时唯一的
/// 比法就是字面前缀，而那恰恰是最容易骗过 `starts_with` 的东西。
/// （注意不能无条件拒绝 —— 那会把「同一个目录的不同拼法」也一起
/// 拒掉，而那正是要判对的东西。）
///
/// 语义上判的是**目录**：对尚未落盘的文件请传它的父目录。
pub fn is_within(root: &Path, candidate: &Path) -> bool {
    let r = try_canonical(root);
    let c = try_canonical(candidate);
    if r.is_none() && has_parent_dir(root) {
        return false;
    }
    if c.is_none() && has_parent_dir(candidate) {
        return false;
    }
    let r = r.unwrap_or_else(|| canonical_or_self(root));
    let c = c.unwrap_or_else(|| canonical_or_self(candidate));
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
        assert!(!is_within(
            &skills,
            &evil.join("x.md").parent().unwrap()
        ));
    }
}
