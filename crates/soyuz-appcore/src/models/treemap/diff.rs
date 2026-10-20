//! The treemap as a visual folder diff: the treemap's folder is compared with another
//! one (`soyuz_search::compare_dirs` on a worker, by date and size or, with
//! `[sync] treemap_diff_content`, by content) and every tile is coloured by what differs
//! instead of by its kind ([`Mark`]). Entries only in the other folder have no tile; they
//! are counted in the legend ([`Diff::only_there`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::{Receiver, bounded};
use soyuz_core::OpControl;
use soyuz_search::CompareResult;

use super::scan::Tree;
use crate::host::Wake;

/// What a tile shows in the diff colouring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mark {
    /// Only in the treemap's folder.
    OnlyHere,
    /// Newer in the treemap's folder.
    NewerHere,
    /// Newer in the other folder.
    NewerThere,
    /// Different without one side being newer (or a file against a folder).
    Differ,
    /// A folder with differences somewhere inside.
    Changed,
    /// The same on both sides.
    Same,
}

impl Mark {
    /// The legend, in order.
    pub const LEGEND: [Mark; 6] =
        [Mark::OnlyHere, Mark::NewerHere, Mark::NewerThere, Mark::Differ, Mark::Changed, Mark::Same];

    pub fn label(self) -> &'static str {
        match self {
            Mark::OnlyHere => tr!("tools-treemap-diff-only-here"),
            Mark::NewerHere => tr!("tools-treemap-diff-newer-here"),
            Mark::NewerThere => tr!("tools-treemap-diff-newer-there"),
            Mark::Differ => tr!("tools-treemap-diff-different"),
            Mark::Changed => tr!("tools-treemap-diff-changed"),
            Mark::Same => tr!("tools-treemap-diff-same"),
        }
    }
}

/// Bytes per mark, in [`Mark`] order.
pub type Totals = Vec<(Mark, u64)>;

/// A finished compare, keyed by path relative to the roots.
#[derive(Clone, Debug, Default)]
pub struct Marks {
    map: BTreeMap<PathBuf, Mark>,
    /// Entries only in the other folder (a missing folder counts once).
    pub only_there: usize,
    /// Files the same on both sides.
    pub same: u64,
}

impl Marks {
    pub fn from_result(r: CompareResult) -> Marks {
        let mut map = BTreeMap::new();
        for (list, mark) in [
            (r.only_left, Mark::OnlyHere),
            (r.newer_left, Mark::NewerHere),
            (r.newer_right, Mark::NewerThere),
            (r.differ, Mark::Differ),
        ] {
            for p in list {
                map.insert(p, mark);
            }
        }
        Marks { map, only_there: r.only_right.len(), same: r.same }
    }

    /// Differences listed (entries only in the other folder included).
    pub fn changes(&self) -> usize {
        self.map.len() + self.only_there
    }

    /// The mark of `rel`: its own, one inherited from a folder that is only here or
    /// differs as a whole, [`Mark::Changed`] for a folder with differences inside (`dir`),
    /// else [`Mark::Same`].
    pub fn mark(&self, rel: &Path, dir: bool) -> Mark {
        if let Some(m) = self.map.get(rel) {
            return *m;
        }
        if let Some(m) = rel.ancestors().skip(1).find_map(|a| self.map.get(a)) {
            return *m;
        }
        // Paths order component-wise, so whatever lies inside `rel` sorts right after it.
        let inside = dir
            && if rel.as_os_str().is_empty() {
                self.changes() > 0
            } else {
                self.map.range(rel.to_path_buf()..).next().is_some_and(|(k, _)| k.starts_with(rel))
            };
        if inside { Mark::Changed } else { Mark::Same }
    }
}

/// The compare of the treemap's folder with `other`.
pub struct Diff {
    pub other: PathBuf,
    pub by_content: bool,
    /// Colour the tiles by difference (else by kind, as without a diff).
    pub show: bool,
    state: DiffState,
    ctl: Arc<OpControl>,
    rx: Option<Receiver<Result<Marks, String>>>,
    /// The legend's byte totals per mark for (level, tree generation).
    totals: Option<((usize, u64), Totals)>,
}

#[derive(Debug)]
enum DiffState {
    Comparing,
    Ready(Marks),
    Failed(String),
}

impl std::fmt::Debug for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Diff").field("other", &self.other).field("state", &self.state).finish_non_exhaustive()
    }
}

impl Drop for Diff {
    fn drop(&mut self) {
        self.ctl.cancel();
    }
}

