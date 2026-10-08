//! The workspace-wide filesystem error type.

use std::borrow::Cow;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Portable error classification (raw OS codes are mapped onto these, see PLAN §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsErrorKind {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotEmpty,
    CrossDevice,
    NoSpace,
    NameTooLong,
    InvalidName,
    IsDirectory,
    NotDirectory,
    InUse,
    ReadOnlyFs,
    Unsupported,
    Io,
    /// Operation cancelled through [`OpControl`](crate::OpControl).
    Interrupted,
    Loop,
    /// Corrupt archive data.
    Corrupt,
    WrongPassword,
    PasswordRequired,
}

impl FsErrorKind {
    /// Short default message for this kind.
    pub fn describe(self) -> &'static str {
        match self {
            FsErrorKind::NotFound => "not found",
            FsErrorKind::PermissionDenied => "permission denied",
            FsErrorKind::AlreadyExists => "already exists",
            FsErrorKind::NotEmpty => "directory not empty",
            FsErrorKind::CrossDevice => "cannot move across devices",
            FsErrorKind::NoSpace => "not enough space",
            FsErrorKind::NameTooLong => "name too long",
            FsErrorKind::InvalidName => "invalid name",
            FsErrorKind::IsDirectory => "is a directory",
            FsErrorKind::NotDirectory => "not a directory",
            FsErrorKind::InUse => "in use by another process",
            FsErrorKind::ReadOnlyFs => "read-only filesystem",
            FsErrorKind::Unsupported => "not supported",
            FsErrorKind::Io => "I/O error",
            FsErrorKind::Interrupted => "cancelled",
            FsErrorKind::Loop => "too many levels of symbolic links",
            FsErrorKind::Corrupt => "corrupt data",
            FsErrorKind::WrongPassword => "wrong password",
            FsErrorKind::PasswordRequired => "password required",
        }
    }
}

/// A filesystem (or archive) error with context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FsError {
    pub kind: FsErrorKind,
    pub path: Option<PathBuf>,
    /// Operation name, e.g. `"copy"`, `"read_dir"`.
    pub op: Cow<'static, str>,
    /// Raw OS error code when known.
    pub code: Option<i32>,
    pub message: String,
}

pub type FsResult<T> = Result<T, FsError>;

impl FsError {
    pub fn new(kind: FsErrorKind, op: &'static str, path: Option<&Path>) -> Self {
        FsError {
            kind,
            path: path.map(Path::to_path_buf),
            op: Cow::Borrowed(op),
            code: None,
            message: kind.describe().to_string(),
        }
    }

    /// Same as [`FsError::new`] with a custom message.
    pub fn with_message(kind: FsErrorKind, op: &'static str, path: Option<&Path>, message: impl Into<String>) -> Self {
        FsError { message: message.into(), ..FsError::new(kind, op, path) }
    }

    /// Shorthand for an [`FsErrorKind::Unsupported`] error.
    pub fn unsupported(op: &'static str, path: Option<&Path>) -> Self {
        FsError::new(FsErrorKind::Unsupported, op, path)
    }

    pub fn from_io(op: &'static str, path: Option<&Path>, e: &io::Error) -> Self {
        let code = e.raw_os_error();
        let kind = code.and_then(kind_from_os_code).unwrap_or_else(|| kind_from_io(e.kind()));
        FsError { kind, path: path.map(Path::to_path_buf), op: Cow::Borrowed(op), code, message: e.to_string() }
    }

    /// The cancellation error returned by [`OpControl::checkpoint`](crate::OpControl::checkpoint).
    pub fn interrupted(op: &'static str) -> Self {
        FsError::new(FsErrorKind::Interrupted, op, None)
    }

    /// Errors for which the UI offers "Retry".
    pub fn is_retryable(&self) -> bool {
        matches!(self.kind, FsErrorKind::InUse | FsErrorKind::Io | FsErrorKind::Interrupted)
    }

    pub fn is_cancel(&self) -> bool {
        self.kind == FsErrorKind::Interrupted
    }

    /// Attach a path if none is set yet.
    pub fn at(mut self, path: &Path) -> Self {
        if self.path.is_none() {
            self.path = Some(path.to_path_buf());
        }
        self
    }
}

