//! Folder sizes with streaming partials, and the size tree for the treemap.
//!
//! Sizes are logical (`len`), symlinks are never followed (they count as one file of
//! 0 bytes), and on unix multiply-linked files are counted once per `(dev, ino)`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use crossbeam_channel::RecvTimeoutError;
use ignore::WalkState;
use parking_lot::Mutex;
use rayon::prelude::*;
use soyuz_core::{FsError, FsResult, OpControl};

use crate::walker::{hardlink_id, raw_builder, send_cancellable};

/// Interval between [`SizeEvent::Partial`] events for the path being measured.
pub const PARTIAL_INTERVAL: Duration = Duration::from_millis(200);
/// Name of the synthetic node that holds the folded small children in [`size_tree`]
/// (its path is `<parent>/…`).
pub const OTHER_NODE_NAME: &str = "…";
/// Children smaller than `1 / FOLD_DIVISOR` of their parent are folded into "other".
pub const FOLD_DIVISOR: u64 = 100;
/// At most this many real children per node (the rest is folded).
pub const MAX_CHILDREN: usize = 48;

#[derive(Clone, Debug)]
pub enum SizeEvent {
    /// Every 200 ms per path.
    Partial {
        path: PathBuf,
        bytes: u64,
        files: u64,
        dirs: u64,
    },
    Done {
        path: PathBuf,
        bytes: u64,
        files: u64,
        dirs: u64,
        errors: u32,
    },
}

#[derive(Default)]
struct Counters {
    bytes: AtomicU64,
    files: AtomicU64,
    dirs: AtomicU64,
    errors: AtomicU32,
}

impl Counters {
    fn partial(&self, path: &Path) -> SizeEvent {
        SizeEvent::Partial {
            path: path.to_path_buf(),
            bytes: self.bytes.load(Ordering::Relaxed),
            files: self.files.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
        }
    }

