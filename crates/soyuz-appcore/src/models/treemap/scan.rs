//! Treemap data: an arena of nodes built from `soyuz_search::size_tree` results, and the
//! background scan that fills it progressively (the root's subfolders are measured one
//! after another so tiles appear while the scan runs; deeper levels load on drill-in).
//! Moved from the GUI's `ui/tools/treemap/scan.rs` (Phase T7); [`Tree::generation`] lets a
//! frontend cache a layout until the tree changes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::Sender;
use soyuz_core::OpControl;
use soyuz_search::TreeNode;

use crate::file_class::{FileClass, classify_ext};

/// Levels of detail fetched below each scanned folder.
pub const DEPTH: usize = 4;

/// A subtree produced on a worker thread.
#[derive(Clone, Debug)]
pub struct SubTree {
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    pub is_dir: bool,
    /// The folded "smaller items" node.
    pub other: bool,
    /// Children are known (false for folders beyond the scan depth).
    pub complete: bool,
    pub children: Vec<SubTree>,
}

impl SubTree {
    pub fn file(path: PathBuf, bytes: u64) -> SubTree {
        SubTree {
            name: soyuz_core::entry::display_name(&path),
            path,
            bytes,
            files: 1,
            is_dir: false,
            other: false,
            complete: true,
            children: Vec::new(),
        }
    }
}

