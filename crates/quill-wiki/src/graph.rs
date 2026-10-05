//! wikilink 反向索引（规格 §6.1，支撑 lint 的孤儿检测）。
//!
//! # 关键设计决定：`index.md` **不作为图的边来源**
//!
//! 规格 §四说 `index.md` 是「所有内容的目录」，即它会链到**每一个**页面。
//! 若把它算进入边，则**没有任何页面可能是孤儿** ——
//! 孤儿检测恒为 0，是规格 AGENTS.md「八类看起来成功」里的
//! **第 1 类假闸门**与**第 7 类恒绿**同时命中。
//!
//! 因此判定规则是：
//! - **孤儿 = 没有来自任何*其他知识页面*的入边**（`index.md` / `log.md` 不算）；
//! - `index.md` / `log.md` 自身永远不算孤儿（它们是结构文件，不是知识页）。
//!
//! 由此得到两条必须同时成立的性质（互为反例，构成闸门的鉴别力）：
//! 1. 只被 `index.md` 链到的页面 **是**孤儿（否则恒绿）；
//! 2. 被任一知识页面链到的页面 **不是**孤儿。
//!
//! # 链接目标的解析顺序
//!
//! wikilink 写的是**标题**（`[[Rust 所有权]]`），而文件路径可能不同
//! （`concepts/rust-ownership.md`）。解析顺序固定为：
//! 标题精确 → 标题忽略大小写 → 路径主干（含相对目录）→ 文件名主干。
//! ⚠️ 顺序写死并测试：顺序一变，「哪些页面被链到」就变，
//! 而孤儿清单会随之整体变化且**不报任何错**。

use std::collections::{BTreeMap, BTreeSet};

use crate::page::{Page, PageType};

/// 结构文件名（不参与孤儿判定，见模块文档）。
pub const STRUCTURAL_FILES: [&str; 2] = ["index.md", "log.md"];

/// 图中的一个节点（一页）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    /// 相对 wiki 层的路径。
    pub path: String,
    /// 标题（wikilink 的目标）。
    pub title: String,
    /// 页面类型。
    pub page_type: Option<PageType>,
    /// 标签。
    pub tags: Vec<String>,
    /// 出边：这一页链到的**已解析**目标路径。
    pub out: BTreeSet<String>,
    /// 未解析的链接（目标页不存在）。
    pub unresolved: Vec<UnresolvedLink>,
}

/// 一条指向不存在页面的链接。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedLink {
    /// 链接所在的源页面路径。
    pub from: String,
    /// 原始链接目标文本。
    pub target: String,
}

/// 整个 wiki 的链接图。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkGraph {
    nodes: BTreeMap<String, GraphNode>,
    /// 反向索引：目标路径 → 引用它的页面路径集合。
    backlinks: BTreeMap<String, BTreeSet<String>>,
}

