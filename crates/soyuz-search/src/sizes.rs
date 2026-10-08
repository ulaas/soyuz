//! Folder sizes with streaming partials, and the size tree for the treemap.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use soyuz_core::{FsResult, OpControl};

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

/// Compute folder sizes in the background; cancel through the returned control.
pub fn folder_sizes(
    paths: Vec<PathBuf>,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> (crossbeam_channel::Receiver<SizeEvent>, Arc<OpControl>) {
    let _ = (paths, wake);
    todo!("M6: soyuz_search::folder_sizes")
}

/// Depth-limited tree for the treemap; children sorted by bytes desc, small ones folded
/// into an "other" node.
#[derive(Clone, Debug)]
pub struct TreeNode {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    pub children: Vec<TreeNode>,
}

/// Blocking; callers (app) run it on a worker thread and poll `ctl`.
pub fn size_tree(root: &Path, max_depth: usize, ctl: &OpControl) -> FsResult<TreeNode> {
    let _ = (root, max_depth, ctl);
    todo!("M12: soyuz_search::size_tree")
}
