//! File copy engine with progress, pause/cancel and per-OS fast paths.
//!
//! * Windows: ReFS / Dev Drive block cloning (`FSCTL_DUPLICATE_EXTENTS_TO_FILE`) when the
//!   volume supports it → else `CopyFileExW` with a progress callback that honours pause
//!   (blocks inside the callback) and cancel (`PROGRESS_CANCEL`); sparse files take a manual
//!   range copy so holes survive.
//! * Linux: `FICLONE` (btrfs/xfs reflink) → `copy_file_range` in `chunk`-sized steps →
//!   `read`/`write`; sparse files copy only `SEEK_DATA` extents. `Preserve::ATTRS` carries
//!   the inode flags (no-CoW / compression before the data, no-dump / no-atime after it,
//!   append-only / immutable last, once the copy has its final name); `Preserve::ACL`
//!   carries the `system.posix_acl_*` xattrs even without `Preserve::XATTR`.
//! * macOS: `fclonefileat` (APFS clone) → chunked `read`/`write` + `fcopyfile` for ACLs.
//!
//! Existing destinations are never truncated in place: the copy goes to a hidden sibling
//! temp file that atomically replaces the destination only after the data (and the
//! optional verification) succeeded, so a failed or cancelled overwrite keeps the old file.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use soyuz_core::{
    ByteProgress, DeviceId, EntryKind, FsError, FsErrorKind, FsResult, HashAlgo, OpControl, Overwrite, ProgressSink,
};

use crate::delete::DeleteOptions;
use crate::err;
use crate::links::LinkKind;
use crate::list::{exists_nofollow, stat};
use crate::times::TimesSet;

bitflags! {
    /// Metadata to carry over when copying.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct Preserve: u8 {
        const TIMES = 1;
        const ATTRS = 2;
        const XATTR = 4;
        const ACL = 8;
        const OWNER = 16;
        const SPARSE = 32;
    }
}

/// Copy-on-write cloning preference.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Reflink {
    #[default]
    Auto,
    Never,
    Always,
}

/// Options for [`copy_file`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CopyOptions {
    pub overwrite: Overwrite,
    pub preserve: Preserve,
    pub reflink: Reflink,
    /// Files at least this large bypass the page cache (default 256 MiB).
    pub unbuffered_threshold: u64,
    /// Chunk size for progress / checkpoints (default 8 MiB).
    pub chunk: usize,
    pub verify: Option<HashAlgo>,
    pub fsync: bool,
    pub follow_symlinks: bool,
    pub merge_dirs: bool,
    /// Test hook: sleep to cap throughput (honoured by baseline and raw implementations).
    pub throttle_bps: Option<u64>,
}

impl Default for CopyOptions {
    fn default() -> Self {
        CopyOptions {
            overwrite: Overwrite::Never,
            preserve: Preserve::TIMES | Preserve::ATTRS | Preserve::XATTR,
            reflink: Reflink::Auto,
            unbuffered_threshold: 256 * 1024 * 1024,
            chunk: 8 * 1024 * 1024,
            verify: None,
            fsync: false,
            follow_symlinks: false,
            merge_dirs: true,
            throttle_bps: None,
        }
    }
}

/// Why a copy was skipped by the overwrite policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Exists,
    Older,
    Smaller,
    SameSize,
    SameFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOutcome {
    Copied { bytes: u64, method: CopyMethod },
    Skipped(SkipReason),
}

/// How the bytes were copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMethod {
    Reflink,
    CopyFileEx,
    BlockClone,
    CopyFileRange,
    ReadWrite,
    Clonefile,
    Fcopyfile,
}

