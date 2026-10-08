//! Permanent deletion. Links are never followed.

use std::path::Path;

use serde::{Deserialize, Serialize};
use soyuz_core::{Control, FsResult, OpControl, ProgressSink};

use crate::baseline;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteOptions {
    /// Clear the read-only attribute and retry when deletion is denied.
    pub clear_readonly: bool,
    /// Windows: POSIX delete semantics (delete files with open handles).
    pub posix_semantics: bool,
}

impl Default for DeleteOptions {
    fn default() -> Self {
        DeleteOptions { clear_readonly: false, posix_semantics: true }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DeleteStats {
    pub files: u64,
    pub dirs: u64,
    pub links: u64,
    pub failed: u64,
}

pub fn delete_file(path: &Path, o: &DeleteOptions) -> FsResult<()> {
    baseline::delete_file(path, o)
}

pub fn delete_empty_dir(path: &Path) -> FsResult<()> {
    baseline::delete_empty_dir(path)
}

/// Remove a symlink / junction itself (never follows).
pub fn delete_link(path: &Path) -> FsResult<()> {
    baseline::delete_link(path)
}

/// Post-order removal of a tree; `on_item` sees every attempted item and may stop.
pub fn remove_tree(
    root: &Path,
    o: &DeleteOptions,
    ctl: &OpControl,
    on_item: &mut dyn FnMut(&Path, FsResult<()>) -> Control,
) -> FsResult<DeleteStats> {
    baseline::remove_tree(root, o, ctl, on_item)
}

/// Overwrite the file `passes` times (random, last pass zeros), then delete it.
/// Meaningless on SSD / copy-on-write filesystems (the UI says so).
pub fn secure_wipe(path: &Path, passes: u8, ctl: &OpControl, sink: &mut dyn ProgressSink) -> FsResult<()> {
    baseline::secure_wipe(path, passes, ctl, sink)
}
