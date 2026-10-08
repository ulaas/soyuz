//! Portable `std::fs` implementations of every public function (M0 baseline).
//!
//! Real behaviour, but no raw per-OS fast paths and no mid-file progress (except the
//! chunked loop used when `CopyOptions::throttle_bps` is set). Trash, junctions and ACL
//! views return `Unsupported`; free space is `Unsupported` (needs raw calls).
//! The per-OS implementation replaces these bodies module by module; until then the
//! public modules simply delegate here.

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use soyuz_core::{
    Attributes, ByteProgress, Control, DeviceId, EntryInfo, EntryKind, FsError, FsErrorKind, FsResult, HashAlgo,
    LinkTarget, OpControl, Overwrite, ProgressSink,
};

use crate::attrs::AttrSet;
use crate::copy::{CopyMethod, CopyOptions, CopyOutcome, Preserve, SkipReason};
use crate::delete::{DeleteOptions, DeleteStats};
use crate::launch::AppInfo;
use crate::links::LinkKind;
use crate::list::{ListOptions, ListStats, WalkItem, WalkOptions, WalkStats};
use crate::meta::MetaSnapshot;
use crate::mv::{MoveFast, MoveOptions, RenameMode};
use crate::perms::{AccessRights, SecurityInfo};
use crate::times::TimesSet;
use crate::trash::{TrashItem, TrashSupport};
use crate::volumes::{FsCaps, VolumeInfo, VolumeKind};

fn io_err<'a>(op: &'static str, path: &'a Path) -> impl FnOnce(std::io::Error) -> FsError + 'a {
    move |e| FsError::from_io(op, Some(path), &e)
}

// ---------------------------------------------------------------------------
// Metadata → EntryInfo
// ---------------------------------------------------------------------------

/// Build an [`EntryInfo`] from std metadata. `link_meta` is the `symlink_metadata` result;
/// `stat_links` resolves the target kind of symlinks.
pub(crate) fn entry_from_metadata(path: &Path, md: &Metadata, stat_links: bool) -> EntryInfo {
    let ft = md.file_type();
    let kind = if ft.is_symlink() {
        let target = if stat_links {
            match fs::metadata(path) {
                Ok(t) if t.is_dir() => LinkTarget::Dir,
                Ok(_) => LinkTarget::File,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => LinkTarget::Missing,
                Err(_) => LinkTarget::Unknown,
            }
        } else {
            link_target_hint(&ft)
        };
        EntryKind::Symlink { target }
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        other_kind(&ft)
    };
    let mut e = EntryInfo::minimal(path.to_path_buf(), kind);
    e.size = if ft.is_file() { md.len() } else { 0 };
    e.modified = md.modified().ok();
    e.created = md.created().ok();
    e.accessed = md.accessed().ok();
    fill_platform(&mut e, md);
    e
}

#[cfg(windows)]
fn link_target_hint(ft: &fs::FileType) -> LinkTarget {
    use std::os::windows::fs::FileTypeExt;
    if ft.is_symlink_dir() { LinkTarget::Dir } else { LinkTarget::File }
}

#[cfg(not(windows))]
fn link_target_hint(_ft: &fs::FileType) -> LinkTarget {
    LinkTarget::Unknown
}

#[cfg(unix)]
fn other_kind(ft: &fs::FileType) -> EntryKind {
    use std::os::unix::fs::FileTypeExt;
    if ft.is_block_device() || ft.is_char_device() {
        EntryKind::Device
    } else if ft.is_fifo() {
        EntryKind::Fifo
    } else if ft.is_socket() {
        EntryKind::Socket
    } else {
        EntryKind::Unknown
    }
}

#[cfg(not(unix))]
fn other_kind(_ft: &fs::FileType) -> EntryKind {
    EntryKind::Unknown
}