/// Progress is reported at least every `chunk` bytes; `ctl.checkpoint()` is honoured
/// between chunks (and inside the CopyFileExW callback on Windows). On cancel the partial
/// destination is removed and `Err(Interrupted)` returned.
pub fn copy_file(
    src: &Path,
    dst: &Path,
    opts: &CopyOptions,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> FsResult<CopyOutcome> {
    ctl.checkpoint()?;
    crate::fault::hit(crate::fault::FaultOp::Open, src).map_err(err::map("copy", src))?;
    let op_err = |e: FsError| FsError { op: "copy".into(), ..e };
    let src_l = stat(src, false).map_err(op_err)?;
    let is_link = matches!(src_l.kind, EntryKind::Symlink { .. } | EntryKind::Junction);
    if is_link && !opts.follow_symlinks {
        copy_symlink(src, dst, opts)?;
        return Ok(CopyOutcome::Copied { bytes: 0, method: CopyMethod::ReadWrite });
    }
    let s = if is_link { stat(src, true).map_err(op_err)? } else { src_l };
    match s.kind {
        EntryKind::File => {}
        EntryKind::Dir | EntryKind::Junction => return Err(FsError::new(FsErrorKind::IsDirectory, "copy", Some(src))),
        EntryKind::Symlink { .. } => {
            return Err(FsError::with_message(FsErrorKind::NotFound, "copy", Some(src), "dangling symbolic link"));
        }
        _ => {
            return Err(err::unsupported("copy", src, "special files (devices, FIFOs, sockets) cannot be copied"));
        }
    }

    let mut replacing = false;
    if exists_nofollow(dst) {
        if same_file(src, dst).unwrap_or(false) {
            return Ok(CopyOutcome::Skipped(SkipReason::SameFile));
        }
        let d = stat(dst, false).map_err(op_err)?;
        if matches!(d.kind, EntryKind::Dir | EntryKind::Junction) {
            return Err(FsError::new(FsErrorKind::IsDirectory, "copy", Some(dst)));
        }
        let skip = match opts.overwrite {
            Overwrite::Never => return Err(FsError::new(FsErrorKind::AlreadyExists, "copy", Some(dst))),
            Overwrite::Always => None,
            Overwrite::IfNewer => (s.modified <= d.modified).then_some(SkipReason::Older),
            Overwrite::IfLarger => (s.size <= d.size).then_some(SkipReason::Smaller),
            Overwrite::IfDifferentSize => (s.size == d.size).then_some(SkipReason::SameSize),
        };
        if let Some(reason) = skip {
            return Ok(CopyOutcome::Skipped(reason));
        }
        replacing = true;
    }

    // The data always goes to a hidden temp sibling first, so a crash or kill mid-copy
    // never leaves a truncated file under the final name (only a `.soyuz-part-` file the
    // recovery scan finds). Names too long for the temp suffix are written in place.
    let staged = replacing || temp_fits(dst);
    let target = if staged { temp_sibling(dst) } else { dst.to_path_buf() };
    let result = (|| -> FsResult<CopyMethod> {
        crate::fault::hit(crate::fault::FaultOp::Create, &target).map_err(err::map("copy", &target))?;
        let method = imp::copy_data(src, &target, &s, opts, ctl, sink)?;
        ctl.checkpoint()?;
        if let Some(algo) = opts.verify {
            let a = crate::hash::hash_file(src, algo, ctl, &mut ())?;
            let b = crate::hash::hash_file(&target, algo, ctl, &mut ())?;
            if a != b {
                return Err(FsError::with_message(
                    FsErrorKind::Io,
                    "copy",
                    Some(dst),
                    format!("verification failed: {} of the copy differs from the source", algo.label()),
                ));
            }
        }
        if replacing {
            replace_with(&target, dst)?;
        } else if staged {
            crate::mv::rename(&target, dst, crate::mv::RenameMode::NoReplace)
                .map_err(|e| FsError { op: "copy".into(), ..e })?;
        }
        imp::finish_final(src, dst, opts);
        Ok(method)
    })();
    match result {
        Ok(method) => Ok(CopyOutcome::Copied { bytes: s.size, method }),
        Err(e) => {
            // `AlreadyExists` here means someone else created the destination after our
            // check (exclusive create failed): that file is not ours to remove.
            if staged || e.kind != FsErrorKind::AlreadyExists {
                remove_partial(&target);
            }
            Err(e)
        }
    }
}

/// Marker inside the names of the copy engine's temp files (`.<name>.soyuz-part-<pid>-<n>`).
pub const PART_MARKER: &str = ".soyuz-part-";

/// Longest temp suffix: `.` + marker + pid (10 digits) + `-` + sequence (20 digits).
const TEMP_EXTRA: usize = 1 + PART_MARKER.len() + 10 + 1 + 20;

/// The temp sibling name of `dst` stays within the file-name limit: 255 UTF-16 units
/// (counted as characters) on Windows, `NAME_MAX` = 255 bytes on unix.
fn temp_fits(dst: &Path) -> bool {
    dst.file_name().is_some_and(|n| name_len(n) + TEMP_EXTRA <= 255)
}

#[cfg(unix)]
fn name_len(n: &std::ffi::OsStr) -> usize {
    use std::os::unix::ffi::OsStrExt;
    n.as_bytes().len()
}

#[cfg(not(unix))]
fn name_len(n: &std::ffi::OsStr) -> usize {
    n.to_string_lossy().chars().count()
}

/// `.<name>.soyuz-part-<pid>-<n>` next to `dst`, built from the raw name (never lossy, so
/// non-UTF-8 unix names keep their bytes).
fn temp_sibling(dst: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = std::ffi::OsString::from(".");
    if let Some(n) = dst.file_name() {
        name.push(n);
    }
    name.push(format!("{PART_MARKER}{}-{seq}", std::process::id()));
    dst.with_file_name(name)
}

/// Atomically move the finished temp copy over `dst`.
fn replace_with(tmp: &Path, dst: &Path) -> FsResult<()> {
    #[cfg(windows)]
    {
        // MoveFileExW cannot replace a read-only file: clear the flag, and put it back
        // when the rename fails (the old file stays, and must stay read-only).
        let mut cleared = false;
        if let Ok(e) = stat(dst, false)
            && e.attrs.contains(soyuz_core::Attributes::READONLY)
            && crate::attrs::set_attributes(
                dst,
                &crate::attrs::AttrSet {
                    set: soyuz_core::Attributes::empty(),
                    clear: soyuz_core::Attributes::READONLY,
                },
            )
            .is_ok()
        {
            cleared = true;
        }
        let r = crate::mv::rename(tmp, dst, crate::mv::RenameMode::Replace);
        if r.is_err() && cleared {
            let _ = crate::attrs::set_attributes(
                dst,
                &crate::attrs::AttrSet {
                    set: soyuz_core::Attributes::READONLY,
                    clear: soyuz_core::Attributes::empty(),
                },
            );
        }
        r.map_err(|e| FsError { op: "copy".into(), ..e })
    }
    #[cfg(not(windows))]
    {
        crate::mv::rename(tmp, dst, crate::mv::RenameMode::Replace).map_err(|e| FsError { op: "copy".into(), ..e })
    }
}

fn remove_partial(path: &Path) {
    if exists_nofollow(path) {
        let _ = crate::delete::delete_file(path, &DeleteOptions { clear_readonly: true, posix_semantics: true });
    }
}

/// Generic chunked read/write loop with progress, pause/cancel and throttling.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pump(
    input: &mut File,
    output: &mut File,
    total: u64,
    done_before: u64,
    opts: &CopyOptions,
    ctl: &OpControl,
    paths: (&Path, &Path),
    sink: &mut dyn ProgressSink,
) -> FsResult<u64> {
    let mut chunk = opts.chunk.clamp(64 * 1024, 64 * 1024 * 1024).min(8 * 1024 * 1024);
    if let Some(bps) = opts.throttle_bps {
        // ~20 progress updates per second under throttling.
        chunk = chunk.min((bps / 20).max(4096) as usize);
    }
    let mut buf = vec![0u8; chunk];
    let start = Instant::now();
    let mut done = done_before;
    let mut copied = 0u64;
    loop {
        ctl.checkpoint()?;
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(err::io("copy", paths.0, e)),
        };
        crate::fault::hit(crate::fault::FaultOp::Write, paths.1).map_err(err::map("copy", paths.1))?;
        output.write_all(&buf[..n]).map_err(err::map("copy", paths.1))?;
        done += n as u64;
        copied += n as u64;
        sink.on_progress(ByteProgress { done, total, delta: n as u64 });
        if let Some(bps) = opts.throttle_bps.filter(|b| *b > 0) {
            let expected = Duration::from_secs_f64(copied as f64 / bps as f64);
            let elapsed = start.elapsed();
            if expected > elapsed {
                std::thread::sleep(expected - elapsed);
            }
        }
    }
    Ok(done)
}

