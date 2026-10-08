//! The [`Provider`] trait every location backend implements, plus the entry and
//! capability types shared by all of them.
//!
//! Providers are synchronous (the app and the jobs engine run them on worker threads);
//! network providers block on the crate's small tokio runtime internally. Paths are
//! provider paths: `/`-separated absolute paths for remote providers, native paths for
//! [`crate::local::LocalProvider`].

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use soyuz_core::{EntryInfo, EntryKind, FsError, FsErrorKind, FsResult, LinkTarget};

use crate::uri::RemoteUri;

/// One directory entry as a provider reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Unix permission bits when the protocol reports them.
    pub mode: Option<u32>,
}

impl RemoteEntry {
    pub fn file(name: impl Into<String>, size: u64, modified: Option<SystemTime>) -> Self {
        RemoteEntry { name: name.into(), is_dir: false, is_link: false, size, modified, mode: None }
    }

    pub fn dir(name: impl Into<String>, modified: Option<SystemTime>) -> Self {
        RemoteEntry { name: name.into(), is_dir: true, is_link: false, size: 0, modified, mode: None }
    }

    /// As a pane entry whose `path` is `full` (a URI or a native path).
    pub fn to_entry_info(&self, full: PathBuf) -> EntryInfo {
        let kind = match (self.is_dir, self.is_link) {
            (true, false) => EntryKind::Dir,
            (true, true) => EntryKind::Symlink { target: LinkTarget::Dir },
            (false, true) => EntryKind::Symlink { target: LinkTarget::File },
            (false, false) => EntryKind::File,
        };
        let mut e = EntryInfo::minimal(full, kind);
        e.name = self.name.clone();
        e.size = if self.is_dir { 0 } else { self.size };
        e.modified = self.modified;
        e.mode = self.mode;
        e
    }
}

/// What a provider supports (drives resume, atomic writes, undo and UI affordances).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// `open_read` honours a non-zero offset (SFTP seek, HTTP `Range`, FTP `REST`).
    pub range_read: bool,
    /// `create_write` can continue a partial upload ([`WriteStream::resumed_at`]).
    pub resume_write: bool,
    /// Writes become visible atomically by themselves (S3 multipart): write straight to
    /// the final name instead of a `.soyuzpart` temp + rename.
    pub atomic_write: bool,
    /// `rename` is cheap (a real rename, not copy + delete).
    pub rename: bool,
    /// `rename` replaces an existing destination file.
    pub rename_replaces: bool,
    pub set_times: bool,
    /// Changes are pushed (otherwise the app polls).
    pub watch: bool,
}

impl Capabilities {
    pub const BASIC: Capabilities = Capabilities {
        range_read: false,
        resume_write: false,
        atomic_write: false,
        rename: true,
        rename_replaces: false,
        set_times: false,
        watch: false,
    };
}

/// Options for [`Provider::create_write`].
#[derive(Clone, Debug, Default)]
pub struct WriteOpts {
    /// Continue a partial file at this path when the provider can.
    pub resume: bool,
    /// Expected final size (multipart part sizing, progress).
    pub size_hint: Option<u64>,
}

/// A streaming upload. Dropping it without [`WriteStream::commit`] behaves like
/// `abort(true)` (keep resumable state).
pub trait WriteStream: Write + Send {
    /// Bytes already present from an earlier attempt; the caller skips this many source
    /// bytes. Always `0` unless [`WriteOpts::resume`] was set and the provider resumed.
    fn resumed_at(&self) -> u64 {
        0
    }
    /// Finish the upload (flush, close, complete multipart …).
    fn commit(self: Box<Self>) -> FsResult<()>;
    /// Abandon the upload; `keep_partial` keeps what was written for a later resume.
    fn abort(self: Box<Self>, keep_partial: bool);
}

/// A location backend.
pub trait Provider: Send + Sync {
    /// Short description for logs and the UI ("SFTP alice@host").
    fn describe(&self) -> String;
    fn capabilities(&self) -> Capabilities;
    /// Children of a directory (no `.` / `..`).
    fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>>;
    /// Metadata of one entry (`NotFound` when missing).
    fn stat(&self, path: &str) -> FsResult<RemoteEntry>;
    /// Stream a file from `offset` (non-zero only when `range_read`).
    fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>>;
    /// Stream a file into `path` (created or truncated, or resumed per [`WriteOpts`]).
    fn create_write(&self, path: &str, opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>>;
    fn mkdir(&self, path: &str) -> FsResult<()>;
    fn rename(&self, from: &str, to: &str) -> FsResult<()>;
    /// Delete a file, or an empty directory when `is_dir`.
    fn delete(&self, path: &str, is_dir: bool) -> FsResult<()>;
    fn set_times(&self, path: &str, modified: SystemTime) -> FsResult<()> {
        let _ = modified;
        Err(FsError::unsupported("set_times", Some(std::path::Path::new(path))))
    }
    /// Server-side copy of a file (S3 CopyObject); `Unsupported` → stream it.
    fn copy_file(&self, from: &str, to: &str) -> FsResult<()> {
        let _ = to;
        Err(FsError::unsupported("copy", Some(std::path::Path::new(from))))
    }
    /// The URI of `path` on this provider (`None` for the local filesystem).
    fn uri_of(&self, path: &str) -> Option<RemoteUri> {
        let _ = path;
        None
    }
}

/// Error helper: `kind` + message for a provider path.
pub fn err(kind: FsErrorKind, op: &'static str, path: &str, msg: impl Into<String>) -> FsError {
    FsError::with_message(kind, op, Some(std::path::Path::new(path)), msg)
}

/// Recursively delete `path` (children first). Directories are listed; files deleted.
pub fn delete_tree(p: &dyn Provider, path: &str, is_dir: bool, check: &dyn Fn() -> FsResult<()>) -> FsResult<()> {
    if is_dir {
        for c in p.list(path)? {
            check()?;
            let child = crate::uri::join_path(path, &c.name);
            delete_tree(p, &child, c.is_dir && !c.is_link, check)?;
        }
    }
    p.delete(path, is_dir)
}

/// `Ok(None)` when `path` does not exist.
pub fn stat_opt(p: &dyn Provider, path: &str) -> FsResult<Option<RemoteEntry>> {
    match p.stat(path) {
        Ok(e) => Ok(Some(e)),
        Err(e) if e.kind == FsErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
