//! Permanent deletion. Links (symlinks, junctions) are never followed.
//!
//! * Windows: `SetFileInformationByHandle(FileDispositionInfoEx, DELETE | POSIX_SEMANTICS
//!   [| IGNORE_READONLY_ATTRIBUTE])` on a handle opened with `FILE_FLAG_OPEN_REPARSE_POINT`
//!   (falls back to `DeleteFileW` on FAT / old Windows), `RemoveDirectoryW` for directories.
//! * Unix: `unlinkat` relative to directory fds opened with `O_NOFOLLOW`, so a tree being
//!   removed cannot be redirected through a swapped-in symlink.

use std::fs::OpenOptions;
use std::io::{Seek, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use soyuz_core::{ByteProgress, Control, FsError, FsErrorKind, FsResult, OpControl, ProgressSink};

use crate::err;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteOptions {
    /// Clear the read-only attribute and retry when deletion is denied.
    pub clear_readonly: bool,
    /// Windows: POSIX delete semantics (delete files with open handles).
    pub posix_semantics: bool,
}

impl Default for DeleteOptions {
    fn default() -> Self {
        DeleteOptions { clear_readonly: false, posix_semantics: true }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DeleteStats {
    pub files: u64,
    pub dirs: u64,
    pub links: u64,
    pub failed: u64,
}

/// Delete a file (or a link to a file). Directories → `IsDirectory`.
#[track_caller]
pub fn delete_file(path: &Path, o: &DeleteOptions) -> FsResult<()> {
    let _guard = crate::guard::check("delete_file");
    imp::delete_file(path, o)
}

/// Remove an empty directory (`NotEmpty` otherwise).
pub fn delete_empty_dir(path: &Path) -> FsResult<()> {
    imp::remove_dir(path, false)
}

/// Remove a symlink / junction itself (never follows).
pub fn delete_link(path: &Path) -> FsResult<()> {
    imp::delete_link(path)
}

/// Post-order removal of a tree; `on_item` sees every attempted item and may stop.
/// Never descends into symlinked or junctioned directories (the link itself is removed).
#[track_caller]
pub fn remove_tree(
    root: &Path,
    o: &DeleteOptions,
    ctl: &OpControl,
    on_item: &mut dyn FnMut(&Path, FsResult<()>) -> Control,
) -> FsResult<DeleteStats> {
    let _guard = crate::guard::check("remove_tree");
    imp::remove_tree(root, o, ctl, on_item)
}

/// Overwrite the file `passes` times (random, last pass zeros), then delete it.
/// Meaningless on SSD / copy-on-write filesystems (the UI says so).
pub fn secure_wipe(path: &Path, passes: u8, ctl: &OpControl, sink: &mut dyn ProgressSink) -> FsResult<()> {
    let passes = passes.max(1) as u64;
    let e = crate::list::stat(path, false).map_err(|e| FsError { op: "secure_wipe".into(), ..e })?;
    if e.kind != soyuz_core::EntryKind::File {
        return Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "secure_wipe",
            Some(path),
            "only regular files can be wiped",
        ));
    }
    let len = e.size;
    {
        let mut f = OpenOptions::new().write(true).open(path).map_err(err::map("secure_wipe", path))?;
        let total = len * passes;
        let mut done = 0u64;
        let mut buf = vec![0u8; 1 << 20];
        let mut seed =
            (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(1)
                as u64)
                | 1;
        for pass in 0..passes {
            f.seek(std::io::SeekFrom::Start(0)).map_err(err::map("secure_wipe", path))?;
            let last = pass + 1 == passes;
            let mut left = len;
            while left > 0 {
                ctl.checkpoint()?;
                let n = left.min(buf.len() as u64) as usize;
                if last {
                    buf[..n].fill(0);
                } else {
                    for b in &mut buf[..n] {
                        // xorshift64
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        *b = seed as u8;
                    }
                }
                f.write_all(&buf[..n]).map_err(err::map("secure_wipe", path))?;
                left -= n as u64;
                done += n as u64;
                sink.on_progress(ByteProgress { done, total, delta: n as u64 });
            }
            f.sync_all().map_err(err::map("secure_wipe", path))?;
        }
        f.set_len(0).map_err(err::map("secure_wipe", path))?;
    }
    delete_file(path, &DeleteOptions { clear_readonly: true, posix_semantics: true })
}

fn not_a_link(path: &Path) -> FsError {
    FsError::with_message(FsErrorKind::Unsupported, "delete_link", Some(path), "not a symbolic link or junction")
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};

    use soyuz_core::{Control, EntryKind, FsError, FsErrorKind, FsResult, OpControl};

    use super::{DeleteOptions, DeleteStats};
    use crate::err;
    use crate::list::{WalkItem, WalkOptions, walk};
    use crate::sys::windows::fs as wfs;

    const READONLY: u32 = 0x1;
    const ACCESS_DENIED: i32 = 5;

    fn readonly_error(op: &'static str, path: &Path, e: std::io::Error) -> FsError {
        let mut fe = err::io(op, path, e);
        fe.message = "the item is read-only (clear the read-only attribute to delete it)".into();
        fe
    }

    fn clear_ro(path: &Path, attrs: u32) -> std::io::Result<()> {
        // Keep the other settable bits; FILE_ATTRIBUTE_NORMAL when nothing remains.
        let keep = attrs & !READONLY & 0x3127;
        wfs::set_attributes(path, if keep == 0 { 0x80 } else { keep })
    }

    pub(super) fn delete_file(path: &Path, o: &DeleteOptions) -> FsResult<()> {
        crate::fault::hit(crate::fault::FaultOp::Delete, path).map_err(err::map("delete", path))?;
        let st = wfs::stat_path(path, false).map_err(err::map("delete", path))?;
        if st.is_reparse() && st.is_dir() {
            return delete_link(path);
        }
        if st.is_dir() {
            return Err(FsError::new(FsErrorKind::IsDirectory, "delete", Some(path)));
        }
        if o.posix_semantics {
            match wfs::posix_delete(path, o.clear_readonly) {
                Ok(()) => return Ok(()),
                Err(e) if wfs::is_unsupported_error(&e) => {}
                Err(e) if e.raw_os_error() == Some(ACCESS_DENIED) && st.attrs & READONLY != 0 && !o.clear_readonly => {
                    return Err(readonly_error("delete", path, e));
                }
                Err(e) if e.raw_os_error() == Some(ACCESS_DENIED) => {}
                Err(e) => return Err(err::io("delete", path, e)),
            }
        }
        match wfs::delete_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(ACCESS_DENIED) && st.attrs & READONLY != 0 => {
                if !o.clear_readonly {
                    return Err(readonly_error("delete", path, e));
                }
                clear_ro(path, st.attrs).map_err(err::map("delete", path))?;
                wfs::delete_file(path).map_err(err::map("delete", path))
            }
            Err(e) => Err(err::io("delete", path, e)),
        }
    }

    pub(super) fn remove_dir(path: &Path, clear_readonly: bool) -> FsResult<()> {
        crate::fault::hit(crate::fault::FaultOp::Delete, path).map_err(err::map("delete", path))?;
        match wfs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(ACCESS_DENIED) && clear_readonly => {
                let attrs = wfs::get_attributes(path).map_err(err::map("delete", path))?;
                if attrs & READONLY == 0 {
                    return Err(err::io("delete", path, e));
                }
                clear_ro(path, attrs).map_err(err::map("delete", path))?;
                wfs::remove_dir(path).map_err(err::map("delete", path))
            }
            Err(e) => Err(err::io("delete", path, e)),
        }
    }

    pub(super) fn delete_link(path: &Path) -> FsResult<()> {
        crate::fault::hit(crate::fault::FaultOp::Delete, path).map_err(err::map("delete_link", path))?;
        let st = wfs::stat_path(path, false).map_err(err::map("delete_link", path))?;
        if !st.is_reparse() {
            return Err(super::not_a_link(path));
        }
        let res = if st.is_dir() { wfs::remove_dir(path) } else { wfs::delete_file(path) };
        match res {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(ACCESS_DENIED) && st.attrs & READONLY != 0 => {
                clear_ro(path, st.attrs).map_err(err::map("delete_link", path))?;
                let again = if st.is_dir() { wfs::remove_dir(path) } else { wfs::delete_file(path) };
                again.map_err(err::map("delete_link", path))
            }
            Err(e) => Err(err::io("delete_link", path, e)),
        }
    }

    pub(super) fn remove_tree(
        root: &Path,
        o: &DeleteOptions,
        ctl: &OpControl,
        on_item: &mut dyn FnMut(&Path, FsResult<()>) -> Control,
    ) -> FsResult<DeleteStats> {
        let mut stats = DeleteStats::default();
        let wopts = WalkOptions { follow_symlinks: false, post_order: true, max_depth: None, same_device: false };
        walk(root, &wopts, ctl, &mut |item| {
            let (path, res, counter): (PathBuf, FsResult<()>, Option<&mut u64>) = match item {
                WalkItem::Entry { entry, .. } => match entry.kind {
                    // Checked before `Dir`: a folder that is really a link (other
                    // name-surrogate reparse tags) loses the link, never its target's contents.
                    _ if crate::list::is_link_dir(&entry) => {
                        let r = delete_link(&entry.path);
                        (entry.path, r, Some(&mut stats.links))
                    }
                    EntryKind::Dir => return Control::Continue, // removed on DirDone
                    EntryKind::Symlink { .. } | EntryKind::Junction => {
                        let r = delete_link(&entry.path);
                        (entry.path, r, Some(&mut stats.links))
                    }
                    _ => {
                        let r = delete_file(&entry.path, o);
                        (entry.path, r, Some(&mut stats.files))
                    }
                },
                WalkItem::DirDone { path, .. } => {
                    let r = remove_dir(&path, o.clear_readonly);
                    (path, r, Some(&mut stats.dirs))
                }
                WalkItem::Error { path, error } => (path, Err(error), None),
            };
            match (&res, counter) {
                (Ok(()), Some(c)) => *c += 1,
                (Err(_), _) => stats.failed += 1,
                _ => {}
            }
            on_item(&path, res)
        })
        .map_err(|e| FsError { op: "remove_tree".into(), ..e })?;
        Ok(stats)
    }
}