#[cfg(windows)]
fn fill_platform(e: &mut EntryInfo, md: &Metadata) {
    use std::os::windows::fs::MetadataExt;
    let a = md.file_attributes();
    let map: [(u32, Attributes); 11] = [
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
    for (bit, attr) in map {
        if a & bit != 0 {
            e.attrs |= attr;
        }
    }
    // FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
    if a & (0x40000 | 0x400000) != 0 {
        e.attrs |= Attributes::CLOUD_PLACEHOLDER;
    }
    if matches!(e.kind, EntryKind::File)
        && let Some(ext) = e.extension()
        && ["exe", "com", "bat", "cmd", "ps1", "msi"].iter().any(|x| x.eq_ignore_ascii_case(ext))
    {
        e.attrs |= Attributes::EXECUTABLE;
    }
}

#[cfg(unix)]
fn fill_platform(e: &mut EntryInfo, md: &Metadata) {
    use std::os::unix::fs::MetadataExt;
    let mode = md.mode();
    e.mode = Some(mode);
    e.nlink = u32::try_from(md.nlink()).ok();
    e.dev = Some(DeviceId(md.dev()));
    e.inode = Some(md.ino());
    e.alloc_size = Some(md.blocks().saturating_mul(512));
    if mode & 0o222 == 0 {
        e.attrs |= Attributes::READONLY;
    }
    if matches!(e.kind, EntryKind::File) && mode & 0o111 != 0 {
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
}

#[cfg(not(any(unix, windows)))]
fn fill_platform(_e: &mut EntryInfo, _md: &Metadata) {}

fn meta(path: &Path, follow: bool) -> std::io::Result<Metadata> {
    if follow {
        // A dangling symlink still has metadata of its own.
        fs::metadata(path).or_else(|_| fs::symlink_metadata(path))
    } else {
        fs::symlink_metadata(path)
    }
}

// ---------------------------------------------------------------------------
// list.rs
// ---------------------------------------------------------------------------

fn list_entry(path: PathBuf, opts: &ListOptions) -> EntryInfo {
    match meta(&path, opts.follow_symlinks) {
        Ok(md) => entry_from_metadata(&path, &md, opts.stat_links),
        Err(_) => EntryInfo::minimal(path, EntryKind::Unknown),
    }
}

pub fn read_dir(path: &Path, opts: &ListOptions) -> FsResult<Vec<EntryInfo>> {
    let mut out = Vec::new();
    for de in fs::read_dir(path).map_err(io_err("read_dir", path))? {
        let de = de.map_err(io_err("read_dir", path))?;
        let e = list_entry(de.path(), opts);
        if opts.include_hidden || !e.is_hidden() {
            out.push(e);
        }
    }
    Ok(out)
}

pub fn read_dir_streaming(
    path: &Path,
    opts: &ListOptions,
    ctl: &OpControl,
    sink: &mut dyn FnMut(Vec<EntryInfo>) -> Control,
) -> FsResult<ListStats> {
    let start = Instant::now();
    let chunk = opts.chunk.max(1);
    let mut stats = ListStats::default();
    let mut buf = Vec::with_capacity(chunk.min(4096));
    ctl.checkpoint()?;
    for de in fs::read_dir(path).map_err(io_err("read_dir", path))? {
        let de = match de {
            Ok(d) => d,
            Err(e) => {
                log::debug!("read_dir entry error in {}: {e}", path.display());
                continue;
            }
        };
        let e = list_entry(de.path(), opts);
        if !opts.include_hidden && e.is_hidden() {
            continue;
        }
        buf.push(e);
        if buf.len() >= chunk {
            ctl.checkpoint()?;
            stats.entries += buf.len();
            if sink(std::mem::take(&mut buf)) == Control::Stop {
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

pub fn stat(path: &Path, follow: bool) -> FsResult<EntryInfo> {
    let md = meta(path, follow).map_err(io_err("stat", path))?;
    Ok(entry_from_metadata(path, &md, true))
}

pub fn exists_nofollow(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn device_of_md(md: &Metadata) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(md.dev())
    }
    #[cfg(not(unix))]
    {
        let _ = md;
        None
    }
}

struct Frame {
    path: PathBuf,
    depth: usize,
    iter: fs::ReadDir,
}

pub fn walk(
    root: &Path,
    opts: &WalkOptions,
    ctl: &OpControl,
    sink: &mut dyn FnMut(WalkItem) -> Control,
) -> FsResult<WalkStats> {
    let mut stats = WalkStats::default();
    let root_md = fs::symlink_metadata(root).map_err(io_err("walk", root))?;
    let root_dev = device_of_md(&root_md);
    let max_depth = opts.max_depth.unwrap_or(usize::MAX);
    let mut stack: Vec<Frame> = Vec::new();

    // Returns Ok(false) when the sink asked to stop.
    let visit = |path: PathBuf,
                 md: Metadata,
                 depth: usize,
                 stats: &mut WalkStats,
                 stack: &mut Vec<Frame>,
                 sink: &mut dyn FnMut(WalkItem) -> Control|
     -> FsResult<bool> {
        ctl.checkpoint()?;
        let entry = entry_from_metadata(&path, &md, opts.follow_symlinks);
        let is_link = matches!(entry.kind, EntryKind::Symlink { .. } | EntryKind::Junction);
        let descend_kind = match entry.kind {
            EntryKind::Dir => true,
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
        let is_dir_entry = matches!(entry.kind, EntryKind::Dir) || (is_link && descend_kind);
        if sink(WalkItem::Entry { entry, depth }) == Control::Stop {
            return Ok(false);
        }
        let mut descended = false;
        if descend_kind && depth < max_depth {
            let same_dev = !opts.same_device || {
                let target_md = if is_link { fs::metadata(&path).ok() } else { Some(md) };
                target_md.and_then(|m| device_of_md(&m)) == root_dev
            };
            if same_dev {
                match fs::read_dir(&path) {
                    Ok(iter) => {
                        stack.push(Frame { path: path.clone(), depth, iter });
                        descended = true;
                    }
                    Err(e) => {
                        stats.errors += 1;
                        let error = FsError::from_io("walk", Some(&path), &e);
                        if sink(WalkItem::Error { path: path.clone(), error }) == Control::Stop {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        if is_dir_entry && !descended && opts.post_order && sink(WalkItem::DirDone { path, depth }) == Control::Stop {
            return Ok(false);
        }
        Ok(true)
    };

    let root_md = if opts.follow_symlinks { fs::metadata(root).unwrap_or(root_md) } else { root_md };
    if !visit(root.to_path_buf(), root_md, 0, &mut stats, &mut stack, sink)? {
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
            Some(Err(e)) => {
                stats.errors += 1;
                let path = top.path.clone();
                let error = FsError::from_io("walk", Some(&path), &e);
                if sink(WalkItem::Error { path, error }) == Control::Stop {
                    return Ok(stats);
                }
            }
            Some(Ok(de)) => {
                let depth = top.depth + 1;
                let path = de.path();
                match meta(&path, opts.follow_symlinks) {
                    Ok(md) => {
                        if !visit(path, md, depth, &mut stats, &mut stack, sink)? {
                            return Ok(stats);
                        }
                    }
                    Err(e) => {
                        stats.errors += 1;
                        let error = FsError::from_io("walk", Some(&path), &e);
                        if sink(WalkItem::Error { path, error }) == Control::Stop {
                            return Ok(stats);
                        }
                    }
                }
            }
        }
    }
    Ok(stats)
}

// ---------------------------------------------------------------------------
// copy.rs
// ---------------------------------------------------------------------------

pub fn copy_file(
    src: &Path,
    dst: &Path,
    opts: &CopyOptions,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> FsResult<CopyOutcome> {
    ctl.checkpoint()?;
    let src_link_md = fs::symlink_metadata(src).map_err(io_err("copy", src))?;
    if src_link_md.file_type().is_symlink() && !opts.follow_symlinks {
        copy_symlink(src, dst, opts)?;
        return Ok(CopyOutcome::Copied { bytes: 0, method: CopyMethod::ReadWrite });
    }
    let src_md = fs::metadata(src).map_err(io_err("copy", src))?;
    if src_md.is_dir() {
        return Err(FsError::new(FsErrorKind::IsDirectory, "copy", Some(src)));
    }
    if let Ok(dst_md) = fs::symlink_metadata(dst) {
        if same_file(src, dst).unwrap_or(false) {
            return Ok(CopyOutcome::Skipped(SkipReason::SameFile));
        }
        if dst_md.is_dir() {
            return Err(FsError::new(FsErrorKind::IsDirectory, "copy", Some(dst)));
        }
        let skip = match opts.overwrite {
            Overwrite::Never => return Err(FsError::new(FsErrorKind::AlreadyExists, "copy", Some(dst))),
            Overwrite::Always => None,
            Overwrite::IfNewer => (src_md.modified().ok() <= dst_md.modified().ok()).then_some(SkipReason::Older),
            Overwrite::IfLarger => (src_md.len() <= dst_md.len()).then_some(SkipReason::Smaller),
            Overwrite::IfDifferentSize => (src_md.len() == dst_md.len()).then_some(SkipReason::SameSize),
        };
        if let Some(reason) = skip {
            return Ok(CopyOutcome::Skipped(reason));
        }
        if dst_md.permissions().readonly() {
            let _ = clear_readonly(dst);
        }
    }

    let total = src_md.len();
    let method = if opts.throttle_bps.is_some() {
        copy_chunked(src, dst, total, opts, ctl, sink)?;
        CopyMethod::ReadWrite
    } else {
        fs::copy(src, dst).map_err(|e| {
            let _ = fs::remove_file(dst);
            FsError::from_io("copy", Some(src), &e)
        })?;
        sink.on_progress(ByteProgress { done: total, total, delta: total });
        std_copy_method()
    };

    if let Err(e) = ctl.checkpoint() {
        let _ = fs::remove_file(dst);
        return Err(e);
    }
    if opts.preserve.contains(Preserve::TIMES) {
        let t = TimesSet {
            modified: src_md.modified().ok(),
            created: src_md.created().ok(),
            accessed: src_md.accessed().ok(),
        };
        let _ = set_times(dst, &t, true);
    }
    if opts.fsync
        && let Ok(f) = OpenOptions::new().write(true).open(dst)
    {
        f.sync_all().map_err(io_err("copy", dst))?;
    }
    if let Some(algo) = opts.verify {
        let a = hash_file(src, algo, ctl, &mut ())?;
        let b = hash_file(dst, algo, ctl, &mut ())?;
        if a != b {
            return Err(FsError::with_message(
                FsErrorKind::Io,
                "copy",
                Some(dst),
                format!("verification failed ({} mismatch)", algo.label()),
            ));
        }
    }
    Ok(CopyOutcome::Copied { bytes: total, method })
}

fn std_copy_method() -> CopyMethod {
    if cfg!(windows) {
        CopyMethod::CopyFileEx
    } else if cfg!(target_os = "linux") {
        CopyMethod::CopyFileRange
    } else if cfg!(target_os = "macos") {
        CopyMethod::Fcopyfile
    } else {
        CopyMethod::ReadWrite
    }
}

/// Chunked copy with progress, pause/cancel and optional throttling.
fn copy_chunked(
    src: &Path,
    dst: &Path,
    total: u64,
    opts: &CopyOptions,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> FsResult<()> {
    let result = (|| -> FsResult<()> {
        let mut input = File::open(src).map_err(io_err("copy", src))?;
        let mut output = File::create(dst).map_err(io_err("copy", dst))?;
        let mut chunk = opts.chunk.clamp(4096, 64 * 1024 * 1024);
        if let Some(bps) = opts.throttle_bps {
            // ~20 progress updates per second under throttling.
            chunk = chunk.min((bps / 20).max(4096) as usize);
        }
        let mut buf = vec![0u8; chunk];
        let start = Instant::now();
        let mut done: u64 = 0;
        loop {
            ctl.checkpoint()?;
            let n = input.read(&mut buf).map_err(io_err("copy", src))?;
            if n == 0 {
                break;
            }
            output.write_all(&buf[..n]).map_err(io_err("copy", dst))?;
            done += n as u64;
            sink.on_progress(ByteProgress { done, total, delta: n as u64 });
            if let Some(bps) = opts.throttle_bps.filter(|b| *b > 0) {
                let expected = Duration::from_secs_f64(done as f64 / bps as f64);
                let elapsed = start.elapsed();
                if expected > elapsed {
                    std::thread::sleep(expected - elapsed);
                }
            }
        }
        output.flush().map_err(io_err("copy", dst))?;
        let perms = fs::metadata(src).map_err(io_err("copy", src))?.permissions();
        let _ = fs::set_permissions(dst, perms);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(dst);
    }
    result
}

pub fn copy_symlink(src: &Path, dst: &Path, opts: &CopyOptions) -> FsResult<()> {
    let target = fs::read_link(src).map_err(io_err("copy_symlink", src))?;
    if exists_nofollow(dst) {
        if opts.overwrite == Overwrite::Never {
            return Err(FsError::new(FsErrorKind::AlreadyExists, "copy_symlink", Some(dst)));
        }
        delete_link(dst).or_else(|_| delete_file(dst, &DeleteOptions::default()))?;
    }
    let kind = if fs::metadata(src).map(|m| m.is_dir()).unwrap_or(false) { LinkKind::Dir } else { LinkKind::File };
    symlink(&target, dst, kind)
}

pub fn copy_dir_metadata(src: &Path, dst: &Path, preserve: Preserve) -> FsResult<()> {
    let md = fs::metadata(src).map_err(io_err("copy_dir_metadata", src))?;
    if preserve.contains(Preserve::ATTRS) && md.permissions().readonly() {
        let _ = set_attributes(dst, &AttrSet { set: Attributes::READONLY, clear: Attributes::empty() });
    }
    if preserve.contains(Preserve::TIMES) {
        let t = TimesSet { modified: md.modified().ok(), created: md.created().ok(), accessed: md.accessed().ok() };
        set_times(dst, &t, true)?;
    }
    Ok(())
}

pub fn same_file(a: &Path, b: &Path) -> FsResult<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let ma = fs::metadata(a).map_err(io_err("same_file", a))?;
        let mb = fs::metadata(b).map_err(io_err("same_file", b))?;
        Ok(ma.dev() == mb.dev() && ma.ino() == mb.ino())
    }
    #[cfg(not(unix))]
    {
        let ca = fs::canonicalize(a).map_err(io_err("same_file", a))?;
        let cb = fs::canonicalize(b).map_err(io_err("same_file", b))?;
        Ok(ca.to_string_lossy().to_lowercase() == cb.to_string_lossy().to_lowercase())
    }
}

pub fn device_of(path: &Path) -> FsResult<DeviceId> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let md = fs::metadata(path).or_else(|_| fs::symlink_metadata(path)).map_err(io_err("device_of", path))?;
        Ok(DeviceId(md.dev()))
    }
    #[cfg(not(unix))]
    {
        // Baseline: hash of the volume root (drive letter / UNC share).
        use std::hash::{Hash, Hasher};
        fs::symlink_metadata(path).map_err(io_err("device_of", path))?;
        let root = volume_root(path)?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        root.to_string_lossy().to_lowercase().hash(&mut h);
        Ok(DeviceId(h.finish()))
    }
}

// ---------------------------------------------------------------------------
// mv.rs
// ---------------------------------------------------------------------------

pub fn rename(src: &Path, dst: &Path, mode: RenameMode) -> FsResult<()> {
    fs::symlink_metadata(src).map_err(io_err("rename", src))?;
    match mode {
        RenameMode::Exchange => return Err(FsError::unsupported("rename", Some(src))),
        RenameMode::NoReplace => {
            if exists_nofollow(dst) && !same_entry_case_variant(src, dst) {
                return Err(FsError::new(FsErrorKind::AlreadyExists, "rename", Some(dst)));
            }
        }
        RenameMode::Replace => {}
    }
    fs::rename(src, dst).map_err(io_err("rename", src))
}

/// `src` and `dst` name the same entry (case-only difference on a case-insensitive FS).
fn same_entry_case_variant(src: &Path, dst: &Path) -> bool {
    src.parent() == dst.parent()
        && src.file_name().map(|n| n.to_string_lossy().to_lowercase())
            == dst.file_name().map(|n| n.to_string_lossy().to_lowercase())
        && same_file(src, dst).unwrap_or(false)
}

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
    // Two-step through a temporary name works on every filesystem (incl. FAT).
    let tmp = parent.join(format!(".soyuz-rename-{}-{}", std::process::id(), nanos()));
    fs::rename(src, &tmp).map_err(io_err("rename", src))?;
    fs::rename(&tmp, &dst).map_err(|e| {
        let _ = fs::rename(&tmp, src);
        FsError::from_io("rename", Some(&dst), &e)
    })
}

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

// ---------------------------------------------------------------------------
// delete.rs
// ---------------------------------------------------------------------------

fn clear_readonly(path: &Path) -> FsResult<()> {
    set_attributes(path, &AttrSet { set: Attributes::empty(), clear: Attributes::READONLY })
}

pub fn delete_file(path: &Path, o: &DeleteOptions) -> FsResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && o.clear_readonly => {
            let readonly = fs::symlink_metadata(path).map(|m| m.permissions().readonly()).unwrap_or(false);
            if readonly {
                clear_readonly(path)?;
                fs::remove_file(path).map_err(io_err("delete", path))
            } else {
                Err(FsError::from_io("delete", Some(path), &e))
            }
        }
        Err(e) => Err(FsError::from_io("delete", Some(path), &e)),
    }
}

pub fn delete_empty_dir(path: &Path) -> FsResult<()> {
    fs::remove_dir(path).map_err(io_err("delete", path))
}

pub fn delete_link(path: &Path) -> FsResult<()> {
    let md = fs::symlink_metadata(path).map_err(io_err("delete_link", path))?;
    if !md.file_type().is_symlink() {
        return Err(FsError::with_message(FsErrorKind::Unsupported, "delete_link", Some(path), "not a link"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        if md.file_type().is_symlink_dir() {
            return fs::remove_dir(path).map_err(io_err("delete_link", path));
        }
        fs::remove_file(path).or_else(|_| fs::remove_dir(path)).map_err(io_err("delete_link", path))
    }
    #[cfg(not(windows))]
    {
        fs::remove_file(path).map_err(io_err("delete_link", path))
    }
}

pub fn remove_tree(
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
                let r = delete_empty_dir(&path);
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
    })?;
    Ok(stats)
}

pub fn secure_wipe(path: &Path, passes: u8, ctl: &OpControl, sink: &mut dyn ProgressSink) -> FsResult<()> {
    let passes = passes.max(1) as u64;
    let len = fs::metadata(path).map_err(io_err("secure_wipe", path))?.len();
    {
        let mut f = OpenOptions::new().write(true).open(path).map_err(io_err("secure_wipe", path))?;
        let total = len * passes;
        let mut done = 0u64;
        let mut buf = vec![0u8; 1 << 20];
        let mut seed = (nanos() as u64) | 1;
        for pass in 0..passes {
            use std::io::Seek;
            f.seek(std::io::SeekFrom::Start(0)).map_err(io_err("secure_wipe", path))?;
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
                f.write_all(&buf[..n]).map_err(io_err("secure_wipe", path))?;
                left -= n as u64;
                done += n as u64;
                sink.on_progress(ByteProgress { done, total, delta: n as u64 });
            }
            f.sync_all().map_err(io_err("secure_wipe", path))?;
        }
    }
    delete_file(path, &DeleteOptions { clear_readonly: true, posix_semantics: true })
}

// ---------------------------------------------------------------------------
// create.rs
// ---------------------------------------------------------------------------

pub fn create_dir(path: &Path) -> FsResult<()> {
    fs::create_dir(path).map_err(io_err("create_dir", path))
}

pub fn create_dir_all(path: &Path) -> FsResult<()> {
    fs::create_dir_all(path).map_err(io_err("create_dir", path))
}

pub fn create_file(path: &Path, exclusive: bool) -> FsResult<()> {
    let mut o = OpenOptions::new();
    o.write(true);
    if exclusive {
        o.create_new(true);
    } else {
        o.create(true);
    }
    o.open(path).map(drop).map_err(io_err("create_file", path))
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> FsResult<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let tmp = path.with_file_name(format!(".{name}.tmp-{}-{}", std::process::id(), nanos()));
    let res = (|| {
        let mut f = File::create(&tmp).map_err(io_err("write_atomic", &tmp))?;
        f.write_all(bytes).map_err(io_err("write_atomic", &tmp))?;
        f.sync_all().map_err(io_err("write_atomic", &tmp))?;
        drop(f);
        fs::rename(&tmp, path).map_err(io_err("write_atomic", path))
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

// ---------------------------------------------------------------------------
// links.rs
// ---------------------------------------------------------------------------

pub fn symlink(target: &Path, link: &Path, kind: LinkKind) -> FsResult<()> {
    #[cfg(unix)]
    let res = {
        let _ = kind;
        std::os::unix::fs::symlink(target, link)
    };
    #[cfg(windows)]
    let res = match kind {
        LinkKind::File => std::os::windows::fs::symlink_file(target, link),
        LinkKind::Dir => std::os::windows::fs::symlink_dir(target, link),
    };
    #[cfg(not(any(unix, windows)))]
    let res: std::io::Result<()> = {
        let _ = (target, kind);
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    };
    res.map_err(|e| {
        let mut fe = FsError::from_io("symlink", Some(link), &e);
        if cfg!(windows) && e.raw_os_error() == Some(1314) {
            fe.kind = FsErrorKind::PermissionDenied;
            fe.message = "creating symbolic links requires Developer Mode or elevation".into();
        }
        fe
    })
}

pub fn hardlink(src: &Path, dst: &Path) -> FsResult<()> {
    fs::hard_link(src, dst).map_err(io_err("hardlink", dst))
}

pub fn junction(_target: &Path, link: &Path) -> FsResult<()> {
    Err(FsError::unsupported("junction", Some(link)))
}

pub fn read_link(path: &Path) -> FsResult<PathBuf> {
    fs::read_link(path).map_err(io_err("read_link", path))
}

pub fn resolve_link(path: &Path) -> FsResult<PathBuf> {
    let target = crate::path::strip_verbatim(&read_link(path)?);
    let abs = if target.is_absolute() || target.has_root() {
        target
    } else {
        path.parent().unwrap_or(Path::new("")).join(target)
    };
    Ok(crate::path::normalize(&abs))
}

// ---------------------------------------------------------------------------
// times.rs / attrs.rs / perms.rs / meta.rs
// ---------------------------------------------------------------------------

pub fn set_times(path: &Path, t: &TimesSet, follow: bool) -> FsResult<()> {
    if t.modified.is_none() && t.accessed.is_none() && t.created.is_none() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{FileTimesExt, OpenOptionsExt};
        const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
        if !follow {
            flags |= FILE_FLAG_OPEN_REPARSE_POINT;
        }
        let f = OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(flags)
            .open(path)
            .map_err(io_err("set_times", path))?;
        let mut ft = fs::FileTimes::new();
        if let Some(m) = t.modified {
            ft = ft.set_modified(m);
        }
        if let Some(a) = t.accessed {
            ft = ft.set_accessed(a);
        }
        if let Some(c) = t.created {
            ft = ft.set_created(c);
        }
        f.set_times(ft).map_err(io_err("set_times", path))
    }
    #[cfg(unix)]
    {
        let md = fs::symlink_metadata(path).map_err(io_err("set_times", path))?;
        if md.file_type().is_symlink() && !follow {
            let mtime = t.modified.or_else(|| md.modified().ok()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let atime = t.accessed.or_else(|| md.accessed().ok()).unwrap_or(mtime);
            return filetime::set_symlink_file_times(
                path,
                filetime::FileTime::from_system_time(atime),
                filetime::FileTime::from_system_time(mtime),
            )
            .map_err(io_err("set_times", path));
        }
        let f = File::open(path).map_err(io_err("set_times", path))?;
        let mut ft = fs::FileTimes::new();
        if let Some(m) = t.modified {
            ft = ft.set_modified(m);
        }
        if let Some(a) = t.accessed {
            ft = ft.set_accessed(a);
        }
        f.set_times(ft).map_err(io_err("set_times", path))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = follow;
        Err(FsError::unsupported("set_times", Some(path)))
    }
}

pub fn set_attributes(path: &Path, a: &AttrSet) -> FsResult<()> {
    let supported = Attributes::READONLY;
    if !(a.set | a.clear).difference(supported).is_empty() {
        return Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "set_attributes",
            Some(path),
            "only the read-only attribute is supported by the baseline implementation",
        ));
    }
    let want_ro = if a.set.contains(Attributes::READONLY) {
        Some(true)
    } else if a.clear.contains(Attributes::READONLY) {
        Some(false)
    } else {
        None
    };
    let Some(ro) = want_ro else { return Ok(()) };
    let md = fs::metadata(path).map_err(io_err("set_attributes", path))?;
    let mut perms = md.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perms.mode();
        perms.set_mode(if ro { mode & !0o222 } else { mode | 0o200 });
    }
    #[cfg(not(unix))]
    {
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(ro);
    }
    fs::set_permissions(path, perms).map_err(io_err("set_attributes", path))
}

pub fn set_mode(path: &Path, mode: u32) -> FsResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io_err("set_mode", path))
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        Err(FsError::unsupported("set_mode", Some(path)))
    }
}

pub fn chown(path: &Path, uid: Option<u32>, gid: Option<u32>) -> FsResult<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::chown(path, uid, gid).map_err(io_err("chown", path))
    }
    #[cfg(not(unix))]
    {
        let _ = (uid, gid);
        Err(FsError::unsupported("chown", Some(path)))
    }
}

