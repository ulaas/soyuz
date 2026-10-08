//! Trash / recycle bin, implemented by hand per OS:
//! freedesktop trash spec 1.0 on Linux/BSD, `~/.Trash` + own index on macOS, `$Recycle.Bin`
//! (`$I`/`$R` pairs) on Windows. Volumes without a usable bin report
//! [`TrashSupport::Unsupported`] and `trash()` fails with `Unsupported` — an item is never
//! silently deleted permanently.

mod freedesktop;
mod index;
mod macos;
mod windows;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use soyuz_core::time::serde_rfc3339;
use soyuz_core::{Control, EntryInfo, EntryKind, FsError, FsErrorKind, FsResult, OpControl};

/// One item in a trash bin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrashItem {
    /// Bin-specific id (`$R…` name, `.trashinfo` stem, file name in `~/.Trash`).
    pub id: String,
    /// Where the item came from (empty when unknown, e.g. trashed by another app on macOS).
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
    /// Human-readable reason (e.g. "FAT32 volumes have no recycle bin").
    Unsupported(String),
}

/// Whether `path` can be trashed (e.g. FAT volume on Windows → `Unsupported`).
pub fn trash_support(path: &Path) -> TrashSupport {
    if !crate::list::exists_nofollow(path) {
        return TrashSupport::Unsupported("the item does not exist".into());
    }
    imp::support(path)
}

/// Move `path` to its volume's trash.
pub fn trash(path: &Path) -> FsResult<TrashItem> {
    if !crate::list::exists_nofollow(path) {
        return Err(FsError::new(FsErrorKind::NotFound, "trash", Some(path)));
    }
    let abs = crate::path::strip_verbatim(&crate::path::normalize(path));
    if crate::path::is_root(&abs) {
        return Err(FsError::with_message(
            FsErrorKind::InvalidName,
            "trash",
            Some(path),
            "cannot trash a root directory",
        ));
    }
    imp::trash(&abs)
}

/// Items of all bins (home + per-volume).
pub fn list() -> FsResult<Vec<TrashItem>> {
    let mut items = imp::list()?;
    items.sort_by_key(|i| std::cmp::Reverse(i.deleted_at));
    Ok(items)
}

/// Restore to the original location (or `to`); returns the restored path. An existing
/// destination is `AlreadyExists` (the caller asks the user).
pub fn restore(item: &TrashItem, to: Option<&Path>) -> FsResult<PathBuf> {
    imp::restore(item, to)
}

/// Permanently delete one trashed item.
pub fn purge(item: &TrashItem) -> FsResult<()> {
    imp::purge(item)
}

/// Permanently delete everything in all bins.
pub fn empty(ctl: &OpControl, on_item: &mut dyn FnMut(&TrashItem, FsResult<()>) -> Control) -> FsResult<()> {
    ctl.checkpoint()?;
    imp::empty(ctl, on_item)
}

/// Item ids are single path components: never `..`, separators or empty.
pub(crate) fn safe_id(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', '\0']) && !(cfg!(windows) && id.contains(':'))
}

/// Size of a file, or the total size of a directory tree (errors ignored).
pub(crate) fn tree_size(path: &Path, e: &EntryInfo) -> u64 {
    if e.kind != EntryKind::Dir {
        return e.size;
    }
    let ctl = OpControl::new();
    crate::list::walk(path, &crate::list::WalkOptions::default(), &ctl, &mut |_| Control::Continue)
        .map(|s| s.bytes)
        .unwrap_or(0)
}

/// Permanently remove a file, link or directory tree (read-only items included).
pub(crate) fn remove_any(path: &Path) -> FsResult<()> {
    let e = crate::list::stat(path, false)?;
    let o = crate::delete::DeleteOptions { clear_readonly: true, posix_semantics: true };
    match e.kind {
        EntryKind::Dir => {
            let ctl = OpControl::new();
            let mut first_err = None;
            crate::delete::remove_tree(path, &o, &ctl, &mut |_, r| {
                if let Err(e) = r
                    && first_err.is_none()
                {
                    first_err = Some(e);
                }
                Control::Continue
            })?;
            first_err.map_or(Ok(()), Err)
        }
        EntryKind::Symlink { .. } | EntryKind::Junction => crate::delete::delete_link(path),
        _ => crate::delete::delete_file(path, &o),
    }
}

