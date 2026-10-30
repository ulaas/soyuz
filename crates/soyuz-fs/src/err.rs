//! Raw OS error → [`FsError`] mapping with actionable messages.

use std::io;
use std::path::Path;

use soyuz_core::{FsError, FsErrorKind};

/// Map an OS error, adding hints for codes that need more than the OS text.
pub(crate) fn io(op: &'static str, path: &Path, e: io::Error) -> FsError {
    io_opt(op, Some(path), e)
}

pub(crate) fn io_opt(op: &'static str, path: Option<&Path>, e: io::Error) -> FsError {
    let mut fe = FsError::from_io(op, path, &e);
    if let Some(code) = e.raw_os_error() {
        refine(&mut fe, code);
    }
    fe
}

/// Closure form for `map_err`.
pub(crate) fn map<'a>(op: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> FsError + 'a {
    move |e| io(op, path, e)
}

#[cfg(windows)]
fn refine(fe: &mut FsError, code: i32) {
    let set = |fe: &mut FsError, kind: FsErrorKind, msg: &str| {
        fe.kind = kind;
        fe.message = format!("{msg} (os error {code})");
    };
    match code {
        1314 => set(
            fe,
            FsErrorKind::PermissionDenied,
            "a required privilege is not held: enable Developer Mode or run Soyuz elevated",
        ),
        1235 => set(fe, FsErrorKind::Interrupted, "the operation was cancelled"),
        740 => set(
            fe,
            FsErrorKind::PermissionDenied,
            "this program needs administrator rights (use \u{201c}Run as administrator\u{201d})",
        ),
        21 => set(fe, FsErrorKind::Io, "the device is not ready (no media inserted?)"),
        32 | 33 => set(fe, FsErrorKind::InUse, "the file is open in another program; close it and retry"),
        4390 => set(fe, FsErrorKind::Unsupported, "not a symbolic link or junction"),
        4392 => set(fe, FsErrorKind::Corrupt, "the reparse point data is invalid"),
        1920 | 362..=400 => {
            set(fe, FsErrorKind::Io, "the cloud file is not available (offline or sync provider not running)")
        }
        225 | 226 => set(fe, FsErrorKind::PermissionDenied, "blocked by antivirus software"),
        1392 => set(fe, FsErrorKind::Corrupt, "the file or directory is corrupted and unreadable (run chkdsk)"),
        _ => {}
    }
}

#[cfg(unix)]
fn refine(fe: &mut FsError, code: i32) {
    if code == libc::EPERM && fe.kind == FsErrorKind::PermissionDenied {
        fe.message = format!("{} (immutable or append-only file, or missing privilege)", fe.message);
    } else if code == libc::EXDEV {
        fe.kind = FsErrorKind::CrossDevice;
    } else if code == libc::ENOTSUP || code == libc::EOPNOTSUPP {
        fe.kind = FsErrorKind::Unsupported;
    }
}

#[cfg(not(any(unix, windows)))]
fn refine(_fe: &mut FsError, _code: i32) {}

/// An `Unsupported` error with an explanation.
pub(crate) fn unsupported(op: &'static str, path: &Path, why: impl Into<String>) -> FsError {
    FsError::with_message(FsErrorKind::Unsupported, op, Some(path), why)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        let e = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(io("stat", Path::new("x"), e).kind, FsErrorKind::NotFound);
        #[cfg(windows)]
        {
            let fe = io("symlink", Path::new("x"), io::Error::from_raw_os_error(1314));
            assert_eq!(fe.kind, FsErrorKind::PermissionDenied);
            assert!(fe.message.contains("Developer Mode"));
            let fe = io("copy", Path::new("x"), io::Error::from_raw_os_error(1235));
            assert!(fe.is_cancel());
            let fe = io("delete", Path::new("x"), io::Error::from_raw_os_error(32));
            assert!(fe.is_retryable());
        }
        #[cfg(unix)]
        {
            let fe = io("rename", Path::new("x"), io::Error::from_raw_os_error(libc::EXDEV));
            assert_eq!(fe.kind, FsErrorKind::CrossDevice);
        }
    }
}
