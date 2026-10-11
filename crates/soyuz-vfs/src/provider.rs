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

/// Whether `name` is one safe path segment: not empty, not `.` / `..`, and free of NUL
/// and path separators. `remote` names may not contain `\` either (a Windows / Mac SMB
/// client can plant `..\..\x`, and SFTP allows a literal backslash); local unix names
/// may (Windows names never do).
pub fn is_safe_name(name: &str, remote: bool) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\0'])
        && !(name.contains('\\') && (remote || cfg!(windows)))
}

/// [`is_safe_name`] as an error (`InvalidName`) for `op` on the child `name` of `dir`.
pub fn check_child_name(op: &'static str, dir: &str, name: &str, remote: bool) -> FsResult<()> {
    if is_safe_name(name, remote) {
        Ok(())
    } else {
        Err(err(
            FsErrorKind::InvalidName,
            op,
            dir,
            format!("the server listed an unsafe name ({name:?}); refusing to follow it"),
        ))
    }
}

/// Drop entries with unsafe names ([`is_safe_name`]) from a remote listing of `dir`.
/// The first time this happens it is logged as a warning, later ones at debug level.
pub fn sanitize_listing(who: &str, dir: &str, mut entries: Vec<RemoteEntry>) -> Vec<RemoteEntry> {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    entries.retain(|e| {
        let ok = is_safe_name(&e.name, true);
        if !ok {
            if WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::debug!("soyuz-vfs: {who}: skipping unsafe name {:?} in {dir}", e.name);
            } else {
                log::warn!(
                    "soyuz-vfs: {who}: skipping unsafe name {:?} in {dir} (further ones are logged at debug level)",
                    e.name
                );
            }
        }
        ok
    });
    entries
}

/// Recursively delete `path` (children first). Directories are listed; files deleted.
/// A child whose listed name is not a single safe segment ([`is_safe_name`]) stops the
/// delete with `InvalidName` instead of being joined into a path outside `path`.
pub fn delete_tree(p: &dyn Provider, path: &str, is_dir: bool, check: &dyn Fn() -> FsResult<()>) -> FsResult<()> {
    if is_dir {
        let remote = p.uri_of(path).is_some();
        for c in p.list(path)? {
            check()?;
            check_child_name("delete", path, &c.name, remote)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeProvider;

    /// A server whose listings carry names a hostile client planted, unfiltered (what a
    /// provider without [`sanitize_listing`] would hand back).
    struct Hostile {
        inner: FakeProvider,
        names: Vec<&'static str>,
    }

    impl Provider for Hostile {
        fn describe(&self) -> String {
            "hostile".into()
        }
        fn capabilities(&self) -> Capabilities {
            self.inner.capabilities()
        }
        fn list(&self, path: &str) -> FsResult<Vec<RemoteEntry>> {
            let mut l = self.inner.list(path)?;
            if path == "/share/dir" {
                l.extend(self.names.iter().map(|n| RemoteEntry::file(*n, 1, None)));
            }
            Ok(l)
        }
        fn stat(&self, path: &str) -> FsResult<RemoteEntry> {
            self.inner.stat(path)
        }
        fn open_read(&self, path: &str, offset: u64) -> FsResult<Box<dyn Read + Send>> {
            self.inner.open_read(path, offset)
        }
        fn create_write(&self, path: &str, opts: &WriteOpts) -> FsResult<Box<dyn WriteStream>> {
            self.inner.create_write(path, opts)
        }
        fn mkdir(&self, path: &str) -> FsResult<()> {
            self.inner.mkdir(path)
        }
        fn rename(&self, from: &str, to: &str) -> FsResult<()> {
            self.inner.rename(from, to)
        }
        fn delete(&self, path: &str, is_dir: bool) -> FsResult<()> {
            self.inner.delete(path, is_dir)
        }
        fn uri_of(&self, path: &str) -> Option<RemoteUri> {
            self.inner.uri_of(path)
        }
    }

    #[test]
    fn safe_names() {
        for bad in ["", ".", "..", "a/b", "..\\..\\Other\\victim.txt", "a\\b", "nul\0"] {
            assert!(!is_safe_name(bad, true), "{bad:?}");
        }
        for good in ["a", "a b.txt", "...", "..a", "ü€"] {
            assert!(is_safe_name(good, true), "{good:?}");
        }
        assert_eq!(is_safe_name("a\\b", false), !cfg!(windows));
        let l = sanitize_listing(
            "t",
            "/d",
            vec![RemoteEntry::file("ok", 1, None), RemoteEntry::file("..\\x", 1, None), RemoteEntry::dir("..", None)],
        );
        assert_eq!(l.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["ok"]);
    }

    #[test]
    fn delete_tree_refuses_hostile_names() {
        for name in ["..\\..\\Other\\victim.txt", "..", "../../Other/victim.txt", "x\\..\\..\\..\\Other"] {
            let inner = FakeProvider::new("hostile-delete");
            inner.put("/share/dir/keep.txt", b"k");
            inner.put("/Other/victim.txt", b"v");
            inner.put("/share/victim.txt", b"v");
            let p = Hostile { inner, names: vec![name] };
            let e = delete_tree(&p, "/share/dir", true, &|| Ok(())).unwrap_err();
            assert_eq!(e.kind, FsErrorKind::InvalidName, "{name:?}");
            assert!(p.inner.exists("/Other/victim.txt"), "{name:?}");
            assert!(p.inner.exists("/Other"), "{name:?}");
            assert!(p.inner.exists("/share/victim.txt"), "{name:?}");
        }
    }

    #[test]
    fn hostile_names_are_not_listed_or_followed() {
        // A name with a literal backslash stored on the server (what Samba exposes for a
        // `..\..\Other\victim.txt` planted by a Windows / Mac client, or a plain SFTP name).
        let p = FakeProvider::new("hostile-list");
        p.put("/share/dir/a.txt", b"a");
        p.put("/share/dir/..\\..\\Other\\victim.txt", b"x");
        p.put("/Other/victim.txt", b"v");
        let names: Vec<String> = p.list("/share/dir").unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["a.txt"]);
        // The planted node keeps the folder non-empty; nothing outside it is touched.
        let e = delete_tree(&p, "/share/dir", true, &|| Ok(())).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::NotEmpty);
        assert!(!p.exists("/share/dir/a.txt"));
        assert_eq!(p.get("/Other/victim.txt").as_deref(), Some(&b"v"[..]));
    }
}
