//! Directory listing, `stat` and tree walking.
//!
//! * Windows: `NtQueryDirectoryFile(FileIdFullDirectoryInformation)` into a 64→256 KiB buffer
//!   (falls back to `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)` when the
//!   file system refuses it) — the entry data already carries attributes, times, size, file
//!   id and the reparse tag, so no per-entry stat.
//! * Linux: `getdents` (rustix `Dir`) + `statx(dirfd, name, AT_SYMLINK_NOFOLLOW)` relative to
//!   the directory fd (falls back to `fstatat` on kernels without statx).
//! * macOS: `getdents`-style `readdir` + `fstatat(dirfd, name)`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use soyuz_core::{Attributes, Control, EntryInfo, EntryKind, FsError, FsErrorKind, FsResult, LinkTarget, OpControl};

use crate::err;

/// Options for [`read_dir`] / [`read_dir_streaming`].
#[derive(Clone, Debug)]
pub struct ListOptions {
    pub follow_symlinks: bool,
    pub include_hidden: bool,
    /// Resolve the kind of a symlink's target.
    pub stat_links: bool,
    /// Resolve owner/group names (slower).
    pub owners: bool,
    /// Entries per streamed chunk (default 512).
    pub chunk: usize,
}

impl Default for ListOptions {
    fn default() -> Self {
        ListOptions { follow_symlinks: false, include_hidden: true, stat_links: true, owners: false, chunk: 512 }
    }
}

/// Summary of a streamed listing.
#[derive(Clone, Copy, Debug, Default)]
pub struct ListStats {
    pub entries: usize,
    pub elapsed: Duration,
    /// The sink returned `Control::Stop` before the end.
    pub truncated: bool,
}

/// List a directory (all entries at once).
#[track_caller]
pub fn read_dir(path: &Path, opts: &ListOptions) -> FsResult<Vec<EntryInfo>> {
    let _guard = crate::guard::check("read_dir");
    let mut out = Vec::new();
    for e in imp::open_dir(path, opts).map_err(err::map("read_dir", path))? {
        let e = e.map_err(err::map("read_dir", path))?;
        if opts.include_hidden || !e.is_hidden() {
            out.push(e);
        }
    }
    Ok(out)
}

/// Streams chunks; `sink` returns `Control::Stop` to abort; `ctl.checkpoint()` between
/// chunks. Uses FindFirstFileExW(LARGE_FETCH) / getdents+statx / readdir+fstatat.
/// Unreadable individual entries are skipped (logged at debug level).
#[track_caller]
pub fn read_dir_streaming(
    path: &Path,
    opts: &ListOptions,
    ctl: &OpControl,
    sink: &mut dyn FnMut(Vec<EntryInfo>) -> Control,
) -> FsResult<ListStats> {
    let _guard = crate::guard::check("read_dir_streaming");
    let start = Instant::now();
    let chunk = opts.chunk.max(1);
    let mut stats = ListStats::default();
    ctl.checkpoint()?;
    let iter = imp::open_dir(path, opts).map_err(err::map("read_dir", path))?;
    let mut buf = Vec::with_capacity(chunk.min(4096));
    for e in iter {
        let e = match e {
            Ok(e) => e,
            Err(e) => {
                log::debug!("read_dir entry error in {}: {e}", path.display());
                continue;
            }
        };
        if !opts.include_hidden && e.is_hidden() {
            continue;
        }
        buf.push(e);
        if buf.len() >= chunk {
            ctl.checkpoint()?;
            stats.entries += buf.len();
            if sink(std::mem::replace(&mut buf, Vec::with_capacity(chunk.min(4096)))) == Control::Stop {
                stats.truncated = true;
                stats.elapsed = start.elapsed();
                return Ok(stats);
            }
        }
    }
    if !buf.is_empty() {
        ctl.checkpoint()?;
        stats.entries += buf.len();
        if sink(buf) == Control::Stop {
            stats.truncated = true;
        }
    }
    stats.elapsed = start.elapsed();
    Ok(stats)
}

/// Metadata of one path (`follow` = resolve symlinks; a dangling link reports itself).
#[track_caller]
pub fn stat(path: &Path, follow: bool) -> FsResult<EntryInfo> {
    let _guard = crate::guard::check("stat");
    imp::stat(path, follow).map_err(err::map("stat", path))
}