impl std::error::Error for FsError {}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.op, self.message)?;
        if let Some(p) = &self.path {
            write!(f, " ({})", p.display())?;
        }
        Ok(())
    }
}

fn kind_from_io(k: io::ErrorKind) -> FsErrorKind {
    use io::ErrorKind as K;
    match k {
        K::NotFound => FsErrorKind::NotFound,
        K::PermissionDenied => FsErrorKind::PermissionDenied,
        K::AlreadyExists => FsErrorKind::AlreadyExists,
        K::DirectoryNotEmpty => FsErrorKind::NotEmpty,
        K::CrossesDevices => FsErrorKind::CrossDevice,
        K::StorageFull | K::QuotaExceeded => FsErrorKind::NoSpace,
        K::InvalidFilename => FsErrorKind::InvalidName,
        K::IsADirectory => FsErrorKind::IsDirectory,
        K::NotADirectory => FsErrorKind::NotDirectory,
        K::ResourceBusy | K::ExecutableFileBusy => FsErrorKind::InUse,
        K::ReadOnlyFilesystem => FsErrorKind::ReadOnlyFs,
        K::Unsupported => FsErrorKind::Unsupported,
        K::Interrupted => FsErrorKind::Interrupted,
        _ => FsErrorKind::Io,
    }
}

#[cfg(windows)]
fn kind_from_os_code(code: i32) -> Option<FsErrorKind> {
    Some(match code {
        2 | 3 => FsErrorKind::NotFound,
        5 => FsErrorKind::PermissionDenied,
        17 => FsErrorKind::CrossDevice,
        19 => FsErrorKind::ReadOnlyFs,
        32 | 33 => FsErrorKind::InUse,
        39 | 112 => FsErrorKind::NoSpace,
        50 => FsErrorKind::Unsupported,
        80 | 183 => FsErrorKind::AlreadyExists,
        123 | 161 => FsErrorKind::InvalidName,
        145 => FsErrorKind::NotEmpty,
        206 => FsErrorKind::NameTooLong,
        267 => FsErrorKind::NotDirectory,
        1921 => FsErrorKind::Loop,
        _ => return None,
    })
}

#[cfg(target_os = "linux")]
fn kind_from_os_code(code: i32) -> Option<FsErrorKind> {
    Some(match code {
        36 => FsErrorKind::NameTooLong, // ENAMETOOLONG
        40 => FsErrorKind::Loop,        // ELOOP
        16 | 26 => FsErrorKind::InUse,  // EBUSY, ETXTBSY
        _ => return None,
    })
}

#[cfg(target_os = "macos")]
fn kind_from_os_code(code: i32) -> Option<FsErrorKind> {
    Some(match code {
        63 => FsErrorKind::NameTooLong, // ENAMETOOLONG
        62 => FsErrorKind::Loop,        // ELOOP
        16 | 26 => FsErrorKind::InUse,  // EBUSY, ETXTBSY
        _ => return None,
    })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn kind_from_os_code(_code: i32) -> Option<FsErrorKind> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_mapping() {
        let e = io::Error::from(io::ErrorKind::NotFound);
        let fe = FsError::from_io("stat", Some(Path::new("/x")), &e);
        assert_eq!(fe.kind, FsErrorKind::NotFound);
        assert_eq!(fe.path.as_deref(), Some(Path::new("/x")));
        assert!(!fe.is_retryable());
        assert!(FsError::interrupted("copy").is_cancel());
        assert!(FsError::interrupted("copy").is_retryable());
        let s = fe.to_string();
        assert!(s.starts_with("stat: "), "{s}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_codes() {
        let e = io::Error::from_raw_os_error(32);
        assert_eq!(FsError::from_io("delete", None, &e).kind, FsErrorKind::InUse);
        let e = io::Error::from_raw_os_error(17);
        assert_eq!(FsError::from_io("rename", None, &e).kind, FsErrorKind::CrossDevice);
    }

    #[test]
    fn serde_round_trip() {
        let fe = FsError::new(FsErrorKind::NoSpace, "copy", Some(Path::new("/a")));
        let s = toml::to_string(&fe).unwrap();
        let back: FsError = toml::from_str(&s).unwrap();
        assert_eq!(back, fe);
    }
}
