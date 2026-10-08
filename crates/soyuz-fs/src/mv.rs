//! Rename / move.
//!
//! * Windows: `MoveFileExW` (never `MOVEFILE_COPY_ALLOWED`: cross-volume moves are reported
//!   as `CrossDevice` and the caller copies + deletes with progress).
//! * Linux: `renameat2(RENAME_NOREPLACE / RENAME_EXCHANGE)`; filesystems without
//!   `RENAME_NOREPLACE` fall back to `link` + `unlink` (atomic no-replace for files).
//! * macOS: `renameatx_np(RENAME_EXCL / RENAME_SWAP)`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use soyuz_core::{FsError, FsErrorKind, FsResult};

use crate::list::exists_nofollow;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RenameMode {
    /// Fail with `AlreadyExists` if the destination exists.
    #[default]
    NoReplace,
    Replace,
    /// Atomically swap (Linux `RENAME_EXCHANGE`, macOS `RENAME_SWAP`); `Unsupported` elsewhere.
    Exchange,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MoveOptions {
    pub mode: RenameMode,
    /// Test hook: always report `NeedsCopy`.
    pub force_cross_device: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveFast {
    Renamed,
    NeedsCopy,
}

/// Same-device rename; `CrossDevice` error otherwise.
pub fn rename(src: &Path, dst: &Path, mode: RenameMode) -> FsResult<()> {
    match imp::rename(src, dst, mode) {
        Err(e)
            if e.kind == FsErrorKind::AlreadyExists && mode == RenameMode::NoReplace && is_case_variant(src, dst) =>
        {
            // `a` → `A` on a case-insensitive volume: the "existing" destination is the source.
            imp::rename(src, dst, RenameMode::Replace)
        }
        other => other,
    }
}

/// `src` and `dst` name the same entry through a case-only difference on a
/// case-insensitive volume: same parent, names equal ignoring case, the same object
/// *without following links* (a symlink `a` → `A` is not a case variant of `A`), and the
/// directory has no entry spelled exactly like `dst` (on a case-sensitive volume a
/// hardlink `A` of `a` is a separate name, not a case variant).
pub fn is_case_variant(src: &Path, dst: &Path) -> bool {
    let (Some(sn), Some(dn)) = (src.file_name(), dst.file_name()) else { return false };
    if src.parent() != dst.parent()
        || sn == dn
        || sn.to_string_lossy().to_lowercase() != dn.to_string_lossy().to_lowercase()
    {
        return false;
    }
    let same = match (crate::copy::file_identity(src, false), crate::copy::file_identity(dst, false)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        return false;
    }
    let parent = match src.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    match std::fs::read_dir(parent) {
        Ok(rd) => !rd.flatten().any(|e| e.file_name() == dn),
        Err(_) => false,
    }
}

/// Case-only rename that is safe on case-insensitive volumes (two-step on FAT/exFAT).
pub fn rename_case_only(src: &Path, new_name: &str) -> FsResult<()> {
    let parent = src.parent().unwrap_or(Path::new(""));
    let dst = parent.join(new_name);
    let old_name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if old_name == new_name {
        return Ok(());
    }
    if old_name.to_lowercase() != new_name.to_lowercase() {
        return rename(src, &dst, RenameMode::NoReplace);
    }
    if !imp::needs_two_step(src) {
        match rename(src, &dst, RenameMode::NoReplace) {
            Ok(()) if imp::on_disk_name_is(&dst, new_name) => return Ok(()),
            Ok(()) => {}
            Err(e) if e.kind != FsErrorKind::AlreadyExists => return Err(e),
            Err(_) => {}
        }
    }
    // Two-step through a temporary name works on every filesystem.
    let current = imp::actual_path(&dst).unwrap_or_else(|| src.to_path_buf());
    let tmp = parent.join(format!(".soyuz-rename-{}-{}", std::process::id(), nanos()));
    imp::rename(&current, &tmp, RenameMode::NoReplace)?;
    imp::rename(&tmp, &dst, RenameMode::NoReplace).inspect_err(|_| {
        let _ = imp::rename(&tmp, &current, RenameMode::NoReplace);
    })
}

/// Try a rename; `Ok(NeedsCopy)` when the move crosses devices (caller copies + deletes).
pub fn try_rename_or_crossdev(src: &Path, dst: &Path, o: &MoveOptions) -> FsResult<MoveFast> {
    if o.force_cross_device {
        return Ok(MoveFast::NeedsCopy);
    }
    match rename(src, dst, o.mode) {
        Ok(()) => Ok(MoveFast::Renamed),
        Err(e) if e.kind == FsErrorKind::CrossDevice => Ok(MoveFast::NeedsCopy),
        Err(e) => Err(e),
    }
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

fn missing_src(src: &Path) -> Option<FsError> {
    (!exists_nofollow(src)).then(|| FsError::new(FsErrorKind::NotFound, "rename", Some(src)))
}

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::{FsError, FsErrorKind, FsResult};

    use super::RenameMode;
    use crate::err;
    use crate::sys::windows::{fs as wfs, volume};

    const MOVEFILE_REPLACE_EXISTING: u32 = 1;

    pub(super) fn rename(src: &Path, dst: &Path, mode: RenameMode) -> FsResult<()> {
        let flags = match mode {
            RenameMode::NoReplace => 0,
            RenameMode::Replace => MOVEFILE_REPLACE_EXISTING,
            RenameMode::Exchange => {
                return Err(err::unsupported("rename", src, "atomic exchange is not supported on Windows"));
            }
        };
        wfs::move_file(src, dst, flags).map_err(|e| {
            let code = e.raw_os_error();
            let mut fe = err::io("rename", src, e);
            if matches!(code, Some(80 | 183)) {
                fe.path = Some(dst.to_path_buf());
            } else if code == Some(17) {
                fe.message = "source and destination are on different volumes".into();
            } else if code == Some(5)
                && super::missing_src(src).is_none()
                && mode == RenameMode::Replace
                && dst.is_dir()
            {
                fe = FsError::with_message(
                    FsErrorKind::AlreadyExists,
                    "rename",
                    Some(dst),
                    "cannot replace a directory",
                );
            }
            fe
        })
    }

    pub(super) fn needs_two_step(src: &Path) -> bool {
        let Ok(root) = volume::volume_path_name(src) else { return false };
        volume::volume_info(&root).is_ok_and(|v| {
            let fs = v.fs.to_ascii_uppercase();
            fs.starts_with("FAT") || fs == "EXFAT"
        })
    }

    pub(super) fn on_disk_name_is(path: &Path, name: &str) -> bool {
        actual_path(path).is_some_and(|p| p.file_name().is_some_and(|n| n == name))
    }

    /// The path with the final component spelled as stored on disk.
    pub(super) fn actual_path(path: &Path) -> Option<PathBuf> {
        let parent = path.parent()?;
        let want = path.file_name()?.to_string_lossy().to_lowercase();
        for e in wfs::read_dir(parent).ok()?.flatten() {
            if e.name.to_string_lossy().to_lowercase() == want {
                return Some(parent.join(e.name));
            }
        }
        None
    }
}

#[cfg(unix)]
mod imp {
    use std::path::{Path, PathBuf};

    use rustix::fs::{AtFlags, CWD, RenameFlags};
    use rustix::io::Errno;
    use soyuz_core::FsResult;

    use super::RenameMode;
    use crate::err;

    fn map(src: &Path, dst: &Path, e: Errno) -> soyuz_core::FsError {
        let mut fe = err::io("rename", src, e.into());
        if e == Errno::EXIST || e == Errno::NOTEMPTY {
            fe.path = Some(dst.to_path_buf());
            fe.kind = soyuz_core::FsErrorKind::AlreadyExists;
        } else if e == Errno::XDEV {
            fe.message = "source and destination are on different filesystems".into();
        }
        fe
    }

    pub(super) fn rename(src: &Path, dst: &Path, mode: RenameMode) -> FsResult<()> {
        match mode {
            RenameMode::Replace => rustix::fs::renameat(CWD, src, CWD, dst).map_err(|e| map(src, dst, e)),
            RenameMode::Exchange => {
                rustix::fs::renameat_with(CWD, src, CWD, dst, RenameFlags::EXCHANGE).map_err(|e| map(src, dst, e))
            }
            RenameMode::NoReplace => match rustix::fs::renameat_with(CWD, src, CWD, dst, RenameFlags::NOREPLACE) {
                Ok(()) => Ok(()),
                Err(Errno::INVAL | Errno::NOSYS | Errno::OPNOTSUPP) => fallback_noreplace(src, dst),
                Err(e) => Err(map(src, dst, e)),
            },
        }
    }

    /// For filesystems without RENAME_NOREPLACE: `link`+`unlink` is atomic for files;
    /// directories (and filesystems without hard links) use check-then-rename.
    fn fallback_noreplace(src: &Path, dst: &Path) -> FsResult<()> {
        if let Some(e) = super::missing_src(src) {
            return Err(e);
        }
        let is_dir = crate::list::stat(src, false).is_ok_and(|e| e.kind == soyuz_core::EntryKind::Dir);
        if !is_dir {
            match rustix::fs::linkat(CWD, src, CWD, dst, AtFlags::empty()) {
                Ok(()) => {
                    return rustix::fs::unlinkat(CWD, src, AtFlags::empty()).map_err(|e| map(src, dst, e));
                }
                Err(Errno::EXIST) => return Err(map(src, dst, Errno::EXIST)),
                Err(_) => {}
            }
        }
        if crate::list::exists_nofollow(dst) {
            return Err(map(src, dst, Errno::EXIST));
        }
        rustix::fs::renameat(CWD, src, CWD, dst).map_err(|e| map(src, dst, e))
    }

    pub(super) fn needs_two_step(_src: &Path) -> bool {
        false
    }

    pub(super) fn on_disk_name_is(path: &Path, name: &str) -> bool {
        actual_path(path).is_some_and(|p| p.file_name().is_some_and(|n| n == name))
    }

    pub(super) fn actual_path(path: &Path) -> Option<PathBuf> {
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let want = path.file_name()?;
        let rd = std::fs::read_dir(parent).ok()?;
        let mut ci = None;
        for e in rd.flatten() {
            let n = e.file_name();
            if n == want {
                return Some(path.to_path_buf());
            }
            if n.to_string_lossy().to_lowercase() == want.to_string_lossy().to_lowercase() {
                ci = Some(path.with_file_name(n));
            }
        }
        ci
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::FsResult;

    use super::RenameMode;
    use crate::err;

    pub(super) fn rename(src: &Path, dst: &Path, _mode: RenameMode) -> FsResult<()> {
        std::fs::rename(src, dst).map_err(err::map("rename", src))
    }
    pub(super) fn needs_two_step(_src: &Path) -> bool {
        true
    }
    pub(super) fn on_disk_name_is(_path: &Path, _name: &str) -> bool {
        true
    }
    pub(super) fn actual_path(_path: &Path) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn symlink_to_case_variant_is_not_replaced() {
        // Case-sensitive volume: `a` is a symlink to `A`. A no-replace rename `a` -> `A`
        // must fail instead of replacing the real file with the link.
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("A");
        std::fs::write(&real, b"precious").unwrap();
        let link = d.path().join("a");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        if std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink()
            && std::fs::symlink_metadata(&real).unwrap().file_type().is_file()
        {
            assert!(!is_case_variant(&link, &real));
            let e = rename(&link, &real, RenameMode::NoReplace).unwrap_err();
            assert_eq!(e.kind, FsErrorKind::AlreadyExists);
            assert_eq!(std::fs::read(&real).unwrap(), b"precious");
            assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        }
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_with_case_variant_name_is_not_a_case_variant() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("x");
        std::fs::write(&a, b"1").unwrap();
        let b = d.path().join("X");
        if std::fs::hard_link(&a, &b).is_ok() {
            assert!(!is_case_variant(&a, &b), "both spellings exist: case-sensitive volume");
        }
    }

    #[cfg(windows)]
    #[test]
    fn case_variant_on_case_insensitive_volume() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("name.txt");
        std::fs::write(&a, b"1").unwrap();
        assert!(is_case_variant(&a, &d.path().join("NAME.txt")));
        assert!(!is_case_variant(&a, &a));
        let other = d.path().join("Other.txt");
        std::fs::write(&other, b"2").unwrap();
        assert!(!is_case_variant(&a, &other));
        rename(&a, &d.path().join("NAME.txt"), RenameMode::NoReplace).unwrap();
        let names: Vec<_> = std::fs::read_dir(d.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(names.iter().any(|n| n == "NAME.txt"));
    }
}