/// True if something (including a dangling symlink) exists at `path`.
#[track_caller]
pub fn exists_nofollow(path: &Path) -> bool {
    let _guard = crate::guard::check("exists_nofollow");
    imp::exists_nofollow(path)
}

/// Options for [`walk`].
#[derive(Clone, Debug, Default)]
pub struct WalkOptions {
    /// Descend into symlinked directories (never by default).
    pub follow_symlinks: bool,
    /// Emit [`WalkItem::DirDone`] after a directory's children.
    pub post_order: bool,
    /// Maximum depth to descend to (root = 0); `None` = unlimited.
    pub max_depth: Option<usize>,
    /// Do not cross into other devices.
    pub same_device: bool,
}

/// One item produced by [`walk`]. The root itself is emitted first, at depth 0.
#[derive(Debug)]
pub enum WalkItem {
    Entry {
        entry: EntryInfo,
        depth: usize,
    },
    /// Post-order notification (only with `post_order`), emitted for every directory
    /// entry after its children (or right after it when it was not descended into).
    DirDone {
        path: PathBuf,
        depth: usize,
    },
    Error {
        path: PathBuf,
        error: FsError,
    },
}

/// Totals of a walk.
#[derive(Clone, Copy, Debug, Default)]
pub struct WalkStats {
    pub files: u64,
    pub dirs: u64,
    pub links: u64,
    pub bytes: u64,
    pub errors: u64,
}

struct Frame {
    path: PathBuf,
    depth: usize,
    iter: std::vec::IntoIter<EntryInfo>,
    /// Canonical path, only tracked when following symlinks (loop detection).
    canon: Option<PathBuf>,
}

/// Name-surrogate reparse bit (`IsReparseTagNameSurrogate`): the reparse point stands
/// for another named entity (a link), as opposed to e.g. cloud-file placeholders.
const REPARSE_NAME_SURROGATE: u32 = 0x2000_0000;

/// A directory whose reparse point is a link of a type other than symlink / junction
/// (e.g. `LX_SYMLINK`, `WCI_LINK`): like a junction it is never descended or emptied,
/// only the link itself is removed. Always `false` off Windows.
pub(crate) fn is_link_dir(e: &EntryInfo) -> bool {
    // Symlink / mount-point tags on a `Dir` entry mean the link was followed on purpose
    // (`follow_symlinks`); unfollowed they are reported as `Symlink` / `Junction`.
    const SYMLINK: u32 = 0xA000_000C;
    const MOUNT_POINT: u32 = 0xA000_0003;
    matches!(e.kind, EntryKind::Dir)
        && e.reparse_tag.is_some_and(|t| t & REPARSE_NAME_SURROGATE != 0 && t != SYMLINK && t != MOUNT_POINT)
}

