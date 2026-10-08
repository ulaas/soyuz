//! Creating files and directories, atomic writes.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use soyuz_core::{EntryKind, FsError, FsErrorKind, FsResult};

use crate::err;

/// Create one directory (`AlreadyExists` if anything exists at `path`, `NotFound` when the
/// parent is missing).
pub fn create_dir(path: &Path) -> FsResult<()> {
    crate::fault::hit(crate::fault::FaultOp::Create, path).map_err(err::map("create_dir", path))?;
    imp::mkdir(path)
}

/// Create a directory and all missing parents; existing directories are fine, an existing
/// non-directory on the way is `NotDirectory`.
pub fn create_dir_all(path: &Path) -> FsResult<()> {
    let path = crate::path::normalize(path);
    let mut missing: Vec<PathBuf> = Vec::new();
    let mut cur = Some(path.as_path());
    while let Some(p) = cur {
        match crate::list::stat(p, true) {
            Ok(e) if e.is_dir_like() => break,
            Ok(_) => {
                return Err(FsError::with_message(
                    FsErrorKind::NotDirectory,
                    "create_dir",
                    Some(p),
                    "a file with this name already exists",
                ));
            }
            Err(e) if e.kind == FsErrorKind::NotFound => {
                missing.push(p.to_path_buf());
                cur = p.parent();
            }
            Err(e) => return Err(FsError { op: "create_dir".into(), ..e }),
        }
    }
    for dir in missing.iter().rev() {
        match imp::mkdir(dir) {
            Ok(()) => {}
            // Raced with another creator: fine as long as it is a directory now.
            Err(e)
                if e.kind == FsErrorKind::AlreadyExists
                    && crate::list::stat(dir, true).is_ok_and(|e| e.is_dir_like()) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Create an empty file; `exclusive` fails with `AlreadyExists` if it exists, otherwise an
/// existing file is left untouched (not truncated).
pub fn create_file(path: &Path, exclusive: bool) -> FsResult<()> {
    if !exclusive && let Ok(e) = crate::list::stat(path, true) {
        return if e.kind == EntryKind::Dir {
            Err(FsError::new(FsErrorKind::IsDirectory, "create_file", Some(path)))
        } else {
            Ok(())
        };
    }
    let mut o = OpenOptions::new();
    o.write(true);
    if exclusive {
        o.create_new(true);
    } else {
        o.create(true);
    }
    o.open(path).map(drop).map_err(err::map("create_file", path))
}

/// Temp file + fsync + rename in the same directory; used for config / session files. On
/// unix the existing file's permission bits are kept.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> FsResult<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let tmp =
        path.with_file_name(format!(".{name}.tmp-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let res = (|| {
        let mut f =
            OpenOptions::new().write(true).create_new(true).open(&tmp).map_err(err::map("write_atomic", &tmp))?;
        f.write_all(bytes).map_err(err::map("write_atomic", &tmp))?;
        #[cfg(unix)]
        if let Some(mode) = crate::list::stat(path, true).ok().and_then(|e| e.mode) {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(mode & 0o7777));
        }
        f.sync_all().map_err(err::map("write_atomic", &tmp))?;
        drop(f);
        crate::mv::rename(&tmp, path, crate::mv::RenameMode::Replace)
            .map_err(|e| FsError { op: "write_atomic".into(), ..e })
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use soyuz_core::FsResult;

    use crate::err;

    pub(super) fn mkdir(path: &Path) -> FsResult<()> {
        crate::sys::windows::fs::create_dir(path, None).map_err(err::map("create_dir", path))
    }
}

#[cfg(unix)]
mod imp {
    use std::path::Path;

    use rustix::fs::{CWD, Mode};
    use soyuz_core::FsResult;

    use crate::err;

    pub(super) fn mkdir(path: &Path) -> FsResult<()> {
        rustix::fs::mkdirat(CWD, path, Mode::from_raw_mode(0o777)).map_err(|e| err::io("create_dir", path, e.into()))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::Path;

    use soyuz_core::FsResult;

    use crate::err;

    pub(super) fn mkdir(path: &Path) -> FsResult<()> {
        std::fs::create_dir(path).map_err(err::map("create_dir", path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_and_files() {
        let d = tempfile::tempdir().unwrap();
        let deep = d.path().join("a").join("b").join("c");
        create_dir_all(&deep).unwrap();
        create_dir_all(&deep).unwrap();
        assert!(deep.is_dir());
        assert_eq!(create_dir(&deep).unwrap_err().kind, FsErrorKind::AlreadyExists);
        assert_eq!(create_dir(&d.path().join("x").join("y")).unwrap_err().kind, FsErrorKind::NotFound);
        let f = d.path().join("a").join("file");
        create_file(&f, false).unwrap();
        std::fs::write(&f, b"keep").unwrap();
        create_file(&f, false).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"keep", "non-exclusive create does not truncate");
        assert_eq!(create_dir_all(&f.join("sub")).unwrap_err().kind, FsErrorKind::NotDirectory);
        assert_eq!(create_file(&d.path().join("a"), false).unwrap_err().kind, FsErrorKind::IsDirectory);
    }
}
