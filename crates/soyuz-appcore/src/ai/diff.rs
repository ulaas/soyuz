//! Plan previews without a toolkit: the before / after trees of a plan (added, moved,
//! renamed and deleted nodes, the way the folder-compare tool colours them) and the
//! changed-characters split of a rename. Frontends draw them (the GUI's
//! `ui::ai::diff`, the terminal UI's AI panel).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use soyuz_ai::tools::{Plan, PlanItem, PlanOp};

/// What happens to one node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Change {
    Added,
    Moved,
    Renamed,
    Deleted,
    Changed,
    /// A folder shown only for structure.
    Context,
}

impl Change {
    /// Short label ("added", "moved", …); empty for [`Change::Context`].
    pub fn label(self) -> &'static str {
        match self {
            Change::Added => tr!("ai-diff-added"),
            Change::Moved => tr!("ai-diff-moved"),
            Change::Renamed => tr!("ai-diff-renamed"),
            Change::Deleted => tr!("ai-diff-deleted"),
            Change::Changed => tr!("ai-diff-changed"),
            Change::Context => "",
        }
    }
}

/// One side of a plan item: the path and what happens to it.
pub type Side = Option<(PathBuf, Change)>;

/// `(before, after)` entries of one plan item.
pub fn sides(i: &PlanItem) -> (Side, Side) {
    match i.op {
        PlanOp::Move => (i.src.clone().map(|s| (s, Change::Moved)), i.dst.clone().map(|d| (d, Change::Moved))),
        PlanOp::Rename => (i.src.clone().map(|s| (s, Change::Renamed)), i.dst.clone().map(|d| (d, Change::Renamed))),
        PlanOp::Copy | PlanOp::SyncCopy => (None, i.dst.clone().map(|d| (d, Change::Added))),
        // Media tools write a new file beside the input (an overwrite replaces it).
        PlanOp::MediaConvert | PlanOp::MediaExtractAudio | PlanOp::MediaTrim | PlanOp::MediaFrame => {
            (None, i.dst.clone().map(|d| (d, Change::Added)))
        }
        PlanOp::Create | PlanOp::Mkdir | PlanOp::Extract | PlanOp::Compress | PlanOp::Restore => {
            (None, i.dst.clone().map(|d| (d, Change::Added)))
        }
        PlanOp::Trash | PlanOp::Delete | PlanOp::SyncDelete | PlanOp::EmptyTrash => {
            (i.src.clone().map(|s| (s, Change::Deleted)), None)
        }
        PlanOp::SetMeta | PlanOp::Tag => {
            (i.src.clone().map(|s| (s, Change::Changed)), i.src.clone().map(|s| (s, Change::Changed)))
        }
        PlanOp::Undo | PlanOp::RunCommand => (None, None),
    }
}

/// The distinct kinds of change in a plan, sorted (for a legend).
pub fn changes(plan: &Plan) -> Vec<Change> {
    let mut seen: Vec<Change> = plan
        .items
        .iter()
        .flat_map(|i| {
            let (a, b) = sides(i);
            a.into_iter().chain(b).map(|(_, c)| c)
        })
        .collect();
    seen.sort();
    seen.dedup();
    seen
}

fn comps(p: &Path) -> Vec<String> {
    p.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

/// Longest common ancestor folder of `paths`.
pub fn common_root(paths: &[&Path]) -> PathBuf {
    let mut it = paths.iter();
    let Some(first) = it.next() else { return PathBuf::new() };
    let mut common: Vec<String> = comps(first.parent().unwrap_or(first));
    for p in it {
        let c = comps(p.parent().unwrap_or(p));
        let n = common.iter().zip(&c).take_while(|(a, b)| a.eq_ignore_ascii_case(b)).count();
        common.truncate(n);
    }
    let mut out = PathBuf::new();
    for c in common {
        out.push(if c.ends_with(':') { format!("{c}\\") } else { c });
    }
    out
}

/// A tree node (children by name).
#[derive(Default, Debug)]
pub struct Node {
    pub change: Option<Change>,
    pub path: PathBuf,
    pub dir: bool,
    pub children: BTreeMap<String, Node>,
}

impl Node {
    fn insert(&mut self, root: &Path, path: &Path, change: Change, dir: bool) {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let parts = comps(rel);
        let mut node = self;
        let mut cur = root.to_path_buf();
        let n = parts.len();
        for (i, part) in parts.into_iter().enumerate() {
            cur.push(&part);
            let child = node.children.entry(part).or_insert_with(|| Node {
                path: cur.clone(),
                dir: true,
                ..Default::default()
            });
            if i + 1 == n {
                child.change = Some(change);
                child.dir = dir || !child.children.is_empty();
            }
            node = child;
        }
    }

    /// Number of nodes below this one.
    pub fn count(&self) -> usize {
        self.children.values().map(|c| 1 + c.count()).sum()
    }
}

/// The before / after trees of a plan, relative to their common root.
pub fn trees(plan: &Plan) -> (PathBuf, Node, Node) {
    let all: Vec<&Path> =
        plan.items.iter().flat_map(|i| i.src.iter().chain(i.dst.iter())).map(PathBuf::as_path).collect();
    let root = common_root(&all);
    let mut before = Node::default();
    let mut after = Node::default();
    for i in &plan.items {
        let dir = i.kind == "dir";
        let (b, a) = sides(i);
        if let Some((p, c)) = b {
            before.insert(&root, &p, c, dir);
        }
        if let Some((p, c)) = a {
            after.insert(&root, &p, c, dir);
        }
    }
    (root, before, after)
}

/// `(prefix, changed, suffix)` split of `old` vs `new` by common prefix / suffix (chars).
pub fn char_diff<'a>(old: &'a str, new: &'a str) -> ((&'a str, &'a str, &'a str), (&'a str, &'a str, &'a str)) {
    let oc: Vec<(usize, char)> = old.char_indices().collect();
    let nc: Vec<(usize, char)> = new.char_indices().collect();
    let pre = oc.iter().zip(&nc).take_while(|(a, b)| a.1 == b.1).count();
    let max_suf = oc.len().min(nc.len()) - pre;
    let suf = oc.iter().rev().zip(nc.iter().rev()).take(max_suf).take_while(|(a, b)| a.1 == b.1).count();
    let cut = |s: &'a str, v: &[(usize, char)], from: usize, to_from_end: usize| -> (&'a str, &'a str, &'a str) {
        let start = v.get(from).map_or(s.len(), |x| x.0);
        let end_idx = v.len() - to_from_end;
        let end = v.get(end_idx).map_or(s.len(), |x| x.0);
        (&s[..start], &s[start..end], &s[end..])
    };
    (cut(old, &oc, pre, suf), cut(new, &nc, pre, suf))
}