// ---------------------------------------------------------------------------
// Unix
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod imp {
    use std::ffi::{OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use rustix::fd::{AsFd, OwnedFd};
    use rustix::fs::{AtFlags, CWD, Dir, Mode, OFlags};
    use rustix::io::Errno;
    use soyuz_core::{Attributes, Control, FsError, FsErrorKind, FsResult, OpControl};

    use super::{DeleteOptions, DeleteStats};
    use crate::err;
    use crate::sys::unix as us;

    const S_IFMT: u32 = 0o170000;
    const S_IFDIR: u32 = 0o040000;
    const S_IFLNK: u32 = 0o120000;

    fn lstat_mode(path: &Path, op: &'static str) -> FsResult<u32> {
        us::ustat_at(CWD, path.as_os_str(), false).map(|s| s.mode).map_err(err::map(op, path))
    }

    fn try_clear_immutable(path: &Path) -> bool {
        crate::attrs::set_attributes(
            path,
            &crate::attrs::AttrSet { set: Attributes::empty(), clear: Attributes::IMMUTABLE },
        )
        .is_ok()
    }

    pub(super) fn delete_file(path: &Path, o: &DeleteOptions) -> FsResult<()> {
        let mode = lstat_mode(path, "delete")?;
        if mode & S_IFMT == S_IFDIR {
            return Err(FsError::new(FsErrorKind::IsDirectory, "delete", Some(path)));
        }
        match rustix::fs::unlinkat(CWD, path, AtFlags::empty()) {
            Ok(()) => Ok(()),
            Err(Errno::PERM) if o.clear_readonly && try_clear_immutable(path) => {
                rustix::fs::unlinkat(CWD, path, AtFlags::empty()).map_err(|e| err::io("delete", path, e.into()))
            }
            Err(e) => Err(err::io("delete", path, e.into())),
        }
    }

    pub(super) fn remove_dir(path: &Path, _clear_readonly: bool) -> FsResult<()> {
        rustix::fs::unlinkat(CWD, path, AtFlags::REMOVEDIR).map_err(|e| err::io("delete", path, e.into()))
    }

    pub(super) fn delete_link(path: &Path) -> FsResult<()> {
        let mode = lstat_mode(path, "delete_link")?;
        if mode & S_IFMT != S_IFLNK {
            return Err(super::not_a_link(path));
        }
        rustix::fs::unlinkat(CWD, path, AtFlags::empty()).map_err(|e| err::io("delete_link", path, e.into()))
    }

    struct Frame {
        dir: Dir,
        path: PathBuf,
        /// Name inside the parent frame's directory (None for the root).
        name: Option<OsString>,
    }

    fn open_dir_nofollow(dirfd: impl AsFd, name: &OsStr) -> rustix::io::Result<Dir> {
        let fd: OwnedFd = rustix::fs::openat(
            dirfd,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Dir::new(fd)
    }

    pub(super) fn remove_tree(
        root: &Path,
        o: &DeleteOptions,
        ctl: &OpControl,
        on_item: &mut dyn FnMut(&Path, FsResult<()>) -> Control,
    ) -> FsResult<DeleteStats> {
        let mut stats = DeleteStats::default();
        let mode = lstat_mode(root, "remove_tree")?;
        ctl.checkpoint()?;
        if mode & S_IFMT != S_IFDIR {
            let is_link = mode & S_IFMT == S_IFLNK;
            let res = if is_link { delete_link(root) } else { delete_file(root, o) };
            match (&res, is_link) {
                (Ok(()), true) => stats.links += 1,
                (Ok(()), false) => stats.files += 1,
                (Err(_), _) => stats.failed += 1,
            }
            on_item(root, res);
            return Ok(stats);
        }
        let dir = open_dir_nofollow(CWD, root.as_os_str()).map_err(|e| err::io("remove_tree", root, e.into()))?;
        let mut stack = vec![Frame { dir, path: root.to_path_buf(), name: None }];

        macro_rules! report {
            ($path:expr, $res:expr, $counter:expr) => {{
                let res: FsResult<()> = $res;
                match &res {
                    Ok(()) => $counter += 1,
                    Err(_) => stats.failed += 1,
                }
                if on_item($path, res) == Control::Stop {
                    return Ok(stats);
                }
            }};
        }

        while let Some(top) = stack.last_mut() {
            let next = top.dir.read();
            match next {
                None | Some(Err(_)) => {
                    if let Some(Err(e)) = next {
                        let path = top.path.clone();
                        stats.failed += 1;
                        if on_item(&path, Err(err::io("remove_tree", &path, e.into()))) == Control::Stop {
                            return Ok(stats);
                        }
                    }
                    let done = stack.pop().expect("non-empty stack");
                    let res = match (&done.name, stack.last()) {
                        (Some(name), Some(parent)) => match parent.dir.fd() {
                            Ok(pfd) => rustix::fs::unlinkat(pfd, name.as_os_str(), AtFlags::REMOVEDIR),
                            Err(e) => Err(e),
                        },
                        _ => rustix::fs::unlinkat(CWD, &done.path, AtFlags::REMOVEDIR),
                    }
                    .map_err(|e| err::io("delete", &done.path, e.into()));
                    report!(&done.path, res, stats.dirs);
                }
                Some(Ok(ent)) => {
                    let name = OsStr::from_bytes(ent.file_name().to_bytes()).to_os_string();
                    if name == "." || name == ".." {
                        continue;
                    }
                    ctl.checkpoint()?;
                    let path = top.path.join(&name);
                    let fd = match top.dir.fd() {
                        Ok(fd) => fd,
                        Err(e) => {
                            report!(&path, Err(err::io("delete", &path, e.into())), stats.files);
                            continue;
                        }
                    };
                    let st = match us::ustat_at(fd, &name, false) {
                        Ok(st) => st,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => {
                            report!(&path, Err(err::io("delete", &path, e)), stats.files);
                            continue;
                        }
                    };
                    match st.mode & S_IFMT {
                        S_IFDIR => match open_dir_nofollow(fd, &name) {
                            Ok(dir) => stack.push(Frame { dir, path, name: Some(name) }),
                            Err(e) => report!(&path, Err(err::io("delete", &path, e.into())), stats.dirs),
                        },
                        kind => {
                            let mut res = rustix::fs::unlinkat(fd, name.as_os_str(), AtFlags::empty());
                            if res == Err(Errno::PERM) && o.clear_readonly && try_clear_immutable(&path) {
                                res = rustix::fs::unlinkat(fd, name.as_os_str(), AtFlags::empty());
                            }
                            let res = res.map_err(|e| err::io("delete", &path, e.into()));
                            if kind == S_IFLNK {
                                report!(&path, res, stats.links);
                            } else {
                                report!(&path, res, stats.files);
                            }
                        }
                    }
                }
            }
        }
        Ok(stats)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::Path;

    use soyuz_core::{Control, FsResult, OpControl};

    use super::{DeleteOptions, DeleteStats};
    use crate::err;

    pub(super) fn delete_file(path: &Path, _o: &DeleteOptions) -> FsResult<()> {
        std::fs::remove_file(path).map_err(err::map("delete", path))
    }
    pub(super) fn remove_dir(path: &Path, _c: bool) -> FsResult<()> {
        std::fs::remove_dir(path).map_err(err::map("delete", path))
    }
    pub(super) fn delete_link(path: &Path) -> FsResult<()> {
        std::fs::remove_file(path).map_err(err::map("delete_link", path))
    }
    pub(super) fn remove_tree(
        root: &Path,
        _o: &DeleteOptions,
        _ctl: &OpControl,
        _on_item: &mut dyn FnMut(&Path, FsResult<()>) -> Control,
    ) -> FsResult<DeleteStats> {
        std::fs::remove_dir_all(root).map_err(err::map("remove_tree", root))?;
        Ok(DeleteStats::default())
    }
}
