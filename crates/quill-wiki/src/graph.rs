use std::collections::{BTreeMap, BTreeSet};

use crate::page::{Page, PageType};

pub const STRUCTURAL_FILES: [&str; 2] = ["index.md", "log.md"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub path: String,

    pub title: String,

    pub page_type: Option<PageType>,

    pub tags: Vec<String>,

    pub out: BTreeSet<String>,

    pub unresolved: Vec<UnresolvedLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedLink {
    pub from: String,

    pub target: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkGraph {
    nodes: BTreeMap<String, GraphNode>,

    backlinks: BTreeMap<String, BTreeSet<String>>,
}

impl LinkGraph {
    pub fn build(pages: &[Page]) -> Self {
        let pages: Vec<&Page> = pages
            .iter()
            .filter(|p| !STRUCTURAL_FILES.contains(&p.path.as_str()))
            .collect();

        let mut by_title: BTreeMap<String, String> = BTreeMap::new();
        let mut by_title_ci: BTreeMap<String, String> = BTreeMap::new();
        let mut by_stem: BTreeMap<String, String> = BTreeMap::new();
        let mut by_relative_stem: BTreeMap<String, String> = BTreeMap::new();

        for p in &pages {
            let title = p.title();

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

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.nodes.values().map(|n| n.out.len()).sum()
    }

    pub fn node(&self, path: &str) -> Option<&GraphNode> {
        self.nodes.get(path)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &GraphNode> {
        self.nodes.values()
    }

    pub fn backlinks(&self, path: &str) -> impl Iterator<Item = &String> {
        self.backlinks.get(path).into_iter().flatten()
    }

    pub fn out_links(&self, path: &str) -> impl Iterator<Item = &String> {
        self.nodes.get(path).into_iter().flat_map(|n| n.out.iter())
    }

    pub fn orphans(&self) -> Vec<&GraphNode> {
        self.nodes
            .values()
            .filter(|n| self.backlinks(&n.path).next().is_none())
            .collect()
    }

    pub fn unresolved_links(&self) -> Vec<&UnresolvedLink> {
        self.nodes
            .values()
            .flat_map(|n| n.unresolved.iter())
            .collect()
    }

    pub fn unlinked_tag_pairs(&self, min_shared: usize) -> Vec<(String, String, Vec<String>)> {
        let paths: Vec<&String> = self.nodes.keys().collect();
        let mut out = Vec::new();
        for (i, a) in paths.iter().enumerate() {
            for b in paths.iter().skip(i + 1) {
                let (Some(na), Some(nb)) = (self.nodes.get(*a), self.nodes.get(*b)) else {
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

    let with_md = if t.ends_with(".md") {
        t.to_string()
    } else {
        format!("{t}.md")
    };
    if let Some(p) = by_relative_stem.get(&with_md) {
        return Some(p.clone());
    }

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

        let by_rel = page(
            "d.md",
            "---\ntitle: 引用者2\ntype: concept\nupdated: 2026-10-04\n---\n\n[[concepts/a]]\n",
        );
        let g3 = LinkGraph::build(&[by_rel, pages[0].clone(), pages[1].clone()]);
        assert!(g3.node("d.md").expect("d").out.contains("concepts/a.md"));

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

        assert_eq!(got, vec![("a.md", "b.md"), ("b.md", "c.md")]);
        assert_eq!(pairs[0].2, vec!["内存".to_string(), "安全".to_string()]);
    }

    #[test]
    fn duplicate_titles_keep_first_declared() {
        let pages = vec![
            concept("a.md", "同名", "见 [[同名]]", "t"),
            concept("b.md", "同名", "无", "t"),
        ];
        let g = LinkGraph::build(&pages);
        assert!(g.node("a.md").expect("a").out.contains("a.md"));
        assert!(g.node("b.md").expect("b").out.is_empty());
    }
}