/// Convert a `size_tree` node (stats only the ambiguous leaves: one file, no children).
pub fn convert(n: TreeNode) -> SubTree {
    let other = n.is_other();
    let has_children = !n.children.is_empty();
    let is_dir = !other
        && (has_children || n.files != 1 || std::fs::symlink_metadata(&n.path).map(|m| m.is_dir()).unwrap_or(false));
    let complete = !(is_dir && !has_children && n.files > 0);
    let name =
        if other { tr!("tools-treemap-smaller-items").to_string() } else { soyuz_core::entry::display_name(&n.path) };
    SubTree {
        name,
        path: n.path,
        bytes: n.bytes,
        files: n.files,
        is_dir,
        other,
        complete,
        children: n.children.into_iter().map(convert).collect(),
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    pub is_dir: bool,
    pub other: bool,
    pub complete: bool,
    pub loading: bool,
    pub class: FileClass,
    pub parent: Option<usize>,
    /// Sorted by bytes, descending.
    pub children: Vec<usize>,
}

impl Node {
    /// Display name (the folded node follows the current UI language).
    pub fn label(&self) -> &str {
        if self.other { tr!("tools-treemap-smaller-items") } else { &self.name }
    }
}

/// Node arena; index 0 is the root.
#[derive(Clone, Debug)]
pub struct Tree {
    pub nodes: Vec<Node>,
    /// Bumped whenever nodes are added, replaced or removed (layout caches key on it).
    pub generation: u64,
}

fn class_of(name: &str, is_dir: bool, other: bool) -> FileClass {
    if is_dir {
        return FileClass::Folder;
    }
    if other {
        return FileClass::Other;
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => classify_ext(&ext.to_ascii_lowercase()),
        _ => FileClass::Other,
    }
}

impl Tree {
    pub fn new(root: &Path) -> Tree {
        Tree {
            nodes: vec![Node {
                name: soyuz_core::entry::display_name(root),
                path: root.to_path_buf(),
                bytes: 0,
                files: 0,
                is_dir: true,
                other: false,
                complete: false,
                loading: true,
                class: FileClass::Folder,
                parent: None,
                children: Vec::new(),
            }],
            generation: 0,
        }
    }

    fn push(&mut self, parent: usize, s: SubTree) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            class: class_of(&s.name, s.is_dir, s.other),
            name: s.name,
            path: s.path,
            bytes: s.bytes,
            files: s.files,
            is_dir: s.is_dir,
            other: s.other,
            complete: s.complete,
            loading: false,
            parent: Some(parent),
            children: Vec::new(),
        });
        let mut kids: Vec<usize> = s.children.into_iter().map(|c| self.push(id, c)).collect();
        kids.sort_by_key(|&k| std::cmp::Reverse(self.nodes[k].bytes));
        self.nodes[id].children = kids;
        id
    }

    /// Add a scanned child under `parent`, updating totals up the chain.
    pub fn add_child(&mut self, parent: usize, s: SubTree) -> usize {
        let (bytes, files) = (s.bytes, s.files);
        self.generation += 1;
        let id = self.push(parent, s);
        self.nodes[parent].children.push(id);
        self.sort(parent);
        self.bump(parent, bytes as i128, files as i128);
        id
    }

    /// Replace the children of `at` with a freshly scanned subtree (drill-in).
    pub fn replace(&mut self, at: usize, s: SubTree) {
        let old_bytes = self.nodes[at].bytes as i128;
        let old_files = self.nodes[at].files as i128;
        self.generation += 1;
        let kids: Vec<usize> = s.children.into_iter().map(|c| self.push(at, c)).collect();
        let n = &mut self.nodes[at];
        n.children = kids;
        n.complete = true;
        n.loading = false;
        n.bytes = s.bytes;
        n.files = s.files;
        self.sort(at);
        if let Some(parent) = self.nodes[at].parent {
            self.bump(parent, s.bytes as i128 - old_bytes, s.files as i128 - old_files);
        }
    }

    fn sort(&mut self, at: usize) {
        let mut kids = std::mem::take(&mut self.nodes[at].children);
        kids.sort_by_key(|&k| std::cmp::Reverse(self.nodes[k].bytes));
        self.nodes[at].children = kids;
    }

    fn bump(&mut self, mut at: usize, bytes: i128, files: i128) {
        loop {
            let n = &mut self.nodes[at];
            n.bytes = (n.bytes as i128 + bytes).max(0) as u64;
            n.files = (n.files as i128 + files).max(0) as u64;
            match n.parent {
                Some(p) => {
                    self.sort(p);
                    at = p;
                }
                None => break,
            }
        }
    }

    /// Unlink a node (after it was sent to the trash) and subtract its size.
    pub fn remove(&mut self, id: usize) {
        let Some(parent) = self.nodes[id].parent else { return };
        self.generation += 1;
        self.nodes[parent].children.retain(|&k| k != id);
        let (b, f) = (self.nodes[id].bytes as i128, self.nodes[id].files as i128);
        self.bump(parent, -b, -f);
        self.nodes[id].parent = None;
    }

    /// Root → `id` chain.
    pub fn chain(&self, id: usize) -> Vec<usize> {
        let mut v = vec![id];
        let mut cur = id;
        while let Some(p) = self.nodes[cur].parent {
            v.push(p);
            cur = p;
        }
        v.reverse();
        v
    }

    /// Is `id` inside (or equal to) `ancestor`?
    pub fn is_under(&self, id: usize, ancestor: usize) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.nodes[c].parent;
        }
        false
    }

    /// The `n` largest files below `at`.
    pub fn largest_files(&self, at: usize, n: usize) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        let mut stack = vec![at];
        while let Some(i) = stack.pop() {
            let node = &self.nodes[i];
            if !node.is_dir && !node.other && i != at {
                out.push(i);
                continue;
            }
            stack.extend(node.children.iter().copied());
        }
        out.sort_by_key(|&k| std::cmp::Reverse(self.nodes[k].bytes));
        out.truncate(n);
        out
    }

    pub fn find(&self, path: &Path) -> Option<usize> {
        // Removed nodes stay in the arena but are unlinked (no parent).
        (0..self.nodes.len()).find(|&i| self.nodes[i].path == path && (i == 0 || self.nodes[i].parent.is_some()))
    }
}

/// Worker → window messages.
pub enum ScanMsg {
    /// Files directly in the root and the number of subfolders to measure.
    Begin {
        files: Vec<SubTree>,
        dirs: usize,
    },
    Child(SubTree),
    Progress {
        done: usize,
        current: PathBuf,
    },
    Done {
        errors: u32,
    },
    Failed(String),
    Subtree {
        path: PathBuf,
        result: Result<SubTree, String>,
    },
}