/// The file name of a plan path (empty when there is none).
pub fn name_of(p: &Option<PathBuf>) -> String {
    p.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_diff_finds_the_changed_middle() {
        assert_eq!(
            char_diff("IMG_0001.jpg", "2026-06-14 IMG_0001.jpg"),
            (("", "", "IMG_0001.jpg"), ("", "2026-06-14 ", "IMG_0001.jpg"))
        );
        assert_eq!(
            char_diff("report draft.pdf", "report final.pdf"),
            (("report ", "draft", ".pdf"), ("report ", "final", ".pdf"))
        );
        assert_eq!(char_diff("same", "same"), (("same", "", ""), ("same", "", "")));
        assert_eq!(char_diff("añb", "aüb"), (("a", "ñ", "b"), ("a", "ü", "b")));
        assert_eq!(char_diff("aaa", "aa"), (("aa", "a", ""), ("aa", "", "")));
    }

    fn item(root: &Path, op: PlanOp, src: Option<&str>, dst: Option<&str>) -> PlanItem {
        PlanItem {
            op,
            src: src.map(|s| root.join(s)),
            dst: dst.map(|d| root.join(d)),
            kind: "file".into(),
            bytes: 1,
            entries: 1,
            conflict: None,
        }
    }

    fn plan(items: Vec<PlanItem>) -> Plan {
        Plan {
            tool: "move".into(),
            summary: String::new(),
            risk: soyuz_ai::tools::Risk::Write,
            items,
            total_bytes: 0,
            total_entries: 0,
            conflicts: 0,
            executable: true,
            warnings: Vec::new(),
            action: soyuz_ai::tools::PlanAction::None,
        }
    }

    fn root() -> PathBuf {
        if cfg!(windows) { PathBuf::from("C:\\w") } else { PathBuf::from("/w") }
    }

    #[test]
    fn trees_from_a_move_plan() {
        let root = root();
        let plan = plan(vec![
            item(&root, PlanOp::Move, Some("a.jpg"), Some("Images/a.jpg")),
            item(&root, PlanOp::Mkdir, None, Some("Images")),
            item(&root, PlanOp::Trash, Some("junk.tmp"), None),
        ]);
        let (r, before, after) = trees(&plan);
        assert_eq!(r, root);
        assert_eq!(before.children.len(), 2);
        assert_eq!(before.children["junk.tmp"].change, Some(Change::Deleted));
        assert_eq!(before.children["a.jpg"].change, Some(Change::Moved));
        assert_eq!(after.children["Images"].change, Some(Change::Added));
        assert_eq!(after.children["Images"].children["a.jpg"].change, Some(Change::Moved));
        assert_eq!(after.children["Images"].children["a.jpg"].path, root.join("Images").join("a.jpg"));
        assert!(after.children["Images"].dir);
        assert!(!before.children["a.jpg"].dir);
        assert_eq!(before.count(), 2);
        assert_eq!(after.count(), 2);
        assert_eq!(changes(&plan), vec![Change::Added, Change::Moved, Change::Deleted]);
    }

    #[test]
    fn trees_keep_structure_folders_as_context() {
        let root = root();
        let plan = plan(vec![
            item(&root, PlanOp::Rename, Some("x/y/old.txt"), Some("x/y/new.txt")),
            item(&root, PlanOp::Rename, Some("x/z.txt"), Some("x/q.txt")),
            item(&root, PlanOp::Tag, Some("x/t.md"), None),
        ]);
        let (r, before, after) = trees(&plan);
        assert_eq!(r, root.join("x"));
        let y = &before.children["y"];
        assert_eq!(y.change, None, "a structure-only folder");
        assert!(y.dir);
        assert_eq!(y.children["old.txt"].change, Some(Change::Renamed));
        assert_eq!(after.children["y"].children["new.txt"].change, Some(Change::Renamed));
        assert_eq!(before.children["t.md"].change, Some(Change::Changed));
        assert_eq!(after.children["t.md"].change, Some(Change::Changed));
        assert_eq!(name_of(&plan.items[0].dst), "new.txt");
        assert_eq!(name_of(&None), "");
    }

    #[test]
    fn common_root_of_nothing_and_of_siblings() {
        assert_eq!(common_root(&[]), PathBuf::new());
        let root = root();
        let a = root.join("a").join("1.txt");
        let b = root.join("A").join("2.txt");
        // Case-insensitive component match keeps the first spelling.
        assert_eq!(common_root(&[a.as_path(), b.as_path()]), root.join("a"));
    }
}