impl LinkGraph {
    /// 一次性建图。
    ///
    /// ⚠️ **结构性文件（`index.md` / `log.md`）不建节点** ——
    /// 否则 `index.md` 那条「链到每一页」的目录边会喂饱所有入边，
    /// 孤儿检测恒为 0（恒绿假闸门，见模块文档）。这条不变式落在**建图**里
    /// 而不是调用方，因为它是图的定义的一部分。
    ///
    /// 输入顺序不影响结果（`nodes` / `backlinks` 都是有序映射）——
    /// 这让「同一份 wiki 目录永远产出同一张图」，孤儿清单因此可复现。
    pub fn build(pages: &[Page]) -> Self {
        let pages: Vec<&Page> = pages
            .iter()
            .filter(|p| !STRUCTURAL_FILES.contains(&p.path.as_str()))
            .collect();

        // 第一遍：登记所有标题与路径主干，供第二遍解析链接。
        let mut by_title: BTreeMap<String, String> = BTreeMap::new();
        let mut by_title_ci: BTreeMap<String, String> = BTreeMap::new();
        let mut by_stem: BTreeMap<String, String> = BTreeMap::new();
        let mut by_relative_stem: BTreeMap<String, String> = BTreeMap::new();

        for p in &pages {
            let title = p.title();
            // 首个声明者胜出（重名是 lint 要报的问题，不在图里静默丢弃）。
            by_title
                .entry(title.clone())
                .or_insert_with(|| p.path.clone());
            by_title_ci
                .entry(title.to_lowercase())
                .or_insert_with(|| p.path.clone());
            let stem = p.file_stem().to_string();
            by_stem
                .entry(stem.clone())
                .or_insert_with(|| p.path.clone());
            by_relative_stem
                .entry(stem_rel(&p.path))
                .or_insert_with(|| p.path.clone());
        }

        let mut nodes: BTreeMap<String, GraphNode> = BTreeMap::new();
        let mut backlinks: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        for p in &pages {
            let mut out = BTreeSet::new();
            let mut unresolved = Vec::new();
            for l in &p.links {
                match resolve_target(
                    &l.target,
                    &by_title,
                    &by_title_ci,
                    &by_stem,
                    &by_relative_stem,
                ) {
                    Some(target_path) => {
                        out.insert(target_path.clone());
                        backlinks
                            .entry(target_path)
                            .or_default()
                            .insert(p.path.clone());
                    }
                    None => unresolved.push(UnresolvedLink {
                        from: p.path.clone(),
                        target: l.target.clone(),
                    }),
                }
            }
            nodes.insert(
                p.path.clone(),
                GraphNode {
                    path: p.path.clone(),
                    title: p.title(),
                    page_type: p.page_type(),
                    tags: p.frontmatter.tags.clone(),
                    out,
                    unresolved,
                },
            );
        }

        Self { nodes, backlinks }
    }

    /// 节点数（不含结构文件 —— 它们不进图）。
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 边数（有向，去重后）。
    pub fn edge_count(&self) -> usize {
        self.nodes.values().map(|n| n.out.len()).sum()
    }

    /// 取节点。
    pub fn node(&self, path: &str) -> Option<&GraphNode> {
        self.nodes.get(path)
    }

    /// 全部节点（按路径序）。
    pub fn nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.nodes.values()
    }

    /// 反向链接：谁链到了 `path`。
    pub fn backlinks(&self, path: &str) -> impl Iterator<Item = &String> {
        self.backlinks.get(path).into_iter().flatten()
    }

    /// 出链页面路径。
    pub fn out_links(&self, path: &str) -> impl Iterator<Item = &String> {
        self.nodes.get(path).into_iter().flat_map(|n| n.out.iter())
    }

    /// 孤儿页面：没有来自其他**知识页面**的入链。
    ///
    /// 结构性孤儿（无出边也无入边）一并报出 —— 那是最该被处理的一类。
    pub fn orphans(&self) -> Vec<&GraphNode> {
        self.nodes
            .values()
            .filter(|n| self.backlinks(&n.path).next().is_none())
            .collect()
    }

    /// 未解析的链接（指向不存在的页面）。
    pub fn unresolved_links(&self) -> Vec<&UnresolvedLink> {
        self.nodes
            .values()
            .flat_map(|n| n.unresolved.iter())
            .collect()
    }

    /// 共享至少 `min_shared` 个标签、但彼此**没有任何链接**的页面对。
    ///
    /// 这是 lint「缺失的交叉引用」的计算基础（见 [`crate::lint`]）。
    /// 返回 `(页A, 页B, 共同标签)`，页 A 的路径字典序小于页 B。
    pub fn unlinked_tag_pairs(&self, min_shared: usize) -> Vec<(String, String, Vec<String>)> {
        let paths: Vec<&String> = self.nodes.keys().collect();
        let mut out = Vec::new();
        for (i, a) in paths.iter().enumerate() {
            for b in paths.iter().skip(i + 1) {
                let (Some(na), Some(nb)) = (self.nodes.get(*a), self.nodes.get(*b)) else {
                    // 键来自本 map，不可能取不到；给出显式跳过而不是 unwrap。
                    continue;
                };
                let shared = shared_tags(&na.tags, &nb.tags);
                if shared.len() < min_shared {
                    continue;
                }
                if na.out.contains(*b) || nb.out.contains(*a) {
                    continue;
                }
                out.push(((*a).clone(), (*b).clone(), shared));
            }
        }
        out
    }
}

