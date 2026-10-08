//! Trash / recycle bin, implemented by hand per OS (M3):
//! freedesktop trash spec 1.0 on Linux, `~/.Trash` + own index on macOS, `$Recycle.Bin`
//! (`$I`/`$R` pairs) on Windows. M0 baseline: every operation returns `Unsupported`.

mod freedesktop;
mod index;
mod macos;
mod windows;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use soyuz_core::time::serde_rfc3339;
use soyuz_core::{Control, EntryKind, FsResult, OpControl};

use crate::baseline;

/// One item in a trash bin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrashItem {
    /// Bin-specific id (`$R…` name, `.trashinfo` stem, index key).
    pub id: String,
    pub original: PathBuf,
    #[serde(with = "serde_rfc3339")]
    pub deleted_at: SystemTime,
    pub size: u64,
    pub kind: EntryKind,
    /// The bin directory holding the item.
    pub trash_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrashSupport {
    Native,
    /// Human-readable reason (e.g. "FAT volume has no recycle bin").
    Unsupported(String),
}

/// Whether `path` can be trashed (e.g. FAT volume on Windows → `Unsupported`).
pub fn trash_support(path: &Path) -> TrashSupport {
    baseline::trash_support(path)
}

/// Move `path` to its volume's trash.
pub fn trash(path: &Path) -> FsResult<TrashItem> {
    baseline::trash(path)
}

/// Items of all bins (home + per-volume).
pub fn list() -> FsResult<Vec<TrashItem>> {
    baseline::trash_list()
}

/// Restore to the original location (or `to`); returns the restored path.
pub fn restore(item: &TrashItem, to: Option<&Path>) -> FsResult<PathBuf> {
    baseline::trash_restore(item, to)
}

/// Permanently delete one trashed item.
pub fn purge(item: &TrashItem) -> FsResult<()> {
    baseline::trash_purge(item)
}

/// Permanently delete everything in all bins.
pub fn empty(ctl: &OpControl, on_item: &mut dyn FnMut(&TrashItem, FsResult<()>) -> Control) -> FsResult<()> {
    baseline::trash_empty(ctl, on_item)
}