impl Diff {
    /// Compare `root` with `other` on a worker (`wake` when done).
    pub fn start(root: &Path, other: PathBuf, by_content: bool, wake: Wake) -> Diff {
        let ctl = OpControl::new();
        let (tx, rx) = bounded(1);
        let (a, b, c) = (root.to_path_buf(), other.clone(), ctl.clone());
        let spawned = std::thread::Builder::new().name("soyuz-treemap-diff".into()).spawn(move || {
            let r =
                soyuz_search::compare_dirs(&a, &b, by_content, &c).map(Marks::from_result).map_err(|e| e.to_string());
            let _ = tx.send(r);
            wake();
        });
        let state = match spawned {
            Ok(_) => DiffState::Comparing,
            Err(e) => DiffState::Failed(e.to_string()),
        };
        Diff { other, by_content, show: true, state, ctl, rx: Some(rx), totals: None }
    }

    /// Take the worker's result.
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        if let Ok(r) = rx.try_recv() {
            self.state = match r {
                Ok(m) => DiffState::Ready(m),
                Err(e) => DiffState::Failed(e),
            };
            self.rx = None;
            self.totals = None;
        }
    }

    pub fn is_comparing(&self) -> bool {
        matches!(self.state, DiffState::Comparing)
    }

    pub fn marks(&self) -> Option<&Marks> {
        match &self.state {
            DiffState::Ready(m) => Some(m),
            _ => None,
        }
    }

    /// The compare line ("12 differences from Backup · 1 item only there"; the folder by
    /// name, frontends show the whole path on hover).
    pub fn status(&self) -> String {
        let other = soyuz_core::entry::display_name(&self.other);
        match &self.state {
            DiffState::Comparing => tr!("tools-treemap-diff-comparing", other = other),
            DiffState::Failed(e) => tr!("tools-treemap-diff-failed", other = other, error = e),
            DiffState::Ready(m) => {
                tr!("tools-treemap-diff-compared", other = other, count = m.changes(), there = m.only_there)
            }
        }
    }

    /// Bytes per mark below `level` (files, folded small items and unmeasured folders;
    /// marks without bytes left out), cached until the level or the tree changes.
    pub fn totals(&mut self, root: &Path, tree: &Tree, level: usize) -> Totals {
        let key = (level, tree.generation);
        if let Some((k, t)) = &self.totals
            && *k == key
        {
            return t.clone();
        }
        let Some(marks) = self.marks() else { return Vec::new() };
        let mut sum: BTreeMap<Mark, u64> = BTreeMap::new();
        let mut stack = vec![level];
        while let Some(id) = stack.pop() {
            let n = &tree.nodes[id];
            if n.is_dir && !n.children.is_empty() {
                stack.extend(n.children.iter().copied());
                continue;
            }
            let rel = n.path.strip_prefix(root).unwrap_or(&n.path);
            let mark = if n.other {
                // The folded small items take their folder's own (or inherited) mark.
                marks.mark(rel.parent().unwrap_or(rel), false)
            } else {
                marks.mark(rel, n.is_dir)
            };
            *sum.entry(mark).or_default() += n.bytes;
        }
        let t: Totals = sum.into_iter().filter(|(_, b)| *b > 0).collect();
        self.totals = Some((key, t.clone()));
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn marks_inherit_and_roll_up() {
        let r = CompareResult {
            only_left: vec![p("new")],
            only_right: vec![p("gone"), p("old.txt")],
            newer_left: vec![p("a/b/edited.txt")],
            newer_right: vec![],
            differ: vec![p("clash")],
            same: 4,
        };
        let m = Marks::from_result(r);
        assert_eq!(m.changes(), 5);
        assert_eq!(m.mark(&p("new"), true), Mark::OnlyHere);
        assert_eq!(m.mark(&p("new/x/y.bin"), false), Mark::OnlyHere, "inside a folder only here");
        assert_eq!(m.mark(&p("clash/inner.txt"), false), Mark::Differ);
        assert_eq!(m.mark(&p("a/b/edited.txt"), false), Mark::NewerHere);
        assert_eq!(m.mark(&p("a"), true), Mark::Changed);
        assert_eq!(m.mark(&p("a/b"), true), Mark::Changed);
        assert_eq!(m.mark(&p("a/c"), true), Mark::Same);
        assert_eq!(m.mark(&p("a0"), true), Mark::Same, "a sibling sorting after a/ is not inside it");
        assert_eq!(m.mark(&p("a/same.txt"), false), Mark::Same);
        assert_eq!(m.mark(&p(""), true), Mark::Changed, "the root holds every difference");
        let none = Marks::default();
        assert_eq!(none.mark(&p(""), true), Mark::Same);
        assert!(Mark::LEGEND.iter().all(|m| !m.label().is_empty()));
    }
}