/// Recreate the symlink `src` at `dst` (never follows). Junctions are recreated as
/// junctions on Windows.
pub fn copy_symlink(src: &Path, dst: &Path, opts: &CopyOptions) -> FsResult<()> {
    let e = stat(src, false).map_err(|e| FsError { op: "copy_symlink".into(), ..e })?;
    let target = crate::links::read_link(src).map_err(|e| FsError { op: "copy_symlink".into(), ..e })?;
    if exists_nofollow(dst) {
        if opts.overwrite == Overwrite::Never {
            return Err(FsError::new(FsErrorKind::AlreadyExists, "copy_symlink", Some(dst)));
        }
        let d = stat(dst, false)?;
        match d.kind {
            EntryKind::Symlink { .. } | EntryKind::Junction => crate::delete::delete_link(dst)?,
            EntryKind::Dir => return Err(FsError::new(FsErrorKind::IsDirectory, "copy_symlink", Some(dst))),
            _ => crate::delete::delete_file(dst, &DeleteOptions { clear_readonly: true, posix_semantics: true })?,
        }
    }
    if e.kind == EntryKind::Junction {
        let abs = crate::links::resolve_link(src)?;
        return crate::links::junction(&abs, dst);
    }
    let kind = match e.kind {
        EntryKind::Symlink { target: soyuz_core::LinkTarget::Dir } => LinkKind::Dir,
        _ if cfg!(windows) && stat(src, true).is_ok_and(|t| t.kind == EntryKind::Dir) => LinkKind::Dir,
        _ => LinkKind::File,
    };
    crate::links::symlink(&target, dst, kind)?;
    if opts.preserve.contains(Preserve::TIMES) {
        let t = TimesSet { modified: e.modified, created: e.created, accessed: e.accessed };
        let _ = crate::times::set_times(dst, &t, false);
    }
    Ok(())
}

/// Copy directory metadata (times / attributes / mode / xattrs / owner) after its contents
/// were copied.
pub fn copy_dir_metadata(src: &Path, dst: &Path, preserve: Preserve) -> FsResult<()> {
    let s = stat(src, true).map_err(|e| FsError { op: "copy_dir_metadata".into(), ..e })?;
    imp::dir_metadata(src, dst, &s, preserve)?;
    if preserve.contains(Preserve::TIMES) {
        let t = TimesSet { modified: s.modified, created: s.created, accessed: s.accessed };
        crate::times::set_times(dst, &t, true)?;
    }
    Ok(())
}

/// Identity of a filesystem object: device (volume serial on Windows, `st_dev` on unix)
/// plus file id (128-bit `FILE_ID_INFO` on Windows, inode on unix). Two paths with equal
/// identities name the same object (hardlinks, case variants, aliases through links).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct FileIdentity {
    pub dev: u64,
    pub id: u128,
}

/// [`FileIdentity`] of `path` (`follow == false`: of a link itself, not its target).
#[track_caller]
pub fn file_identity(path: &Path, follow: bool) -> FsResult<FileIdentity> {
    let _guard = crate::guard::check("file_identity");
    #[cfg(windows)]
    {
        use crate::sys::windows::fs::file_id;
        let (dev, id) = file_id(path, follow).map_err(err::map("file_identity", path))?;
        Ok(FileIdentity { dev, id: u128::from_le_bytes(id) })
    }
    #[cfg(not(windows))]
    {
        let e = stat(path, follow).map_err(|e| FsError { op: "file_identity".into(), ..e })?;
        match (e.dev, e.inode) {
            (Some(d), Some(i)) => Ok(FileIdentity { dev: d.0, id: u128::from(i) }),
            _ => Err(FsError::unsupported("file_identity", Some(path))),
        }
    }
}

/// dev+inode / FileIdInfo equality (symlinks followed).
#[track_caller]
pub fn same_file(a: &Path, b: &Path) -> FsResult<bool> {
    let _guard = crate::guard::check("same_file");
    let ia = file_identity(a, true).map_err(|e| FsError { op: "same_file".into(), ..e })?;
    let ib = file_identity(b, true).map_err(|e| FsError { op: "same_file".into(), ..e })?;
    Ok(ia == ib)
}

