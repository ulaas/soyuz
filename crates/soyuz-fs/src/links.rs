//! Symbolic links, hard links and junctions.
//!
//! Windows symlinks use `CreateSymbolicLinkW(SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE)`;
//! junctions are built by hand (`REPARSE_DATA_BUFFER` mount-point layout +
//! `FSCTL_SET_REPARSE_POINT`); link targets are read with `FSCTL_GET_REPARSE_POINT`.
//! Unix uses `symlinkat` / `linkat` / `readlinkat`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soyuz_core::{FsError, FsErrorKind, FsResult};

/// Windows needs to know whether a symlink points at a file or a directory.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    File,
    Dir,
}

/// Create a symbolic link at `link` pointing to `target` (stored as given; relative targets
/// stay relative to the link's directory).
pub fn symlink(target: &Path, link: &Path, kind: LinkKind) -> FsResult<()> {
    imp::symlink(target, link, kind)
}

/// Create a hard link `dst` to the existing file `src`.
pub fn hardlink(src: &Path, dst: &Path) -> FsResult<()> {
    if crate::list::stat(src, false).is_ok_and(|e| e.kind == soyuz_core::EntryKind::Dir) {
        return Err(FsError::with_message(
            FsErrorKind::IsDirectory,
            "hardlink",
            Some(src),
            "directories cannot be hard-linked",
        ));
    }
    imp::hardlink(src, dst)
}

/// NTFS junction (directory mount point) at `link` → `target` (made absolute). The link
/// directory must not exist. `Err(Unsupported)` on non-Windows (not cfg-gated).
pub fn junction(target: &Path, link: &Path) -> FsResult<()> {
    imp::junction(target, link)
}

/// The raw link target (may be relative). Works for symlinks, junctions and app
/// execution aliases on Windows.
#[track_caller]
pub fn read_link(path: &Path) -> FsResult<PathBuf> {
    let _guard = crate::guard::check("read_link");
    imp::read_link(path)
}

/// The link target made absolute (relative targets resolved against the link's parent),
/// normalized but not canonicalized.
#[track_caller]
pub fn resolve_link(path: &Path) -> FsResult<PathBuf> {
    let _guard = crate::guard::check("resolve_link");
    let target = crate::path::strip_verbatim(&read_link(path)?);
    let abs = if target.is_absolute() || target.has_root() {
        target
    } else {
        path.parent().unwrap_or(Path::new("")).join(target)
    };
    Ok(crate::path::normalize(&abs))
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use soyuz_core::{FsError, FsErrorKind, FsResult};

    use super::LinkKind;
    use crate::err;
    use crate::sys::reparse::{self, Reparse};
    use crate::sys::windows::fs as wfs;

    pub(super) fn symlink(target: &Path, link: &Path, kind: LinkKind) -> FsResult<()> {
        let raw = target.to_string_lossy();
        // Windows symlinks with forward slashes do not resolve; verbatim prefixes would end
        // up in the print name.
        let t = if target.is_absolute() {
            crate::path::strip_verbatim(target).to_string_lossy().replace('/', "\\")
        } else {
            raw.replace('/', "\\")
        };
        wfs::create_symlink(link, &OsString::from(t), kind == LinkKind::Dir).map_err(err::map("symlink", link))
    }

    pub(super) fn hardlink(src: &Path, dst: &Path) -> FsResult<()> {
        wfs::create_hardlink(dst, src).map_err(|e| {
            let code = e.raw_os_error();
            let mut fe = err::io("hardlink", dst, e);
            if code == Some(17) {
                fe.message = "hard links must be on the same volume as their target".into();
            } else if code == Some(1142) {
                fe.message = "too many hard links to this file".into();
            }
            fe
        })
    }

    pub(super) fn junction(target: &Path, link: &Path) -> FsResult<()> {
        let abs = crate::path::strip_verbatim(&crate::path::normalize(target));
        let abs_s = abs.to_string_lossy().into_owned();
        if abs_s.starts_with(r"\\") {
            return Err(err::unsupported(
                "junction",
                link,
                "junctions cannot point to network paths; use a symbolic link",
            ));
        }
        let subst: Vec<u16> = format!(r"\??\{abs_s}").encode_utf16().collect();
        let print: Vec<u16> = abs_s.encode_utf16().collect();
        let buf = reparse::build_mount_point(&subst, &print).map_err(err::map("junction", link))?;
        wfs::create_dir(link, None).map_err(err::map("junction", link))?;
        if let Err(e) = wfs::set_reparse_data(link, &buf) {
            let _ = wfs::remove_dir(link);
            return Err(err::io("junction", link, e));
        }
        Ok(())
    }

    pub(super) fn read_link(path: &Path) -> FsResult<PathBuf> {
        let buf = wfs::get_reparse_data(path).map_err(err::map("read_link", path))?;
        let parsed = reparse::parse(&buf)
            .map_err(|e| FsError::with_message(FsErrorKind::Corrupt, "read_link", Some(path), e.to_string()))?;
        let s = |w: &[u16]| String::from_utf16_lossy(w);
        Ok(PathBuf::from(match parsed {
            Reparse::Symlink { substitute, relative: true, .. } => s(&substitute),
            Reparse::Symlink { substitute, print, .. } | Reparse::MountPoint { substitute, print } => {
                let sub = reparse::nt_to_dos(&s(&substitute));
                if sub.is_empty() { s(&print) } else { sub }
            }
            Reparse::AppExecLink { target, .. } => s(&target),
            Reparse::Other(tag) => {
                return Err(FsError::with_message(
                    FsErrorKind::Unsupported,
                    "read_link",
                    Some(path),
                    format!("not a symbolic link or junction (reparse tag {tag:#010x})"),
                ));
            }
        }))
    }
}

#[cfg(unix)]
mod imp {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::{Path, PathBuf};

    use rustix::fs::{AtFlags, CWD};
    use soyuz_core::FsResult;

    use super::LinkKind;
    use crate::err;

    pub(super) fn symlink(target: &Path, link: &Path, _kind: LinkKind) -> FsResult<()> {
        rustix::fs::symlinkat(target, CWD, link).map_err(|e| err::io("symlink", link, e.into()))
    }

    pub(super) fn hardlink(src: &Path, dst: &Path) -> FsResult<()> {
        rustix::fs::linkat(CWD, src, CWD, dst, AtFlags::empty()).map_err(|e| err::io("hardlink", dst, e.into()))
    }

    pub(super) fn junction(_target: &Path, link: &Path) -> FsResult<()> {
        Err(err::unsupported("junction", link, "junctions are an NTFS feature; use a symbolic link"))
    }

    pub(super) fn read_link(path: &Path) -> FsResult<PathBuf> {
        let t = rustix::fs::readlinkat(CWD, path, Vec::new()).map_err(|e| err::io("read_link", path, e.into()))?;
        Ok(PathBuf::from(OsString::from_vec(t.into_bytes())))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::FsResult;

    use super::LinkKind;
    use crate::err;

    pub(super) fn symlink(_t: &Path, link: &Path, _k: LinkKind) -> FsResult<()> {
        Err(err::unsupported("symlink", link, "not supported on this platform"))
    }
    pub(super) fn hardlink(src: &Path, dst: &Path) -> FsResult<()> {
        std::fs::hard_link(src, dst).map_err(err::map("hardlink", dst))
    }
    pub(super) fn junction(_t: &Path, link: &Path) -> FsResult<()> {
        Err(err::unsupported("junction", link, "not supported on this platform"))
    }
    pub(super) fn read_link(path: &Path) -> FsResult<PathBuf> {
        std::fs::read_link(path).map_err(err::map("read_link", path))
    }
}