/// Percent-decoding shared with `.desktop` `file://` URLs.
pub(crate) fn percent_decode_pub(s: &str) -> Vec<u8> {
    freedesktop::percent_decode(s)
}

#[cfg_attr(windows, allow(dead_code))]
fn not_in_bin(item: &TrashItem) -> FsError {
    FsError::with_message(FsErrorKind::NotFound, "trash", Some(&item.trash_root), "unknown trash directory")
}

#[cfg(windows)]
mod imp {
    pub(super) use super::windows::{empty, list, purge, restore, support, trash};
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::{Control, FsResult, OpControl};

    use super::macos::{discover, empty_all};
    use super::{TrashItem, TrashSupport};

    pub(super) fn support(path: &Path) -> TrashSupport {
        discover::support(path)
    }
    pub(super) fn trash(path: &Path) -> FsResult<TrashItem> {
        discover::bin_for(path)?.trash(path)
    }
    pub(super) fn list() -> FsResult<Vec<TrashItem>> {
        Ok(discover::all_bins().iter().flat_map(|b| b.list()).collect())
    }
    pub(super) fn restore(item: &TrashItem, to: Option<&Path>) -> FsResult<PathBuf> {
        discover::bin_of(&item.trash_root).ok_or_else(|| super::not_in_bin(item))?.restore(item, to)
    }
    pub(super) fn purge(item: &TrashItem) -> FsResult<()> {
        discover::bin_of(&item.trash_root).ok_or_else(|| super::not_in_bin(item))?.purge(item)
    }
    pub(super) fn empty(ctl: &OpControl, on_item: &mut dyn FnMut(&TrashItem, FsResult<()>) -> Control) -> FsResult<()> {
        empty_all(&discover::all_bins(), ctl, on_item)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::{Control, FsResult, OpControl};

    use super::freedesktop::discover;
    use super::{TrashItem, TrashSupport};

    pub(super) fn support(path: &Path) -> TrashSupport {
        discover::support(path)
    }
    pub(super) fn trash(path: &Path) -> FsResult<TrashItem> {
        discover::bin_for(path)?.trash(path)
    }
    pub(super) fn list() -> FsResult<Vec<TrashItem>> {
        Ok(discover::all_bins().iter().flat_map(|b| b.list()).collect())
    }
    pub(super) fn restore(item: &TrashItem, to: Option<&Path>) -> FsResult<PathBuf> {
        discover::bin_of(&item.trash_root).ok_or_else(|| super::not_in_bin(item))?.restore(item, to)
    }
    pub(super) fn purge(item: &TrashItem) -> FsResult<()> {
        discover::bin_of(&item.trash_root).ok_or_else(|| super::not_in_bin(item))?.purge(item)
    }
    pub(super) fn empty(ctl: &OpControl, on_item: &mut dyn FnMut(&TrashItem, FsResult<()>) -> Control) -> FsResult<()> {
        for bin in discover::all_bins() {
            for item in bin.list() {
                ctl.checkpoint()?;
                let res = bin.purge(&item);
                if on_item(&item, res) == Control::Stop {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::{Control, FsError, FsResult, OpControl};

    use super::{TrashItem, TrashSupport};

    pub(super) fn support(_path: &Path) -> TrashSupport {
        TrashSupport::Unsupported("no trash on this platform".into())
    }
    pub(super) fn trash(path: &Path) -> FsResult<TrashItem> {
        Err(FsError::unsupported("trash", Some(path)))
    }
    pub(super) fn list() -> FsResult<Vec<TrashItem>> {
        Ok(Vec::new())
    }
    pub(super) fn restore(item: &TrashItem, _to: Option<&Path>) -> FsResult<PathBuf> {
        Err(FsError::unsupported("restore", Some(&item.original)))
    }
    pub(super) fn purge(item: &TrashItem) -> FsResult<()> {
        Err(FsError::unsupported("purge", Some(&item.original)))
    }
    pub(super) fn empty(_ctl: &OpControl, _f: &mut dyn FnMut(&TrashItem, FsResult<()>) -> Control) -> FsResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_single_components() {
        assert!(safe_id("$RABC123.txt"));
        assert!(safe_id("note.1.txt"));
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "a\0b"] {
            assert!(!safe_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn missing_items() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("nope");
        assert!(matches!(trash_support(&p), TrashSupport::Unsupported(_)));
        assert_eq!(trash(&p).unwrap_err().kind, FsErrorKind::NotFound);
    }
}