/// Scan `root` progressively on a worker.
pub fn start_scan(root: PathBuf, tx: Sender<ScanMsg>, ctl: Arc<OpControl>, wake: Arc<dyn Fn() + Send + Sync>) {
    let spawned = std::thread::Builder::new().name("soyuz-treemap".into()).spawn(move || {
        let rd = match std::fs::read_dir(&root) {
            Ok(rd) => rd,
            Err(e) => {
                let _ = tx.send(ScanMsg::Failed(e.to_string()));
                wake();
                return;
            }
        };
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for ent in rd.flatten() {
            let Ok(ft) = ent.file_type() else { continue };
            if ft.is_dir() {
                dirs.push(ent.path());
            } else if ft.is_file() {
                let size = ent.metadata().map(|m| m.len()).unwrap_or(0);
                files.push(SubTree::file(ent.path(), size));
            }
        }
        dirs.sort();
        let _ = tx.send(ScanMsg::Begin { files, dirs: dirs.len() });
        wake();
        let mut errors = 0u32;
        for (i, d) in dirs.into_iter().enumerate() {
            if ctl.is_cancelled() {
                return;
            }
            let _ = tx.send(ScanMsg::Progress { done: i, current: d.clone() });
            match soyuz_search::size_tree(&d, DEPTH, &ctl) {
                Ok(t) => {
                    if tx.send(ScanMsg::Child(convert(t))).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind == soyuz_core::FsErrorKind::Interrupted => return,
                Err(_) => {
                    errors += 1;
                    // Unreadable folder: show it as an empty tile so it is not silently lost.
                    let mut s = SubTree::file(d, 0);
                    s.is_dir = true;
                    s.files = 0;
                    let _ = tx.send(ScanMsg::Child(s));
                }
            }
            wake();
        }
        let _ = tx.send(ScanMsg::Done { errors });
        wake();
    });
    if spawned.is_err() {
        log::warn!("cannot spawn treemap scan");
    }
}

/// Measure one folder deeper (drill-in beyond the initial depth).
pub fn scan_subtree(path: PathBuf, tx: Sender<ScanMsg>, ctl: Arc<OpControl>, wake: Arc<dyn Fn() + Send + Sync>) {
    let _ = std::thread::Builder::new().name("soyuz-treemap-sub".into()).spawn(move || {
        let result = soyuz_search::size_tree(&path, DEPTH, &ctl).map(convert).map_err(|e| e.to_string());
        if !ctl.is_cancelled() {
            let _ = tx.send(ScanMsg::Subtree { path, result });
        }
        wake();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(name: &str, bytes: u64) -> SubTree {
        SubTree::file(PathBuf::from(format!("/r/{name}")), bytes)
    }

    #[test]
    fn arena_totals_sorting_and_removal() {
        let mut t = Tree::new(Path::new("/r"));
        t.add_child(0, leaf("a.mp4", 100));
        let dir = SubTree {
            name: "d".into(),
            path: "/r/d".into(),
            bytes: 300,
            files: 2,
            is_dir: true,
            other: false,
            complete: true,
            children: vec![leaf("d/x.png", 200), leaf("d/y.rs", 100)],
        };
        let d = t.add_child(0, dir);
        assert_eq!(t.nodes[0].bytes, 400);
        assert_eq!(t.generation, 2, "every change bumps the generation");
        assert_eq!(t.nodes[0].files, 3);
        assert_eq!(t.nodes[0].children[0], d, "children sorted by size");
        assert_eq!(t.nodes[t.nodes[d].children[0]].class, FileClass::Image);
        let largest = t.largest_files(0, 2);
        assert_eq!(largest.iter().map(|&i| t.nodes[i].bytes).collect::<Vec<_>>(), vec![200, 100]);
        let x = t.nodes[d].children[0];
        assert_eq!(t.chain(x), vec![0, d, x]);
        assert!(t.is_under(x, d));
        t.remove(x);
        assert_eq!(t.nodes[d].bytes, 100);
        assert_eq!(t.nodes[0].bytes, 200);
        assert_eq!(t.nodes[0].files, 2);
        // Children now tie at 100; the order stays stable and valid.
        assert_eq!(t.nodes[0].children.len(), 2);
    }

    #[test]
    fn replace_subtree_on_drill_in() {
        let mut t = Tree::new(Path::new("/r"));
        let mut shallow = leaf("deep", 50);
        shallow.is_dir = true;
        shallow.complete = false;
        shallow.files = 5;
        let id = t.add_child(0, shallow);
        let mut full = leaf("deep", 60);
        full.is_dir = true;
        full.files = 6;
        full.children = vec![leaf("deep/a", 40), leaf("deep/b", 20)];
        t.replace(id, full);
        assert!(t.nodes[id].complete);
        assert_eq!(t.nodes[id].children.len(), 2);
        assert_eq!(t.nodes[0].bytes, 60, "parent totals follow the rescan");
    }
}