/// Iterative (explicit stack) tree walk used by delete/copy/size. Junctions (and other
/// link-type reparse directories, see [`is_link_dir`]) are never descended; symlinked directories only with `follow_symlinks` (with loop detection).
pub fn walk(
    root: &Path,
    opts: &WalkOptions,
    ctl: &OpControl,
    sink: &mut dyn FnMut(WalkItem) -> Control,
) -> FsResult<WalkStats> {
    let mut stats = WalkStats::default();
    let mut root_entry = stat(root, false).map_err(|e| FsError { op: "walk".into(), ..e })?;
    if opts.follow_symlinks
        && matches!(root_entry.kind, EntryKind::Symlink { .. })
        && let Ok(t) = stat(root, true)
    {
        root_entry = EntryInfo { path: root.to_path_buf(), ..t };
    }
    let root_dev = if opts.same_device { crate::copy::device_of(root).ok() } else { None };
    let list_opts = ListOptions {
        follow_symlinks: opts.follow_symlinks,
        include_hidden: true,
        stat_links: false,
        owners: false,
        chunk: 0,
    };
    let max_depth = opts.max_depth.unwrap_or(usize::MAX);
    let mut stack: Vec<Frame> = Vec::new();

    // Returns Ok(false) when the sink asked to stop.
    let visit = |entry: EntryInfo,
                 depth: usize,
                 stats: &mut WalkStats,
                 stack: &mut Vec<Frame>,
                 sink: &mut dyn FnMut(WalkItem) -> Control|
     -> FsResult<bool> {
        ctl.checkpoint()?;
        let path = entry.path.clone();
        let was_reparse = entry.attrs.contains(Attributes::REPARSE) || entry.reparse_tag.is_some();
        let descend_kind = match entry.kind {
            EntryKind::Dir => !is_link_dir(&entry),
            EntryKind::Symlink { target: LinkTarget::Dir } => opts.follow_symlinks,
            _ => false,
        };
        match entry.kind {
            EntryKind::Dir => stats.dirs += 1,
            EntryKind::Symlink { .. } | EntryKind::Junction => stats.links += 1,
            _ => {
                stats.files += 1;
                stats.bytes += entry.size;
            }
        }
        let entry_dev = entry.dev;
        if sink(WalkItem::Entry { entry, depth }) == Control::Stop {
            return Ok(false);
        }
        let mut descended = false;
        if descend_kind && depth < max_depth {
            let same_dev = match root_dev {
                None => true,
                Some(rd) => {
                    let dev = if was_reparse || entry_dev.is_none() && opts.follow_symlinks {
                        crate::copy::device_of(&path).ok()
                    } else {
                        entry_dev.or(Some(rd))
                    };
                    dev == Some(rd)
                }
            };
            let canon = if opts.follow_symlinks { std::fs::canonicalize(&path).ok() } else { None };
            let looped = canon.as_ref().is_some_and(|c| stack.iter().any(|f| f.canon.as_ref() == Some(c)));
            if looped {
                stats.errors += 1;
                let error = FsError::with_message(FsErrorKind::Loop, "walk", Some(&path), "symbolic link loop");
                if sink(WalkItem::Error { path: path.clone(), error }) == Control::Stop {
                    return Ok(false);
                }
            } else if same_dev {
                match read_dir(&path, &list_opts) {
                    Ok(children) => {
                        stack.push(Frame { path: path.clone(), depth, iter: children.into_iter(), canon });
                        descended = true;
                    }
                    Err(error) => {
                        stats.errors += 1;
                        let error = FsError { op: "walk".into(), ..error };
                        if sink(WalkItem::Error { path: path.clone(), error }) == Control::Stop {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        let is_dir_entry = descend_kind;
        if is_dir_entry && !descended && opts.post_order && sink(WalkItem::DirDone { path, depth }) == Control::Stop {
            return Ok(false);
        }
        Ok(true)
    };

    if !visit(root_entry, 0, &mut stats, &mut stack, sink)? {
        return Ok(stats);
    }
    while let Some(top) = stack.last_mut() {
        match top.iter.next() {
            None => {
                let done = stack.pop().expect("non-empty stack");
                if opts.post_order && sink(WalkItem::DirDone { path: done.path, depth: done.depth }) == Control::Stop {
                    return Ok(stats);
                }
            }
            Some(entry) => {
                let depth = top.depth + 1;
                if !visit(entry, depth, &mut stats, &mut stack, sink)? {
                    return Ok(stats);
                }
            }
        }
    }
    Ok(stats)
}

/// `.exe`-style extensions that Windows treats as executable.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_executable_ext(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && ["exe", "com", "bat", "cmd", "ps1", "msi"].iter().any(|x| x.eq_ignore_ascii_case(ext))
    })
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub(crate) mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    use soyuz_core::{Attributes, DeviceId, EntryInfo, EntryKind, LinkTarget};

    use super::ListOptions;
    use crate::sys::reparse::{IO_REPARSE_TAG_APPEXECLINK, IO_REPARSE_TAG_MOUNT_POINT, IO_REPARSE_TAG_SYMLINK};
    use crate::sys::windows::fs::{self as wfs, RawStat};
    use crate::sys::windows::ticks_to_system;

    const ATTR_MAP: [(u32, Attributes); 11] = [
        (0x1, Attributes::READONLY),
        (0x2, Attributes::HIDDEN),
        (0x4, Attributes::SYSTEM),
        (0x20, Attributes::ARCHIVE),
        (0x100, Attributes::TEMPORARY),
        (0x200, Attributes::SPARSE),
        (0x400, Attributes::REPARSE),
        (0x800, Attributes::COMPRESSED),
        (0x1000, Attributes::OFFLINE),
        (0x2000, Attributes::NOT_CONTENT_INDEXED),
        (0x4000, Attributes::ENCRYPTED),
    ];
    /// FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | PINNED-less cloud bits.
    const CLOUD_ATTRS: u32 = 0x0004_0000 | 0x0040_0000;

    pub(crate) fn attrs_from_raw(a: u32) -> Attributes {
        let mut out = Attributes::empty();
        for (bit, attr) in ATTR_MAP {
            if a & bit != 0 {
                out |= attr;
            }
        }
        if a & CLOUD_ATTRS != 0 {
            out |= Attributes::CLOUD_PLACEHOLDER;
        }
        out
    }

    /// Native attribute bits for the portable ones that `SetFileAttributesW` accepts.
    pub(crate) fn raw_from_attrs(a: Attributes) -> u32 {
        let mut out = 0;
        for (bit, attr) in ATTR_MAP {
            if a.contains(attr) && bit & 0x3127 != 0 {
                out |= bit;
            }
        }
        out
    }

    fn is_cloud_tag(tag: u32) -> bool {
        // IO_REPARSE_TAG_CLOUD, IO_REPARSE_TAG_CLOUD_1..F (0x9000x01A).
        tag & 0xFFFF_0FFF == 0x9000_001A
    }

    fn not_found(e: &io::Error) -> bool {
        matches!(e.raw_os_error(), Some(2 | 3 | 123 | 161 | 1921))
    }

    fn link_target(path: &Path, raw: &RawStat, stat_links: bool) -> LinkTarget {
        if !stat_links {
            return if raw.is_dir() { LinkTarget::Dir } else { LinkTarget::File };
        }
        match wfs::stat_path(path, true) {
            Ok(t) if t.is_dir() => LinkTarget::Dir,
            Ok(_) => LinkTarget::File,
            Err(e) if not_found(&e) => LinkTarget::Missing,
            Err(_) => LinkTarget::Unknown,
        }
    }

    pub(crate) fn kind_of(path: &Path, raw: &RawStat, stat_links: bool) -> EntryKind {
        let base = if raw.is_dir() { EntryKind::Dir } else { EntryKind::File };
        if !raw.is_reparse() {
            return base;
        }
        match raw.reparse_tag {
            IO_REPARSE_TAG_SYMLINK => EntryKind::Symlink { target: link_target(path, raw, stat_links) },
            IO_REPARSE_TAG_MOUNT_POINT => EntryKind::Junction,
            _ => base,
        }
    }

    pub(crate) fn entry_from_raw(path: PathBuf, raw: &RawStat, stat_links: bool) -> EntryInfo {
        let kind = kind_of(&path, raw, stat_links);
        let mut e = EntryInfo::minimal(path, kind);
        e.attrs = attrs_from_raw(raw.attrs);
        if raw.is_reparse() {
            e.reparse_tag = Some(raw.reparse_tag);
            if is_cloud_tag(raw.reparse_tag) {
                e.attrs |= Attributes::CLOUD_PLACEHOLDER;
            }
        }
        let file_like = matches!(e.kind, EntryKind::File);
        e.size = if file_like { raw.size } else { 0 };
        e.alloc_size = raw.alloc;
        e.modified = ticks_to_system(raw.written);
        e.created = ticks_to_system(raw.created);
        e.accessed = ticks_to_system(raw.accessed);
        e.nlink = raw.nlink;
        e.dev = raw.volume.map(DeviceId);
        e.inode = raw.file_index;
        if file_like && (super::windows_executable_ext(&e.name) || raw.reparse_tag == IO_REPARSE_TAG_APPEXECLINK) {
            e.attrs |= Attributes::EXECUTABLE;
        }
        e
    }

    fn finish(mut e: EntryInfo, raw: &RawStat, opts: &ListOptions) -> EntryInfo {
        if opts.follow_symlinks
            && raw.is_reparse()
            && matches!(e.kind, EntryKind::Symlink { .. } | EntryKind::Junction)
            && let Ok(t) = wfs::stat_path(&e.path, true)
        {
            let mut followed = entry_from_raw(e.path.clone(), &t, opts.stat_links);
            followed.attrs |= Attributes::REPARSE;
            followed.reparse_tag = e.reparse_tag;
            e = followed;
        }
        if opts.owners
            && let Ok((owner, group)) = crate::sys::windows::security::owner_group(&e.path)
        {
            e.owner = Some(owner);
            e.group = group;
        }
        e
    }

    /// `NtQueryDirectoryFile` with large buffers, or `FindFirstFileExW` when the file system
    /// refuses the information class.
    enum Inner {
        Nt(crate::sys::windows::ntdir::NtDirIter),
        Find(wfs::DirIter),
    }

    pub(crate) struct Iter {
        dir: PathBuf,
        inner: Inner,
        opts: ListOptions,
    }

    impl Iterator for Iter {
        type Item = io::Result<EntryInfo>;

        fn next(&mut self) -> Option<io::Result<EntryInfo>> {
            let next = match &mut self.inner {
                Inner::Nt(it) => it.next(),
                Inner::Find(it) => it.next(),
            };
            let fe = match next? {
                Ok(fe) => fe,
                Err(e) => return Some(Err(e)),
            };
            let path = self.dir.join(&fe.name);
            let e = entry_from_raw(path, &fe.stat, self.opts.stat_links);
            Some(Ok(finish(e, &fe.stat, &self.opts)))
        }
    }

    pub(crate) fn open_dir(path: &Path, opts: &ListOptions) -> io::Result<Iter> {
        use crate::sys::windows::ntdir::{self, OpenError};
        let inner = match ntdir::read_dir(path) {
            Ok(it) => Inner::Nt(it),
            Err(OpenError::Unsupported) => Inner::Find(wfs::read_dir(path)?),
            Err(OpenError::Io(e)) => return Err(e),
        };
        Ok(Iter { dir: path.to_path_buf(), inner, opts: opts.clone() })
    }

    pub(crate) fn stat(path: &Path, follow: bool) -> io::Result<EntryInfo> {
        let raw = match wfs::stat_path(path, follow) {
            Ok(r) => r,
            // A dangling link still has metadata of its own.
            Err(e) if follow && not_found(&e) => wfs::stat_path(path, false).map_err(|_| e)?,
            Err(e) => return Err(e),
        };
        Ok(entry_from_raw(path.to_path_buf(), &raw, true))
    }

    pub(crate) fn exists_nofollow(path: &Path) -> bool {
        match wfs::get_attributes(path) {
            Ok(_) => true,
            Err(e) => !not_found(&e) && wfs::find_single(path).is_ok(),
        }
    }
}

// ---------------------------------------------------------------------------
// Unix
// ---------------------------------------------------------------------------

#[cfg(unix)]
pub(crate) mod imp {
    use std::ffi::OsStr;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use rustix::fd::{AsFd, OwnedFd};
    use rustix::fs::{CWD, Dir, Mode, OFlags};
    use soyuz_core::{Attributes, DeviceId, EntryInfo, EntryKind, LinkTarget};

    use super::ListOptions;
    use crate::sys::unix::{self as us, UStat};

    pub(crate) fn kind_from_mode(mode: u32) -> EntryKind {
        match mode & 0o170000 {
            0o100000 => EntryKind::File,
            0o040000 => EntryKind::Dir,
            0o120000 => EntryKind::Symlink { target: LinkTarget::Unknown },
            0o060000 | 0o020000 => EntryKind::Device,
            0o010000 => EntryKind::Fifo,
            0o140000 => EntryKind::Socket,
            _ => EntryKind::Unknown,
        }
    }

    pub(crate) fn entry_from_ustat(
        path: PathBuf,
        st: &UStat,
        link_target: Option<LinkTarget>,
        owners: bool,
    ) -> EntryInfo {
        let mut kind = kind_from_mode(st.mode);
        if let (EntryKind::Symlink { .. }, Some(t)) = (kind, link_target) {
            kind = EntryKind::Symlink { target: t };
        }
        let mut e = EntryInfo::minimal(path, kind);
        let mode = st.mode;
        e.mode = Some(mode);
        e.size = if matches!(kind, EntryKind::File) { st.size } else { 0 };
        e.alloc_size = Some(st.blocks.saturating_mul(512));
        e.modified = st.mtime;
        e.created = st.btime;
        e.accessed = st.atime;
        e.nlink = u32::try_from(st.nlink).ok();
        e.dev = Some(DeviceId(st.dev));
        e.inode = Some(st.ino);
        e.attrs = st.extra;
        if mode & 0o222 == 0 {
            e.attrs |= Attributes::READONLY;
        }
        if matches!(kind, EntryKind::File) && mode & 0o111 != 0 {
            e.attrs |= Attributes::EXECUTABLE;
        }
        if mode & 0o4000 != 0 {
            e.attrs |= Attributes::SETUID;
        }
        if mode & 0o2000 != 0 {
            e.attrs |= Attributes::SETGID;
        }
        if mode & 0o1000 != 0 {
            e.attrs |= Attributes::STICKY;
        }
        if e.name.starts_with('.') {
            e.attrs |= Attributes::HIDDEN;
        }
        if matches!(kind, EntryKind::File) && st.blocks.saturating_mul(512) < st.size {
            e.attrs |= Attributes::SPARSE;
        }
        if owners {
            e.owner = us::user_name(st.uid);
            e.group = us::group_name(st.gid);
        }
        e
    }

    fn target_kind(dirfd: impl AsFd, name: &OsStr) -> LinkTarget {
        match us::ustat_at(dirfd, name, true) {
            Ok(t) if t.mode & 0o170000 == 0o040000 => LinkTarget::Dir,
            Ok(_) => LinkTarget::File,
            Err(e) if e.kind() == io::ErrorKind::NotFound => LinkTarget::Missing,
            Err(_) => LinkTarget::Unknown,
        }
    }

    fn build(dirfd: impl AsFd + Copy, name: &OsStr, path: PathBuf, opts: &ListOptions) -> io::Result<EntryInfo> {
        let mut st = us::ustat_at(dirfd, name, false)?;
        let is_link = st.mode & 0o170000 == 0o120000;
        if is_link
            && opts.follow_symlinks
            && let Ok(t) = us::ustat_at(dirfd, name, true)
        {
            st = t;
        }
        let lt = (is_link && st.mode & 0o170000 == 0o120000 && opts.stat_links).then(|| target_kind(dirfd, name));
        Ok(entry_from_ustat(path, &st, lt, opts.owners))
    }

    pub(crate) struct Iter {
        base: PathBuf,
        fd: OwnedFd,
        dir: Dir,
        opts: ListOptions,
    }

    impl Iterator for Iter {
        type Item = io::Result<EntryInfo>;

        fn next(&mut self) -> Option<io::Result<EntryInfo>> {
            loop {
                let ent = match self.dir.read()? {
                    Ok(ent) => ent,
                    Err(e) => return Some(Err(e.into())),
                };
                let name = OsStr::from_bytes(ent.file_name().to_bytes());
                if name == "." || name == ".." {
                    continue;
                }
                let path = self.base.join(name);
                return Some(build(self.fd.as_fd(), name, path, &self.opts));
            }
        }
    }

    pub(crate) fn open_dir(path: &Path, opts: &ListOptions) -> io::Result<Iter> {
        let fd = rustix::fs::openat(CWD, path, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())?;
        let dir = Dir::read_from(&fd)?;
        Ok(Iter { base: path.to_path_buf(), fd, dir, opts: opts.clone() })
    }

    pub(crate) fn stat(path: &Path, follow: bool) -> io::Result<EntryInfo> {
        let name = path.as_os_str();
        let st = match us::ustat_at(CWD, name, follow) {
            Ok(st) => st,
            Err(e) if follow && e.kind() == io::ErrorKind::NotFound => us::ustat_at(CWD, name, false).map_err(|_| e)?,
            Err(e) => return Err(e),
        };
        let lt = (st.mode & 0o170000 == 0o120000).then(|| target_kind(CWD, name));
        Ok(entry_from_ustat(path.to_path_buf(), &st, lt, true))
    }

    pub(crate) fn exists_nofollow(path: &Path) -> bool {
        us::ustat_at(CWD, path.as_os_str(), false).is_ok()
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) mod imp {
    use std::io;
    use std::path::Path;

    use soyuz_core::EntryInfo;

    use super::ListOptions;

    pub(crate) fn open_dir(_path: &Path, _opts: &ListOptions) -> io::Result<std::vec::IntoIter<io::Result<EntryInfo>>> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub(crate) fn stat(_path: &Path, _follow: bool) -> io::Result<EntryInfo> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub(crate) fn exists_nofollow(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }
}

/// Benchmark hooks (not a stable API).
#[doc(hidden)]
pub mod bench_support {
    use std::path::Path;

    /// Count entries with the raw enumerator only (no `EntryInfo` building). Windows:
    /// `ntdll` = `NtQueryDirectoryFile`, otherwise `FindFirstFileExW`. Elsewhere: `read_dir`.
    pub fn count_raw(path: &Path, ntdll: bool) -> std::io::Result<usize> {
        #[cfg(windows)]
        {
            use crate::sys::windows::{fs as wfs, ntdir};
            if ntdll {
                return match ntdir::read_dir(path) {
                    Ok(it) => it.map(|e| e.map(std::hint::black_box)).try_fold(0, |n, e| e.map(|_| n + 1)),
                    Err(ntdir::OpenError::Io(e)) => Err(e),
                    Err(ntdir::OpenError::Unsupported) => Err(std::io::ErrorKind::Unsupported.into()),
                };
            }
            wfs::read_dir(path)?.map(|e| e.map(std::hint::black_box)).try_fold(0, |n, e| e.map(|_| n + 1))
        }
        #[cfg(not(windows))]
        {
            let _ = ntdll;
            read_dir_find_first_file(path)
        }
    }

    /// Windows: list with the `FindFirstFileExW(LARGE_FETCH)` enumerator, the pre-ntdll
    /// path, building the same `EntryInfo`s. Elsewhere: same as [`super::read_dir`].
    pub fn read_dir_find_first_file(path: &Path) -> std::io::Result<usize> {
        #[cfg(windows)]
        {
            // Mirrors `read_dir` (collects the entries) so the two are comparable.
            let mut out = Vec::new();
            for fe in crate::sys::windows::fs::read_dir(path)? {
                let fe = fe?;
                out.push(super::imp::entry_from_raw(path.join(&fe.name), &fe.stat, true));
            }
            Ok(std::hint::black_box(out).len())
        }
        #[cfg(not(windows))]
        {
            super::read_dir(path, &super::ListOptions::default()).map(|v| v.len()).map_err(std::io::Error::other)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_extensions() {
        assert!(windows_executable_ext("setup.EXE"));
        assert!(windows_executable_ext("run.cmd"));
        assert!(!windows_executable_ext(".exe"));
        assert!(!windows_executable_ext("notes.txt"));
    }

    #[test]
    fn link_type_reparse_dirs_are_links() {
        let mut e = EntryInfo::minimal(PathBuf::from("d"), EntryKind::Dir);
        assert!(!is_link_dir(&e), "plain folder");
        e.reparse_tag = Some(0x9000_601A); // IO_REPARSE_TAG_CLOUD_6: a real folder (placeholder)
        assert!(!is_link_dir(&e));
        e.reparse_tag = Some(0xA000_001D); // IO_REPARSE_TAG_LX_SYMLINK: name surrogate
        assert!(is_link_dir(&e));
        e.reparse_tag = Some(0xA000_0027); // IO_REPARSE_TAG_WCI_LINK
        assert!(is_link_dir(&e));
        let f = EntryInfo { kind: EntryKind::File, ..e };
        assert!(!is_link_dir(&f), "files are deleted as files anyway");
    }

    #[test]
    fn walk_reports_listing_errors_and_stops() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(d.path().join(format!("f{i}")), b"x").unwrap();
        }
        let ctl = OpControl::new();
        let mut seen = 0;
        walk(d.path(), &WalkOptions::default(), &ctl, &mut |_| {
            seen += 1;
            if seen == 3 { Control::Stop } else { Control::Continue }
        })
        .unwrap();
        assert_eq!(seen, 3);
        assert_eq!(
            walk(&d.path().join("missing"), &WalkOptions::default(), &ctl, &mut |_| Control::Continue)
                .unwrap_err()
                .kind,
            FsErrorKind::NotFound
        );
    }
}