/// Device (volume) of `path`: `st_dev` on unix, the volume serial number on Windows (the
/// same value [`crate::volumes::volumes`] reports).
#[track_caller]
pub fn device_of(path: &Path) -> FsResult<DeviceId> {
    let _guard = crate::guard::check("device_of");
    #[cfg(windows)]
    {
        use crate::sys::windows::{fs as wfs, volume};
        match wfs::open_meta(path, true).and_then(|h| wfs::stat_handle(h.raw())) {
            Ok(st) if st.volume.is_some() => Ok(DeviceId(st.volume.unwrap_or(0))),
            Ok(_) | Err(_) => {
                // Unopenable objects (locked files, dangling links): ask the volume.
                if !exists_nofollow(path) {
                    return Err(FsError::new(FsErrorKind::NotFound, "device_of", Some(path)));
                }
                let root = volume::volume_path_name(path).map_err(err::map("device_of", path))?;
                let vi = volume::volume_info(&root).map_err(err::map("device_of", path))?;
                Ok(DeviceId(u64::from(vi.serial)))
            }
        }
    }
    #[cfg(not(windows))]
    {
        let e = stat(path, true).map_err(|e| FsError { op: "device_of".into(), ..e })?;
        Ok(e.dev.unwrap_or(DeviceId(0)))
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    use soyuz_core::{Attributes, ByteProgress, EntryInfo, FsErrorKind, FsResult, OpControl, ProgressSink};

    use super::{CopyMethod, CopyOptions, Preserve, Reflink, pump};
    use crate::err;
    use crate::list::imp::raw_from_attrs;
    use crate::sys::windows::{fs as wfs, system_to_ticks, volume};

    const FILE_SUPPORTS_BLOCK_REFCOUNTING: u32 = 0x0800_0000;
    const COPY_FILE_NO_BUFFERING: u32 = 0x1000;
    const COPY_FILE_ALLOW_DECRYPTED_DESTINATION: u32 = 0x8;
    const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

    fn ticks(t: Option<std::time::SystemTime>) -> Option<u64> {
        t.map(system_to_ticks)
    }

    pub(super) fn copy_data(
        src: &Path,
        target: &Path,
        s: &EntryInfo,
        opts: &CopyOptions,
        ctl: &OpControl,
        sink: &mut dyn ProgressSink,
    ) -> FsResult<CopyMethod> {
        let size = s.size;
        let throttled = opts.throttle_bps.is_some();
        if !throttled && opts.reflink != Reflink::Never && size > 0 {
            match block_clone(src, target, s, opts, ctl, sink) {
                Ok(true) => {
                    finish_manual(src, target, s, opts)?;
                    return Ok(CopyMethod::BlockClone);
                }
                Ok(false) if opts.reflink == Reflink::Always => {
                    return Err(err::unsupported(
                        "copy",
                        target,
                        "block cloning is only available on ReFS / Dev Drive volumes (source and destination on the same volume)",
                    ));
                }
                Ok(false) => {}
                Err(e) if e.is_cancel() || opts.reflink == Reflink::Always || e.kind == FsErrorKind::AlreadyExists => {
                    return Err(e);
                }
                Err(e) => {
                    log::debug!("block clone failed, falling back to CopyFileExW: {e}");
                    super::remove_partial(target);
                }
            }
        }
        let sparse = s.attrs.contains(Attributes::SPARSE) && opts.preserve.contains(Preserve::SPARSE);
        if throttled || sparse {
            manual_copy(src, target, s, opts, ctl, sink, sparse)?;
            finish_manual(src, target, s, opts)?;
            return Ok(CopyMethod::ReadWrite);
        }

        let mut flags = COPY_FILE_ALLOW_DECRYPTED_DESTINATION;
        if size >= opts.unbuffered_threshold {
            flags |= COPY_FILE_NO_BUFFERING;
        }
        let mut last = 0u64;
        let mut cancelled = None;
        let res = wfs::copy_file_ex(src, target, flags, &mut |done, total| {
            // Blocks while paused (inside the callback, so CopyFileExW simply waits).
            if let Err(e) = ctl.checkpoint() {
                cancelled = Some(e);
                return false;
            }
            if done > last {
                if let Err(e) = crate::fault::hit(crate::fault::FaultOp::Write, target) {
                    cancelled = Some(err::io("copy", target, e));
                    return false;
                }
                sink.on_progress(ByteProgress { done, total, delta: done - last });
                last = done;
            }
            true
        });
        if let Some(e) = cancelled {
            return Err(e);
        }
        if let Err(e) = res {
            if wfs::is_unsupported_error(&e) {
                // Some redirectors reject CopyFileExW: copy by hand.
                super::remove_partial(target);
                manual_copy(src, target, s, opts, ctl, sink, false)?;
                finish_manual(src, target, s, opts)?;
                return Ok(CopyMethod::ReadWrite);
            }
            return Err(err::io("copy", src, e));
        }
        if last < size {
            sink.on_progress(ByteProgress { done: size, total: size, delta: size - last });
        }
        // CopyFileExW carries attributes, streams and times; re-apply times explicitly
        // (guards against the mtime-reset quirk) and honour a narrower `preserve`.
        let now = Some(system_to_ticks(std::time::SystemTime::now()));
        let times = if opts.preserve.contains(Preserve::TIMES) {
            wfs::set_times(target, true, ticks(s.created), ticks(s.accessed), ticks(s.modified))
        } else {
            wfs::set_times(target, true, now, now, now)
        };
        if let Err(e) = times {
            log::debug!("could not set times on {}: {e}", target.display());
        }
        if !opts.preserve.contains(Preserve::ATTRS) {
            let _ = wfs::set_attributes(target, FILE_ATTRIBUTE_NORMAL);
        }
        if opts.fsync {
            fsync(target)?;
        }
        Ok(CopyMethod::CopyFileEx)
    }

    fn fsync(target: &Path) -> FsResult<()> {
        match OpenOptions::new().write(true).open(target) {
            Ok(f) => f.sync_all().map_err(err::map("copy", target)),
            // Read-only copies cannot be opened for writing; their data was written by
            // CopyFileExW and is flushed by the filesystem.
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
            Err(e) => Err(err::io("copy", target, e)),
        }
    }

    fn open_src(src: &Path) -> FsResult<File> {
        OpenOptions::new()
            .read(true)
            .share_mode(wfs::SHARE_ALL)
            .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN)
            .open(src)
            .map_err(err::map("copy", src))
    }

    fn create_dst(target: &Path) -> FsResult<File> {
        OpenOptions::new().read(true).write(true).create_new(true).open(target).map_err(err::map("copy", target))
    }

    #[allow(clippy::too_many_arguments)]
    fn manual_copy(
        src: &Path,
        target: &Path,
        s: &EntryInfo,
        opts: &CopyOptions,
        ctl: &OpControl,
        sink: &mut dyn ProgressSink,
        sparse: bool,
    ) -> FsResult<()> {
        let mut input = open_src(src)?;
        let mut output = create_dst(target)?;
        if sparse && opts.throttle_bps.is_none() {
            crate::sparse::copy_sparse(&input, &output, s.size, opts.chunk, src, target, ctl, sink)?;
        } else {
            let done = pump(&mut input, &mut output, s.size, 0, opts, ctl, (src, target), sink)?;
            if done == 0 {
                sink.on_progress(ByteProgress { done: 0, total: 0, delta: 0 });
            }
        }
        if opts.fsync {
            output.sync_all().map_err(err::map("copy", target))?;
        }
        Ok(())
    }

    /// Streams, times and attributes for the manual paths (CopyFileExW does this itself).
    fn finish_manual(src: &Path, target: &Path, s: &EntryInfo, opts: &CopyOptions) -> FsResult<()> {
        if opts.preserve.contains(Preserve::XATTR)
            && let Err(e) = crate::xattr::copy_streams(src, target)
        {
            log::debug!("alternate data streams of {} not copied: {e}", src.display());
        }
        if opts.preserve.contains(Preserve::TIMES) {
            wfs::set_times(target, true, ticks(s.created), ticks(s.accessed), ticks(s.modified))
                .map_err(err::map("copy", target))?;
        }
        if opts.preserve.contains(Preserve::ATTRS) {
            let raw = raw_from_attrs(s.attrs);
            if raw != 0 {
                wfs::set_attributes(target, raw).map_err(err::map("copy", target))?;
            }
        }
        Ok(())
    }

    /// `Ok(false)` when block cloning is not possible here (different volumes, NTFS, ...).
    fn block_clone(
        src: &Path,
        target: &Path,
        s: &EntryInfo,
        opts: &CopyOptions,
        ctl: &OpControl,
        sink: &mut dyn ProgressSink,
    ) -> FsResult<bool> {
        let input = open_src(src)?;
        let Ok(vi) = volume::volume_info_handle(wfs::file_handle(&input)) else { return Ok(false) };
        if vi.flags & FILE_SUPPORTS_BLOCK_REFCOUNTING == 0 {
            return Ok(false);
        }
        let parent = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let same_volume = super::device_of(parent).is_ok_and(|d| d.0 == u64::from(vi.serial));
        if !same_volume {
            return Ok(false);
        }
        let root = volume::volume_path_name(src).map_err(err::map("copy", src))?;
        let cluster = volume::cluster_size(&root).map_err(err::map("copy", src))?.max(4096);
        let output = create_dst(target)?;
        if s.attrs.contains(Attributes::SPARSE) {
            wfs::set_sparse(&output, true).map_err(err::map("copy", target))?;
        }
        wfs::set_end_of_file(&output, s.size).map_err(err::map("copy", target))?;
        let step = (opts.chunk.max(1 << 20) as u64).div_ceil(cluster) * cluster;
        let step = step.max(cluster).min((1u64 << 31) / cluster * cluster);
        let mut off = 0u64;
        while off < s.size {
            ctl.checkpoint()?;
            let remaining = s.size - off;
            let len = if remaining <= step { remaining.div_ceil(cluster) * cluster } else { step };
            crate::fault::hit(crate::fault::FaultOp::Write, target).map_err(err::map("copy", target))?;
            wfs::duplicate_extents(&output, &input, off, off, len).map_err(err::map("copy", target))?;
            let advanced = len.min(remaining);
            off += advanced;
            sink.on_progress(ByteProgress { done: off, total: s.size, delta: advanced });
        }
        if opts.fsync {
            output.sync_all().map_err(err::map("copy", target))?;
        }
        Ok(true)
    }

    pub(super) fn finish_final(_src: &Path, _dst: &Path, _opts: &CopyOptions) {}

    pub(super) fn dir_metadata(src: &Path, dst: &Path, s: &EntryInfo, preserve: Preserve) -> FsResult<()> {
        if preserve.contains(Preserve::XATTR) {
            let _ = crate::xattr::copy_streams(src, dst);
        }
        if preserve.contains(Preserve::ATTRS) {
            // Keep the directory bit; add the settable portable bits.
            let raw = raw_from_attrs(s.attrs) | 0x10;
            wfs::set_attributes(dst, raw).map_err(err::map("copy_dir_metadata", dst))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Unix (Linux + macOS)
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::path::Path;

    use rustix::fs::{CWD, Mode, OFlags, Timestamps};
    use soyuz_core::{ByteProgress, EntryInfo, FsError, FsResult, OpControl, ProgressSink};

    use super::{CopyMethod, CopyOptions, Preserve, Reflink, pump};
    use crate::err;
    use crate::sys::unix::{self as us, UStat};

    fn open_src(src: &Path, follow: bool) -> FsResult<File> {
        let mut flags = OFlags::RDONLY | OFlags::CLOEXEC;
        if !follow {
            flags |= OFlags::NOFOLLOW;
        }
        rustix::fs::openat(CWD, src, flags, Mode::empty()).map(File::from).map_err(|e| err::io("copy", src, e.into()))
    }

    fn create_dst(target: &Path, mode: u32) -> FsResult<File> {
        // Owner-writable while we fill it; the final mode is applied with fchmod.
        let m = Mode::from_raw_mode(((mode & 0o777) | 0o600) as _);
        rustix::fs::openat(CWD, target, OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC, m)
            .map(File::from)
            .map_err(|e| err::io("copy", target, e.into()))
    }

    pub(super) fn copy_data(
        src: &Path,
        target: &Path,
        _s: &EntryInfo,
        opts: &CopyOptions,
        ctl: &OpControl,
        sink: &mut dyn ProgressSink,
    ) -> FsResult<CopyMethod> {
        let input = open_src(src, opts.follow_symlinks)?;
        let st = us::ustat_fd(&input).map_err(err::map("copy", src))?;

        #[cfg(target_os = "macos")]
        if opts.throttle_bps.is_none() && opts.reflink != Reflink::Never {
            match rustix::fs::fclonefileat(&input, CWD, target, rustix::fs::CloneFlags::empty()) {
                Ok(()) => {
                    sink.on_progress(ByteProgress { done: st.size, total: st.size, delta: st.size });
                    // The clone carries mode, flags, xattrs and times; apply what differs.
                    if !opts.preserve.contains(Preserve::TIMES) {
                        let now = std::time::SystemTime::now();
                        let _ = crate::times::set_times(
                            target,
                            &crate::times::TimesSet { modified: Some(now), accessed: Some(now), created: None },
                            false,
                        );
                    }
                    return Ok(CopyMethod::Clonefile);
                }
                Err(e) if opts.reflink == Reflink::Always => {
                    return Err(FsError::with_message(
                        soyuz_core::FsErrorKind::Unsupported,
                        "copy",
                        Some(target),
                        format!("cloning is only available on APFS within one volume ({e})"),
                    ));
                }
                Err(_) => {}
            }
        }

        let mut output = create_dst(target, st.mode)?;
        let mut input = input;
        // Inode flags that only work on an empty file (no-CoW, compression) go first.
        #[cfg(target_os = "linux")]
        let src_flags = if opts.preserve.contains(Preserve::ATTRS) && st.mode & 0o170000 == 0o100000 {
            rustix::fs::ioctl_getflags(&input).unwrap_or(rustix::fs::IFlags::empty())
        } else {
            rustix::fs::IFlags::empty()
        };
        #[cfg(target_os = "linux")]
        set_flags_best_effort(&output, src_flags & EARLY_FLAGS, target);
        let method = copy_bytes(&mut input, &mut output, &st, opts, ctl, sink, (src, target))?;
        apply_fd_metadata(&input, &output, &st, opts, target)?;
        #[cfg(target_os = "linux")]
        set_flags_best_effort(&output, src_flags & MIDDLE_FLAGS, target);
        if opts.fsync {
            output.sync_data().map_err(err::map("copy", target))?;
        }
        #[cfg(target_os = "linux")]
        if st.size >= opts.unbuffered_threshold {
            use rustix::fs::{Advice, fadvise};
            let _ = fadvise(&input, 0, None, Advice::DontNeed);
            let _ = fadvise(&output, 0, None, Advice::DontNeed);
        }
        Ok(method)
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_bytes(
        input: &mut File,
        output: &mut File,
        st: &UStat,
        opts: &CopyOptions,
        ctl: &OpControl,
        sink: &mut dyn ProgressSink,
        paths: (&Path, &Path),
    ) -> FsResult<CopyMethod> {
        let size = st.size;
        let throttled = opts.throttle_bps.is_some();
        if throttled {
            pump(input, output, size, 0, opts, ctl, paths, sink)?;
            return Ok(CopyMethod::ReadWrite);
        }

        #[cfg(target_os = "linux")]
        if opts.reflink != Reflink::Never {
            crate::fault::hit(crate::fault::FaultOp::Write, paths.1).map_err(err::map("copy", paths.1))?;
            match rustix::fs::ioctl_ficlone(&*output, &*input) {
                Ok(()) => {
                    sink.on_progress(ByteProgress { done: size, total: size, delta: size });
                    return Ok(CopyMethod::Reflink);
                }
                Err(e) if opts.reflink == Reflink::Always => {
                    return Err(FsError::with_message(
                        soyuz_core::FsErrorKind::Unsupported,
                        "copy",
                        Some(paths.1),
                        format!("reflinks need btrfs/xfs/bcachefs with both files on one filesystem ({e})"),
                    ));
                }
                Err(_) => {}
            }
        }
        #[cfg(not(target_os = "linux"))]
        if opts.reflink == Reflink::Always {
            return Err(FsError::with_message(
                soyuz_core::FsErrorKind::Unsupported,
                "copy",
                Some(paths.1),
                "copy-on-write cloning is not available for this file",
            ));
        }

        if opts.preserve.contains(Preserve::SPARSE) && crate::sparse::looks_sparse(Some(st.blocks * 512), size) {
            crate::sparse::copy_sparse(input, output, size, opts.chunk, paths.0, paths.1, ctl, sink)?;
            return Ok(CopyMethod::ReadWrite);
        }

        #[cfg(target_os = "linux")]
        {
            use rustix::fs::{FallocateFlags, copy_file_range, fallocate};
            use rustix::io::Errno;
            if size > 0 {
                let _ = fallocate(&*output, FallocateFlags::KEEP_SIZE, 0, size);
            }
            let chunk = opts.chunk.clamp(1 << 20, 1 << 30);
            let mut done = 0u64;
            let mut first = true;
            loop {
                ctl.checkpoint()?;
                crate::fault::hit(crate::fault::FaultOp::Write, paths.1).map_err(err::map("copy", paths.1))?;
                match copy_file_range(&*input, None, &*output, None, chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        first = false;
                        done += n as u64;
                        sink.on_progress(ByteProgress { done, total: size, delta: n as u64 });
                    }
                    Err(Errno::INTR) => continue,
                    Err(Errno::XDEV | Errno::NOSYS | Errno::INVAL | Errno::OPNOTSUPP | Errno::PERM | Errno::BADF)
                        if first =>
                    {
                        let d = pump(input, output, size, 0, opts, ctl, paths, sink)?;
                        finish_progress(d, size, sink);
                        return Ok(CopyMethod::ReadWrite);
                    }
                    Err(e) => return Err(err::io("copy", paths.1, e.into())),
                }
            }
            // copy_file_range may stop early on special filesystems: finish by hand.
            let d = pump(input, output, size, done, opts, ctl, paths, sink)?;
            finish_progress(d, size, sink);
            Ok(CopyMethod::CopyFileRange)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let d = pump(input, output, size, 0, opts, ctl, paths, sink)?;
            finish_progress(d, size, sink);
            Ok(CopyMethod::ReadWrite)
        }
    }

    /// Flags applied to the empty destination before any data (`FS_NOCOW_FL` only works on
    /// empty files; compression only affects data written afterwards).
    #[cfg(target_os = "linux")]
    const EARLY_FLAGS: rustix::fs::IFlags = rustix::fs::IFlags::NOCOW.union(rustix::fs::IFlags::COMPRESSED);
    /// Flags applied after the data and metadata were written.
    #[cfg(target_os = "linux")]
    const MIDDLE_FLAGS: rustix::fs::IFlags = rustix::fs::IFlags::NODUMP.union(rustix::fs::IFlags::NOATIME);
    /// Flags applied last, after the copy got its final name (an immutable or append-only
    /// file can no longer be renamed, have its times set, ...). They need
    /// CAP_LINUX_IMMUTABLE, so without it they are dropped (like `cp -a`).
    #[cfg(target_os = "linux")]
    const LATE_FLAGS: rustix::fs::IFlags = rustix::fs::IFlags::APPEND.union(rustix::fs::IFlags::IMMUTABLE);

    /// OR `flags` into the inode flags of `fd`; failures (filesystem without the flag,
    /// missing privilege) are logged, never fatal.
    #[cfg(target_os = "linux")]
    fn set_flags_best_effort(fd: impl rustix::fd::AsFd, flags: rustix::fs::IFlags, target: &Path) {
        if flags.is_empty() {
            return;
        }
        let res = rustix::fs::ioctl_getflags(&fd)
            .and_then(|cur| if cur.contains(flags) { Ok(()) } else { rustix::fs::ioctl_setflags(&fd, cur | flags) });
        if let Err(e) = res {
            log::debug!("inode flags {flags:?} not carried over to {}: {e}", target.display());
        }
    }

    /// Final step under `Preserve::ATTRS`: the append-only / immutable flags of `src` onto
    /// the finished copy `dst` (already under its final name).
    pub(super) fn finish_final(src: &Path, dst: &Path, opts: &CopyOptions) {
        #[cfg(target_os = "linux")]
        if opts.preserve.contains(Preserve::ATTRS) {
            let Ok(input) = open_src(src, opts.follow_symlinks) else { return };
            let Ok(flags) = rustix::fs::ioctl_getflags(&input) else { return };
            if flags.intersects(LATE_FLAGS) {
                let Ok(out) =
                    rustix::fs::openat(CWD, dst, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())
                else {
                    return;
                };
                set_flags_best_effort(&out, flags & LATE_FLAGS, dst);
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (src, dst, opts);
    }

    fn finish_progress(done: u64, size: u64, sink: &mut dyn ProgressSink) {
        if done == 0 && size == 0 {
            sink.on_progress(ByteProgress { done: 0, total: 0, delta: 0 });
        }
    }

    fn apply_fd_metadata(input: &File, output: &File, st: &UStat, opts: &CopyOptions, target: &Path) -> FsResult<()> {
        if opts.preserve.contains(Preserve::OWNER) {
            let _ = rustix::fs::fchown(
                output,
                Some(rustix::process::Uid::from_raw(st.uid)),
                Some(rustix::process::Gid::from_raw(st.gid)),
            );
        }
        // Permission bits always follow the source (like cp); set-id bits only with ATTRS.
        let mut mode = st.mode & 0o1777;
        if opts.preserve.contains(Preserve::ATTRS) {
            mode = st.mode & 0o7777;
        }
        rustix::fs::fchmod(output, Mode::from_raw_mode(mode as _)).map_err(|e| err::io("copy", target, e.into()))?;
        if opts.preserve.contains(Preserve::XATTR)
            && let Err(e) = crate::xattr::copy_fd(input, output)
        {
            log::debug!("extended attributes of {} not copied: {e}", target.display());
        }
        // POSIX ACLs are xattrs on Linux: carried by XATTR already, else on their own.
        #[cfg(target_os = "linux")]
        if opts.preserve.contains(Preserve::ACL)
            && !opts.preserve.contains(Preserve::XATTR)
            && let Err(e) = crate::xattr::copy_acl_fd(input, output)
        {
            log::debug!("ACL of {} not copied: {e}", target.display());
        }
        #[cfg(target_os = "macos")]
        {
            if opts.preserve.contains(Preserve::ACL) {
                crate::sys::macos::copy_acl(input, output);
            }
            if opts.preserve.contains(Preserve::ATTRS) {
                // Hidden flag only: copying immutable flags would make the copy undeletable.
                crate::sys::macos::set_fd_flags(output, st.flags & us::UF_HIDDEN);
            }
        }
        if opts.preserve.contains(Preserve::TIMES) {
            let times = Timestamps {
                last_access: us::to_timespec(st.atime.unwrap_or(std::time::UNIX_EPOCH)),
                last_modification: us::to_timespec(st.mtime.unwrap_or(std::time::UNIX_EPOCH)),
            };
            rustix::fs::futimens(output, &times).map_err(|e| err::io("copy", target, e.into()))?;
            #[cfg(target_os = "macos")]
            if let Some(b) = st.btime {
                let _ = crate::sys::macos::set_birthtime(target, b);
            }
        }
        Ok(())
    }

    pub(super) fn dir_metadata(src: &Path, dst: &Path, s: &EntryInfo, preserve: Preserve) -> FsResult<()> {
        if let Some(mode) = s.mode {
            let m = if preserve.contains(Preserve::ATTRS) { mode & 0o7777 } else { mode & 0o1777 };
            rustix::fs::chmodat(CWD, dst, Mode::from_raw_mode(m as _), rustix::fs::AtFlags::empty())
                .map_err(|e| err::io("copy_dir_metadata", dst, e.into()))?;
        }
        if preserve.contains(Preserve::XATTR) {
            let _ = crate::xattr::copy_path(src, dst);
        }
        #[cfg(target_os = "linux")]
        if preserve.contains(Preserve::ACL) && !preserve.contains(Preserve::XATTR) {
            let _ = crate::xattr::copy_acl_path(src, dst);
        }
        if preserve.contains(Preserve::OWNER)
            && let Ok(st) = us::ustat_at(CWD, src.as_os_str(), true)
        {
            let _ = rustix::fs::chownat(
                CWD,
                dst,
                Some(rustix::process::Uid::from_raw(st.uid)),
                Some(rustix::process::Gid::from_raw(st.gid)),
                rustix::fs::AtFlags::empty(),
            );
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::Path;

    use soyuz_core::{EntryInfo, FsResult, OpControl, ProgressSink};

    use super::{CopyMethod, CopyOptions, Preserve, pump};
    use crate::err;

    pub(super) fn copy_data(
        src: &Path,
        target: &Path,
        s: &EntryInfo,
        opts: &CopyOptions,
        ctl: &OpControl,
        sink: &mut dyn ProgressSink,
    ) -> FsResult<CopyMethod> {
        let mut input = std::fs::File::open(src).map_err(err::map("copy", src))?;
        let mut output = std::fs::File::create_new(target).map_err(err::map("copy", target))?;
        pump(&mut input, &mut output, s.size, 0, opts, ctl, (src, target), sink)?;
        Ok(CopyMethod::ReadWrite)
    }

    pub(super) fn dir_metadata(_src: &Path, _dst: &Path, _s: &EntryInfo, _p: Preserve) -> FsResult<()> {
        Ok(())
    }

    pub(super) fn finish_final(_src: &Path, _dst: &Path, _opts: &CopyOptions) {}
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn failed_replace_keeps_read_only_flag() {
        let d = tempfile::tempdir().unwrap();
        let dst = d.path().join("ro.txt");
        std::fs::write(&dst, b"old").unwrap();
        let mut perm = std::fs::metadata(&dst).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&dst, perm).unwrap();
        // The temp copy is missing: the rename fails after the flag was cleared.
        assert!(replace_with(&d.path().join("missing.part"), &dst).is_err());
        assert!(std::fs::metadata(&dst).unwrap().permissions().readonly(), "READONLY restored");
        assert_eq!(std::fs::read(&dst).unwrap(), b"old");
        let mut perm = std::fs::metadata(&dst).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(&dst, perm).unwrap();
    }

    #[test]
    fn identity_of_hardlinks_and_case_variants() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.txt");
        std::fs::write(&a, b"x").unwrap();
        let b = d.path().join("b.txt");
        std::fs::hard_link(&a, &b).unwrap();
        assert_eq!(file_identity(&a, false).unwrap(), file_identity(&b, false).unwrap());
        assert_eq!(file_identity(&a, false).unwrap(), file_identity(&d.path().join("A.TXT"), false).unwrap());
        let c = d.path().join("c.txt");
        std::fs::write(&c, b"x").unwrap();
        assert_ne!(file_identity(&a, false).unwrap(), file_identity(&c, false).unwrap());
    }
}
