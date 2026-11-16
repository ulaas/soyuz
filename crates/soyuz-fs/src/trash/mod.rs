//! Trash / recycle bin, implemented by hand per OS:
//! freedesktop trash spec 1.0 on Linux/BSD, `~/.Trash` + own index on macOS, `$Recycle.Bin`
//! (`$I`/`$R` pairs) on Windows. Volumes without a usable bin report
//! [`TrashSupport::Unsupported`] and `trash()` fails with `Unsupported` — an item is never
//! silently deleted permanently.

pub(crate) mod freedesktop;
mod index;
mod macos;
pub(crate) mod windows;

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

/// Test builds (cargo feature `sandbox-trash`, enabled by the workspace crates'
/// dev-dependencies): every trash operation uses a private bin under the temp directory
/// instead of the user's trash, so `cargo test` never moves files into `~/.Trash`, never
/// writes the real restore index and never lists or purges the user's trashed items.
/// `SOYUZ_TRASH_TESTS=1` turns the sandbox off for the opt-in real-trash tests.
#[cfg(feature = "sandbox-trash")]
fn sandbox() -> Option<&'static macos::MacBin> {
    static BIN: std::sync::OnceLock<Option<macos::MacBin>> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        if std::env::var("SOYUZ_TRASH_TESTS").is_ok_and(|v| v == "1") {
            return None;
        }
        let dir = std::env::temp_dir().join(format!("soyuz-test-trash-{}", std::process::id()));
        Some(macos::MacBin { root: dir.join("bin"), index: dir.join("trash-index.json") })
    })
    .as_ref()
}

#[cfg(not(feature = "sandbox-trash"))]
#[inline(always)]
fn sandbox() -> Option<&'static macos::MacBin> {
    None
}

/// The sandbox bin holding `item`, if the sandbox is active.
fn sandbox_for(item: &TrashItem) -> FsResult<Option<&'static macos::MacBin>> {
    match sandbox() {
        Some(b) if b.root == item.trash_root => Ok(Some(b)),
        Some(_) => Err(not_in_bin(item)),
        None => Ok(None),
    }
}

/// Whether `path` can be trashed (e.g. FAT volume on Windows → `Unsupported`).
pub fn trash_support(path: &Path) -> TrashSupport {
    if !crate::list::exists_nofollow(path) {
        return TrashSupport::Unsupported("the item does not exist".into());
    }
    if sandbox().is_some() {
        return TrashSupport::Native;
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
    match sandbox() {
        Some(b) => b.trash(&abs),
        None => imp::trash(&abs),
    }
}

/// Items of all bins (home + per-volume).
pub fn list() -> FsResult<Vec<TrashItem>> {
    let mut items = match sandbox() {
        Some(b) => b.list(),
        None => imp::list()?,
    };
    items.sort_by_key(|i| std::cmp::Reverse(i.deleted_at));
    Ok(items)
}

/// Restore to the original location (or `to`); returns the restored path. An existing
/// destination is `AlreadyExists` (the caller asks the user).
pub fn restore(item: &TrashItem, to: Option<&Path>) -> FsResult<PathBuf> {
    match sandbox_for(item)? {
        Some(b) => b.restore(item, to),
        None => imp::restore(item, to),
    }
}

/// Permanently delete one trashed item.
pub fn purge(item: &TrashItem) -> FsResult<()> {
    match sandbox_for(item)? {
        Some(b) => b.purge(item),
        None => imp::purge(item),
    }
}

/// Permanently delete everything in all bins.
pub fn empty(ctl: &OpControl, on_item: &mut dyn FnMut(&TrashItem, FsResult<()>) -> Control) -> FsResult<()> {
    ctl.checkpoint()?;
    match sandbox() {
        Some(b) => macos::empty_all(std::slice::from_ref(b), ctl, on_item),
        None => imp::empty(ctl, on_item),
    }
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

    /// Test builds never reach the user's trash: items go to the per-process sandbox bin
    /// and come back out of it.
    #[cfg(feature = "sandbox-trash")]
    #[test]
    fn test_builds_use_the_sandbox_bin() {
        if sandbox().is_none() {
            return; // SOYUZ_TRASH_TESTS=1: the real-trash tests are opted in
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sandboxed.txt");
        std::fs::write(&f, b"x").unwrap();
        let item = trash(&f).unwrap();
        assert!(item.trash_root.starts_with(std::env::temp_dir()), "{}", item.trash_root.display());
        assert!(!item.trash_root.starts_with(crate::path::home().join(".Trash")));
        assert!(list().unwrap().iter().any(|i| i.id == item.id && i.original == f));
        assert_eq!(restore(&item, None).unwrap(), f);
        assert_eq!(std::fs::read(&f).unwrap(), b"x");
    }

    #[test]
    fn missing_items() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("nope");
        assert!(matches!(trash_support(&p), TrashSupport::Unsupported(_)));
        assert_eq!(trash(&p).unwrap_err().kind, FsErrorKind::NotFound);
    }
}