pub fn security_info(path: &Path) -> FsResult<SecurityInfo> {
    Err(FsError::unsupported("security_info", Some(path)))
}

pub fn effective_access(path: &Path) -> FsResult<AccessRights> {
    Err(FsError::unsupported("effective_access", Some(path)))
}

pub fn meta_snapshot(path: &Path) -> FsResult<MetaSnapshot> {
    let e = stat(path, false)?;
    Ok(MetaSnapshot {
        times: TimesSet { modified: e.modified, created: e.created, accessed: e.accessed },
        attrs: e.attrs,
        mode: e.mode,
    })
}

// ---------------------------------------------------------------------------
// volumes.rs
// ---------------------------------------------------------------------------

pub fn volumes() -> Vec<VolumeInfo> {
    let mut out = Vec::new();
    let mut push = |mount: PathBuf, label: String| {
        let device = device_of(&mount).unwrap_or(DeviceId(0));
        let read_only = fs::metadata(&mount).map(|m| m.permissions().readonly()).unwrap_or(false);
        out.push(VolumeInfo {
            mount,
            label,
            fs: String::new(),
            kind: VolumeKind::Unknown,
            total: 0,
            free: 0,
            read_only: read_only && !cfg!(windows),
            device,
            supports: FsCaps::HARDLINK | FsCaps::SYMLINK,
        });
    };
    #[cfg(windows)]
    {
        for letter in b'A'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", letter as char));
            if fs::metadata(&root).is_ok() {
                push(root, format!("Local Disk ({}:)", letter as char));
            }
        }
    }
    #[cfg(unix)]
    {
        push(PathBuf::from("/"), "System".into());
        let home = crate::path::home();
        if home != Path::new("/") {
            push(home, "Home".into());
        }
        #[cfg(target_os = "macos")]
        let mounts_dir = Some(PathBuf::from("/Volumes"));
        #[cfg(target_os = "linux")]
        let mounts_dir = std::env::var_os("USER").map(|u| PathBuf::from("/media").join(u));
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let mounts_dir: Option<PathBuf> = None;
        if let Some(dir) = mounts_dir
            && let Ok(rd) = fs::read_dir(dir)
        {
            for de in rd.flatten() {
                let p = de.path();
                if p.is_dir() {
                    let label = de.file_name().to_string_lossy().into_owned();
                    push(p, label);
                }
            }
        }
    }
    out
}