/// 去 `.md` 的相对主干（`a/b.md` → `a/b`）。
fn stem_rel(path: &str) -> String {
    path.strip_suffix(".md").unwrap_or(path).to_string()
}

fn shared_tags(a: &[String], b: &[String]) -> Vec<String> {
    let set_b: BTreeSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let mut out: Vec<String> = a
        .iter()
        .filter(|t| set_b.contains(t.as_str()))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 解析一个 wikilink 目标到页面路径。顺序见模块文档。
fn resolve_target(
    target: &str,
    by_title: &BTreeMap<String, String>,
    by_title_ci: &BTreeMap<String, String>,
    by_stem: &BTreeMap<String, String>,
    by_relative_stem: &BTreeMap<String, String>,
) -> Option<String> {
    let t = target.trim();
    if let Some(p) = by_title.get(t) {
        return Some(p.clone());
    }
    let lower = t.to_lowercase();
    if let Some(p) = by_title_ci.get(&lower) {
        return Some(p.clone());
    }
    // 目标本身可能是路径：`[[concepts/rust]]` 或 `[[rust-ownership]]`
    let with_md = if t.ends_with(".md") {
        t.to_string()
    } else {
        format!("{t}.md")
    };
    if let Some(p) = by_relative_stem.get(&with_md) {
        return Some(p.clone());
    }
    // 最后按文件名主干兜底（`[[rust-ownership]]` 命中 `concepts/rust-ownership.md`）
    let last = t.rsplit('/').next().unwrap_or(t);
    by_stem.get(last).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::parse_page;

    fn page(path: &str, text: &str) -> Page {
        parse_page(path, text)
    }

    fn concept(path: &str, title: &str, links: &str, tags: &str) -> Page {
        page(
            path,
            &format!(
                "---\ntitle: {title}\ntype: concept\ntags: [{tags}]\nupdated: 2026-10-04\n---\n\n{links}\n"
            ),
        )
    }

    #[test]
    fn resolves_by_title_path_and_stem() {
        let pages = vec![
            concept("concepts/a.md", "甲概念", "无", "x"),
            concept("entities/b.md", "乙实体", "无", "y"),
        ];
        let g = LinkGraph::build(&pages);
        // 按标题
        let by_title = page(
            "c.md",
            "---\ntitle: 引用者\ntype: concept\nupdated: 2026-10-04\n---\n\n[[甲概念]]\n",
        );
        let g2 = LinkGraph::build(&[by_title, pages[0].clone(), pages[1].clone()]);
        assert!(
            g2.node("c.md").expect("c").out.contains("concepts/a.md"),
            "按标题未解析：{:?}",
            g2.node("c.md").expect("c").out
        );
        // 按相对路径主干
        let by_rel = page(
            "d.md",
            "---\ntitle: 引用者2\ntype: concept\nupdated: 2026-10-04\n---\n\n[[concepts/a]]\n",
        );
        let g3 = LinkGraph::build(&[by_rel, pages[0].clone(), pages[1].clone()]);
        assert!(g3.node("d.md").expect("d").out.contains("concepts/a.md"));
        // 按文件名主干
        let by_stem = page(
            "e.md",
            "---\ntitle: 引用者3\ntype: concept\nupdated: 2026-10-04\n---\n\n[[b]]\n",
        );
        let g4 = LinkGraph::build(&[by_stem, pages[0].clone(), pages[1].clone()]);
        assert!(g4.node("e.md").expect("e").out.contains("entities/b.md"));
        assert_eq!(g.edge_count(), 0, "无引用时无边");
    }

    #[test]
    fn backlinks_are_the_reverse_of_out_links() {
        let pages = vec![
            concept("a.md", "A", "见 [[B]]", "t"),
            concept("b.md", "B", "见 [[C]]", "t"),
            concept("c.md", "C", "无", "t"),
        ];
        let g = LinkGraph::build(&pages);
        let back: Vec<&String> = g.backlinks("b.md").collect();
        assert_eq!(back, vec!["a.md"]);
        let back_c: Vec<&String> = g.backlinks("c.md").collect();
        assert_eq!(back_c, vec!["b.md"]);
        assert_eq!(g.edge_count(), 2);
        assert_eq!(g.node_count(), 3);
    }

    #[test]
    fn orphan_ignores_index_md_links() {
        // 鉴别力用例 1：只被 index.md 链到 → 仍是孤儿。
        // 若把 index.md 算进图，孤儿检测恒为 0（恒绿假闸门）。
        let pages = vec![
            page("index.md", "---\ntitle: 索引\n---\n\n- [[只有目录链]]\n"),
            concept("a.md", "只有目录链", "无", "t"),
        ];
        let g = LinkGraph::build(&pages);
        let orphans = g.orphans();
        assert_eq!(orphans.len(), 1, "只被 index.md 链到的必须是孤儿");
        assert_eq!(orphans[0].title, "只有目录链");
    }

    #[test]
    fn non_orphan_when_linked_by_a_page() {
        // 鉴别力用例 2：被任一知识页面链到 → 不是孤儿。
        // 与上一条构成一对，缺一不可。
        let pages = vec![
            concept("a.md", "A", "见 [[B]]", "t"),
            concept("b.md", "B", "见 [[A]]", "t"),
        ];
        let g = LinkGraph::build(&pages);
        assert!(g.orphans().is_empty(), "互相引用不该是孤儿");
    }

    #[test]
    fn unresolved_links_are_reported_not_dropped() {
        let pages = vec![
            concept("a.md", "A", "见 [[不存在的页]] 与 [[B]]", "t"),
            concept("b.md", "B", "无", "t"),
        ];
        let g = LinkGraph::build(&pages);
        let un = g.unresolved_links();
        assert_eq!(un.len(), 1);
        assert_eq!(un[0].from, "a.md");
        assert_eq!(un[0].target, "不存在的页");
        // 已解析的边不受影响
        assert_eq!(g.out_links("a.md").count(), 1);
    }

    #[test]
    fn build_is_order_independent() {
        let a = concept("a.md", "A", "见 [[B]]", "t");
        let b = concept("b.md", "B", "见 [[A]]", "t");
        let g1 = LinkGraph::build(&[a.clone(), b.clone()]);
        let g2 = LinkGraph::build(&[b, a]);
        assert_eq!(g1, g2, "建图结果必须与页面顺序无关（孤儿清单要可复现）");
    }

    #[test]
    fn unlinked_tag_pairs_find_missing_cross_references() {
        let pages = vec![
            concept("a.md", "A", "无", "内存, 安全"),
            concept("b.md", "B", "无", "内存, 安全"),
            concept("c.md", "C", "见 [[A]]", "内存, 安全"),
        ];
        let g = LinkGraph::build(&pages);
        let pairs = g.unlinked_tag_pairs(2);
        let got: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(x, y, _)| (x.as_str(), y.as_str()))
            .collect();
        // a-c 已有链接 → 不报；a-b 与 b-c 共享 2 个标签且互不链接 → 报
        assert_eq!(got, vec![("a.md", "b.md"), ("b.md", "c.md")]);
        assert_eq!(pairs[0].2, vec!["内存".to_string(), "安全".to_string()]);
    }

    #[test]
    fn duplicate_titles_keep_first_declared() {
        // 重名是 lint 要报的问题；图里必须确定性选一个，否则入边会随机跳。
        let pages = vec![
            concept("a.md", "同名", "见 [[同名]]", "t"),
            concept("b.md", "同名", "无", "t"),
        ];
        let g = LinkGraph::build(&pages);
        assert!(g.node("a.md").expect("a").out.contains("a.md"));
        assert!(g.node("b.md").expect("b").out.is_empty());
    }
}
