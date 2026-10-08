//! Directory listing, `stat` and tree walking.

use std::path::{Path, PathBuf};
use std::time::Duration;

use soyuz_core::{Control, EntryInfo, FsError, FsResult, OpControl};

use crate::baseline;

/// Options for [`read_dir`] / [`read_dir_streaming`].
#[derive(Clone, Debug)]
pub struct ListOptions {
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    /// Resolve the kind of a symlink's target.
    pub stat_links: bool,
    /// Resolve owner/group names (slower).
    pub owners: bool,
    /// Entries per streamed chunk (default 512).
    pub chunk: usize,
}

impl Default for ListOptions {
    fn default() -> Self {
        ListOptions { follow_symlinks: false, include_hidden: true, stat_links: true, owners: false, chunk: 512 }
    }
}

/// Summary of a streamed listing.
#[derive(Clone, Copy, Debug, Default)]
pub struct ListStats {
    pub entries: usize,
    pub elapsed: Duration,
    /// The sink returned `Control::Stop` before the end.
    pub truncated: bool,
}

/// List a directory (all entries at once).
pub fn read_dir(path: &Path, opts: &ListOptions) -> FsResult<Vec<EntryInfo>> {
    baseline::read_dir(path, opts)
}

/// Streams chunks; `sink` returns `Control::Stop` to abort; `ctl.checkpoint()` between
/// chunks. Raw implementations use FindFirstFileExW(LARGE_FETCH) / readdir+statx /
/// getattrlistbulk.
pub fn read_dir_streaming(
    path: &Path,
    opts: &ListOptions,
    ctl: &OpControl,
    sink: &mut dyn FnMut(Vec<EntryInfo>) -> Control,
) -> FsResult<ListStats> {
    baseline::read_dir_streaming(path, opts, ctl, sink)
}

/// Metadata of one path (`follow` = resolve symlinks).
pub fn stat(path: &Path, follow: bool) -> FsResult<EntryInfo> {
    baseline::stat(path, follow)
}

/// True if something (including a dangling symlink) exists at `path`.
pub fn exists_nofollow(path: &Path) -> bool {
    baseline::exists_nofollow(path)
}

/// Options for [`walk`].
#[derive(Clone, Debug, Default)]
pub struct WalkOptions {
    /// Descend into symlinked directories (never by default).
    pub follow_symlinks: bool,
    /// Emit [`WalkItem::DirDone`] after a directory's children.
    pub post_order: bool,
    /// Maximum depth to descend to (root = 0); `None` = unlimited.
    pub max_depth: Option<usize>,
    /// Do not cross into other devices.
    pub same_device: bool,
}

/// One item produced by [`walk`]. The root itself is emitted first, at depth 0.
#[derive(Debug)]
pub enum WalkItem {
    Entry {
        entry: EntryInfo,
        depth: usize,
    },
    /// Post-order notification (only with `post_order`), emitted for every directory
    /// entry after its children (or right after it when it was not descended into).
    DirDone {
        path: PathBuf,
        depth: usize,
    },
    Error {
        path: PathBuf,
        error: FsError,
    },
}

/// Totals of a walk.
#[derive(Clone, Copy, Debug, Default)]
pub struct WalkStats {
    pub files: u64,
    pub dirs: u64,
    pub links: u64,
    pub bytes: u64,
    pub errors: u64,
}

/// Iterative (explicit stack) tree walk used by delete/copy/size.
pub fn walk(
    root: &Path,
    opts: &WalkOptions,
    ctl: &OpControl,
    sink: &mut dyn FnMut(WalkItem) -> Control,
) -> FsResult<WalkStats> {
    baseline::walk(root, opts, ctl, sink)
}