    fn done(&self, path: &Path) -> SizeEvent {
        SizeEvent::Done {
            path: path.to_path_buf(),
            bytes: self.bytes.load(Ordering::Relaxed),
            files: self.files.load(Ordering::Relaxed),
            dirs: self.dirs.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
}

/// Compute folder sizes in the background; cancel through the returned control.
///
/// Paths are measured one after another, each with a parallel walk; every path gets
/// `Partial` events while it is being measured and one final `Done`. `dirs` excludes the
/// root itself. A non-directory path reports its own size as one file. After a cancel no
/// further events are sent and the channel disconnects.
pub fn folder_sizes(
    paths: Vec<PathBuf>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> (crossbeam_channel::Receiver<SizeEvent>, Arc<OpControl>) {
    let (tx, rx) = crossbeam_channel::bounded(256);
    let ctl = OpControl::new();
    let ctl2 = ctl.clone();
    let spawned = std::thread::Builder::new().name("soyuz-sizes".into()).spawn(move || {
        for path in paths {
            if ctl2.is_cancelled() {
                break;
            }
            let counters = measure(&path, &ctl2, &tx, wake.as_ref());
            if ctl2.is_cancelled() {
                break;
            }
            if send_cancellable(&tx, counters.done(&path), &ctl2) {
                wake();
            }
        }
    });
    if let Err(e) = spawned {
        log::error!("cannot spawn folder-size thread: {e}");
    }
    (rx, ctl)
}

fn measure(
    path: &Path,
    ctl: &OpControl,
    tx: &crossbeam_channel::Sender<SizeEvent>,
    wake: &(dyn Fn() + Send + Sync),
) -> Counters {
    let c = Counters::default();
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.is_dir() => {}
        Ok(md) => {
            c.files.store(1, Ordering::Relaxed);
            c.bytes.store(if md.is_file() { md.len() } else { 0 }, Ordering::Relaxed);
            return c;
        }
        Err(_) => {
            c.errors.store(1, Ordering::Relaxed);
            return c;
        }
    }
    let Some(builder) = raw_builder(&[path], 0) else { return c };
    let seen: Mutex<HashSet<(u64, u64)>> = Mutex::default();
    let (done_tx, done_rx) = crossbeam_channel::bounded::<()>(0);
    std::thread::scope(|s| {
        let (c, seen) = (&c, &seen);
        s.spawn(move || {
            let _done = done_tx; // dropped when the walk ends → wakes the loop below
            builder.build_parallel().run(|| {
                Box::new(move |res| {
                    if ctl.checkpoint().is_err() {
                        return WalkState::Quit;
                    }
                    let dent = match res {
                        Ok(d) => d,
                        Err(_) => {
                            c.errors.fetch_add(1, Ordering::Relaxed);
                            return WalkState::Continue;
                        }
                    };
                    if dent.depth() == 0 {
                        return WalkState::Continue;
                    }
                    if dent.file_type().is_some_and(|t| t.is_dir()) {
                        c.dirs.fetch_add(1, Ordering::Relaxed);
                        return WalkState::Continue;
                    }
                    match dent.metadata() {
                        Ok(md) if md.is_dir() => {
                            c.dirs.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(md) => {
                            if let Some(id) = hardlink_id(&md)
                                && !seen.lock().insert(id)
                            {
                                return WalkState::Continue;
                            }
                            c.files.fetch_add(1, Ordering::Relaxed);
                            if md.is_file() {
                                c.bytes.fetch_add(md.len(), Ordering::Relaxed);
                            }
                        }
                        Err(_) => {
                            c.errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    WalkState::Continue
                })
            });
        });
        while let Err(RecvTimeoutError::Timeout) = done_rx.recv_timeout(PARTIAL_INTERVAL) {
            if !ctl.is_cancelled() && tx.try_send(c.partial(path)).is_ok() {
                wake();
            }
        }
    });
    c
}

/// Depth-limited tree for the treemap; children sorted by bytes desc, small ones folded
/// into an "other" node.
///
/// Children include files as well as folders (so big files show up as tiles). A child
/// smaller than `parent.bytes / FOLD_DIVISOR`, or beyond [`MAX_CHILDREN`], is folded
/// into one node named [`OTHER_NODE_NAME`] (only when that folds at least two nodes).
#[derive(Clone, Debug)]
pub struct TreeNode {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    /// The synthetic "other" node.
    pub fn is_other(&self) -> bool {
        self.path.file_name().is_some_and(|n| n == OTHER_NODE_NAME)
    }
}

/// Blocking; callers (app) run it on a worker thread and poll `ctl`.
///
/// Totals always cover the whole tree; only nodes down to `max_depth` (root = 0) carry
/// children. Unreadable subfolders are skipped; an unreadable root is an error; a cancel
/// returns `Err(Interrupted)`.
pub fn size_tree(root: &Path, max_depth: usize, ctl: &OpControl) -> FsResult<TreeNode> {
    let md = std::fs::symlink_metadata(root).map_err(|e| FsError::from_io("size_tree", Some(root), &e))?;
    if !md.is_dir() {
        let bytes = if md.is_file() { md.len() } else { 0 };
        return Ok(TreeNode { path: root.to_path_buf(), bytes, files: 1, children: Vec::new() });
    }
    let seen = Mutex::default();
    dir_node(root, 0, max_depth, ctl, &seen)
}

fn dir_node(
    path: &Path,
    depth: usize,
    max_depth: usize,
    ctl: &OpControl,
    seen: &Mutex<HashSet<(u64, u64)>>,
) -> FsResult<TreeNode> {
    ctl.checkpoint()?;
    let keep = depth < max_depth;
    let rd = std::fs::read_dir(path).map_err(|e| FsError::from_io("size_tree", Some(path), &e))?;
    let mut node = TreeNode { path: path.to_path_buf(), bytes: 0, files: 0, children: Vec::new() };
    let mut subdirs = Vec::new();
    for (i, ent) in rd.enumerate() {
        if i % 256 == 255 {
            ctl.checkpoint()?;
        }
        let Ok(ent) = ent else { continue };
        let Ok(ft) = ent.file_type() else { continue };
        if ft.is_dir() {
            subdirs.push(ent.path());
            continue;
        }
        let size = match ent.metadata() {
            Ok(md) if md.is_file() => {
                if hardlink_id(&md).is_some_and(|id| !seen.lock().insert(id)) {
                    continue;
                }
                md.len()
            }
            _ => 0,
        };
        node.files += 1;
        node.bytes += size;
        if keep {
            node.children.push(TreeNode { path: ent.path(), bytes: size, files: 1, children: Vec::new() });
        }
    }
    let subs: Vec<FsResult<TreeNode>> =
        subdirs.par_iter().map(|p| dir_node(p, depth + 1, max_depth, ctl, seen)).collect();
    for r in subs {
        match r {
            Ok(n) => {
                node.bytes += n.bytes;
                node.files += n.files;
                if keep {
                    node.children.push(n);
                }
            }
            Err(e) if e.is_cancel() => return Err(e),
            Err(_) => {}
        }
    }
    if keep {
        fold_children(&mut node);
    }
    Ok(node)
}

fn fold_children(node: &mut TreeNode) {
    node.children.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    let threshold = (node.bytes / FOLD_DIVISOR).max(1);
    let cut = node.children.iter().position(|c| c.bytes < threshold).unwrap_or(node.children.len()).min(MAX_CHILDREN);
    if node.children.len() - cut < 2 {
        return;
    }
    let folded = node.children.split_off(cut);
    node.children.push(TreeNode {
        path: node.path.join(OTHER_NODE_NAME),
        bytes: folded.iter().map(|c| c.bytes).sum(),
        files: folded.iter().map(|c| c.files).sum(),
        children: Vec::new(),
    });
    node.children.sort_by_key(|c| std::cmp::Reverse(c.bytes));
}