pub fn space(path: &Path) -> FsResult<(u64, u64)> {
    Err(FsError::with_message(
        FsErrorKind::Unsupported,
        "space",
        Some(path),
        "free space is not available in the baseline implementation",
    ))
}

pub fn volume_root(path: &Path) -> FsResult<PathBuf> {
    let abs = crate::path::normalize(path);
    #[cfg(windows)]
    {
        use std::path::Component;
        let mut out = PathBuf::new();
        for c in abs.components() {
            match c {
                Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
                _ => break,
            }
        }
        if out.as_os_str().is_empty() {
            return Err(FsError::new(FsErrorKind::NotFound, "volume_root", Some(path)));
        }
        if !out.has_root() {
            out.push("\\");
        }
        Ok(out)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mut cur = abs.clone();
        // Walk up to the first existing ancestor.
        while fs::metadata(&cur).is_err() {
            if !cur.pop() {
                return Err(FsError::new(FsErrorKind::NotFound, "volume_root", Some(path)));
            }
        }
        let dev = fs::metadata(&cur).map_err(io_err("volume_root", path))?.dev();
        loop {
            let Some(parent) = cur.parent().map(Path::to_path_buf) else { return Ok(cur) };
            match fs::metadata(&parent) {
                Ok(m) if m.dev() == dev => cur = parent,
                _ => return Ok(cur),
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(abs.ancestors().last().map(Path::to_path_buf).unwrap_or(abs))
    }
}

// ---------------------------------------------------------------------------
// trash (baseline: unsupported everywhere)
// ---------------------------------------------------------------------------

const TRASH_TODO: &str = "trash is not implemented yet (baseline)";

pub fn trash_support(_path: &Path) -> TrashSupport {
    TrashSupport::Unsupported(TRASH_TODO.into())
}

pub fn trash(path: &Path) -> FsResult<TrashItem> {
    Err(FsError::with_message(FsErrorKind::Unsupported, "trash", Some(path), TRASH_TODO))
}

pub fn trash_list() -> FsResult<Vec<TrashItem>> {
    Err(FsError::with_message(FsErrorKind::Unsupported, "trash_list", None, TRASH_TODO))
}

pub fn trash_restore(item: &TrashItem, _to: Option<&Path>) -> FsResult<PathBuf> {
    Err(FsError::with_message(FsErrorKind::Unsupported, "trash_restore", Some(&item.original), TRASH_TODO))
}

pub fn trash_purge(item: &TrashItem) -> FsResult<()> {
    Err(FsError::with_message(FsErrorKind::Unsupported, "trash_purge", Some(&item.original), TRASH_TODO))
}

pub fn trash_empty(_ctl: &OpControl, _on_item: &mut dyn FnMut(&TrashItem, FsResult<()>) -> Control) -> FsResult<()> {
    Err(FsError::with_message(FsErrorKind::Unsupported, "trash_empty", None, TRASH_TODO))
}

// ---------------------------------------------------------------------------
// hash.rs
// ---------------------------------------------------------------------------

pub fn hash_file(path: &Path, algo: HashAlgo, ctl: &OpControl, sink: &mut dyn ProgressSink) -> FsResult<String> {
    use sha1::Digest as _;
    let mut f = File::open(path).map_err(io_err("hash", path))?;
    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
    enum H {
        Blake3(Box<blake3::Hasher>),
        Sha256(sha2::Sha256),
        Sha1(sha1::Sha1),
        Md5(md5::Md5),
        Crc32(crc32fast::Hasher),
    }
    let mut h = match algo {
        HashAlgo::Blake3 => H::Blake3(Box::new(blake3::Hasher::new())),
        HashAlgo::Sha256 => H::Sha256(sha2::Sha256::new()),
        HashAlgo::Sha1 => H::Sha1(sha1::Sha1::new()),
        HashAlgo::Md5 => H::Md5(md5::Md5::new()),
        HashAlgo::Crc32 => H::Crc32(crc32fast::Hasher::new()),
    };
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        ctl.checkpoint()?;
        let n = f.read(&mut buf).map_err(io_err("hash", path))?;
        if n == 0 {
            break;
        }
        let data = &buf[..n];
        match &mut h {
            H::Blake3(x) => {
                x.update(data);
            }
            H::Sha256(x) => x.update(data),
            H::Sha1(x) => x.update(data),
            H::Md5(x) => x.update(data),
            H::Crc32(x) => x.update(data),
        }
        done += n as u64;
        sink.on_progress(ByteProgress { done, total, delta: n as u64 });
    }
    Ok(match h {
        H::Blake3(x) => x.finalize().to_hex().to_string(),
        H::Sha256(x) => hex(&x.finalize()),
        H::Sha1(x) => hex(&x.finalize()),
        H::Md5(x) => hex(&x.finalize()),
        H::Crc32(x) => format!("{:08x}", x.finalize()),
    })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ---------------------------------------------------------------------------
// launch
// ---------------------------------------------------------------------------

fn spawn_detached(mut cmd: Command, op: &'static str, path: &Path) -> FsResult<()> {
    match cmd.spawn() {
        Ok(mut child) => {
            // Reap in the background so no zombie is left behind on unix.
            std::thread::Builder::new()
                .name("soyuz-reap".into())
                .spawn(move || {
                    let _ = child.wait();
                })
                .map(drop)
                .map_err(io_err(op, path))
        }
        Err(e) => Err(FsError::from_io(op, Some(path), &e)),
    }
}

pub fn open_default(path: &Path) -> FsResult<()> {
    fs::symlink_metadata(path).map_err(io_err("open", path))?;
    #[cfg(windows)]
    let cmd = {
        let mut c = Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").arg(path);
        c
    };
    #[cfg(target_os = "macos")]
    let cmd = {
        let mut c = Command::new("open");
        c.arg(path);
        c
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    spawn_detached(cmd, "open", path)
}

pub fn open_with(path: &Path, app: &AppInfo) -> FsResult<()> {
    let mut cmd = Command::new(&app.exec);
    let p = path.to_string_lossy();
    let mut substituted = false;
    for a in &app.args {
        if a.contains("%1")
            || a.contains("%L")
            || a.contains("%f")
            || a.contains("%F")
            || a.contains("%u")
            || a.contains("%U")
        {
            substituted = true;
            let replaced = a
                .replace("%1", &p)
                .replace("%L", &p)
                .replace("%f", &p)
                .replace("%F", &p)
                .replace("%u", &p)
                .replace("%U", &p);
            cmd.arg(replaced);
        } else {
            cmd.arg(a);
        }
    }
    if !substituted {
        cmd.arg(path);
    }
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        cmd.current_dir(dir);
    }
    spawn_detached(cmd, "open_with", path)
}

pub fn apps_for(_path: &Path) -> Vec<AppInfo> {
    Vec::new()
}

pub fn open_terminal_at(dir: &Path, preferred: Option<&str>) -> FsResult<()> {
    let mut candidates: Vec<Command> = Vec::new();
    if let Some(p) = preferred.filter(|p| !p.trim().is_empty()) {
        let mut c = Command::new(p);
        c.current_dir(dir);
        candidates.push(c);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        let mut wt = Command::new("wt.exe");
        wt.arg("-d").arg(dir).current_dir(dir);
        candidates.push(wt);
        for exe in ["pwsh.exe", "powershell.exe", "cmd.exe"] {
            let mut c = Command::new(exe);
            c.current_dir(dir).creation_flags(CREATE_NEW_CONSOLE);
            candidates.push(c);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        c.arg("-a").arg("Terminal").arg(dir);
        candidates.push(c);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut names: Vec<String> = Vec::new();
        if let Ok(t) = std::env::var("TERMINAL") {
            names.push(t);
        }
        names.extend(
            ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "alacritty", "kitty", "xterm"]
                .map(String::from),
        );
        for n in names {
            let mut c = Command::new(n);
            c.current_dir(dir);
            candidates.push(c);
        }
    }
    let mut last_err = None;
    for cmd in candidates {
        match spawn_detached(cmd, "open_terminal", dir) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| FsError::unsupported("open_terminal", Some(dir))))
}

pub fn run_elevated(exe: &Path, args: &[String]) -> FsResult<()> {
    #[cfg(windows)]
    {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let mut script = format!("Start-Process -Verb RunAs -FilePath {}", quote(&exe.to_string_lossy()));
        if !args.is_empty() {
            let list: Vec<String> = args.iter().map(|a| quote(a)).collect();
            script.push_str(&format!(" -ArgumentList {}", list.join(",")));
        }
        let mut c = Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script]);
        spawn_detached(c, "run_elevated", exe)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut c = Command::new("pkexec");
        c.arg(exe).args(args);
        spawn_detached(c, "run_elevated", exe)
    }
    #[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
    {
        let _ = args;
        Err(FsError::unsupported("run_elevated", Some(exe)))
    }
}

pub fn reveal_in_system(path: &Path) -> FsResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("explorer.exe");
        c.raw_arg(format!("/select,\"{}\"", crate::path::strip_verbatim(path).display()));
        spawn_detached(c, "reveal", path)
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        spawn_detached(c, "reveal", path)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        let mut c = Command::new("xdg-open");
        c.arg(dir);
        spawn_detached(c, "reveal", path)
    }
}

pub fn shortcut_target(_path: &Path) -> FsResult<Option<PathBuf>> {
    Ok(None)
}
