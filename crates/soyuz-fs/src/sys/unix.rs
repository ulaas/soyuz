//! Unix shims shared by Linux and macOS: stat relative to a directory fd (statx on Linux)
//! and cached user/group name lookups (`getpwuid_r` / `getgrgid_r`, which rustix lacks).

use std::collections::HashMap;
use std::ffi::{CStr, OsStr};
use std::io;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rustix::fd::AsFd;
use rustix::fs::AtFlags;
use soyuz_core::Attributes;

/// Portable subset of `struct stat` / `struct statx`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct UStat {
    pub mode: u32,
    pub size: u64,
    pub blocks: u64,
    pub nlink: u64,
    pub uid: u32,
    pub gid: u32,
    pub ino: u64,
    pub dev: u64,
    pub atime: Option<SystemTime>,
    pub mtime: Option<SystemTime>,
    pub btime: Option<SystemTime>,
    /// Attributes only available from the native call (statx attributes, BSD `st_flags`).
    pub extra: Attributes,
    /// macOS `st_flags` (0 elsewhere).
    pub flags: u32,
}

pub(crate) fn ts(sec: i64, nsec: i64) -> Option<SystemTime> {
    let nsec = nsec.clamp(0, 999_999_999) as u32;
    if sec >= 0 {
        UNIX_EPOCH.checked_add(Duration::new(sec as u64, nsec))
    } else {
        UNIX_EPOCH.checked_sub(Duration::new(sec.unsigned_abs(), 0))?.checked_add(Duration::new(0, nsec))
    }
}

/// `SystemTime` → `(seconds, nanoseconds)` since the epoch.
pub(crate) fn to_timespec(t: SystemTime) -> rustix::fs::Timespec {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => rustix::fs::Timespec { tv_sec: d.as_secs() as i64, tv_nsec: d.subsec_nanos() as _ },
        Err(e) => {
            let d = e.duration();
            let (mut s, mut n) = (-(d.as_secs() as i64), -(d.subsec_nanos() as i64));
            if n < 0 {
                s -= 1;
                n += 1_000_000_000;
            }
            rustix::fs::Timespec { tv_sec: s, tv_nsec: n as _ }
        }
    }
}

fn at_flags(follow: bool) -> AtFlags {
    if follow { AtFlags::empty() } else { AtFlags::SYMLINK_NOFOLLOW }
}

#[cfg(target_os = "linux")]
mod statx_impl {
    use std::sync::atomic::{AtomicBool, Ordering};

    use rustix::fs::{StatxAttributes, StatxFlags};

    use super::*;

    static NO_STATX: AtomicBool = AtomicBool::new(false);

    pub(super) fn stat(dirfd: impl AsFd, name: &OsStr, flags: AtFlags) -> io::Result<UStat> {
        if !NO_STATX.load(Ordering::Relaxed) {
            match rustix::fs::statx(&dirfd, name, flags, StatxFlags::BASIC_STATS | StatxFlags::BTIME) {
                Ok(x) => {
                    let mut extra = Attributes::empty();
                    if x.stx_attributes.contains(StatxAttributes::IMMUTABLE) {
                        extra |= Attributes::IMMUTABLE;
                    }
                    if x.stx_attributes.contains(StatxAttributes::COMPRESSED) {
                        extra |= Attributes::COMPRESSED;
                    }
                    if x.stx_attributes.contains(StatxAttributes::ENCRYPTED) {
                        extra |= Attributes::ENCRYPTED;
                    }
                    let btime = (x.stx_mask & StatxFlags::BTIME.bits() != 0)
                        .then(|| ts(x.stx_btime.tv_sec, i64::from(x.stx_btime.tv_nsec)))
                        .flatten();
                    return Ok(UStat {
                        mode: u32::from(x.stx_mode),
                        size: x.stx_size,
                        blocks: x.stx_blocks,
                        nlink: u64::from(x.stx_nlink),
                        uid: x.stx_uid,
                        gid: x.stx_gid,
                        ino: x.stx_ino,
                        dev: rustix::fs::makedev(x.stx_dev_major, x.stx_dev_minor),
                        atime: ts(x.stx_atime.tv_sec, i64::from(x.stx_atime.tv_nsec)),
                        mtime: ts(x.stx_mtime.tv_sec, i64::from(x.stx_mtime.tv_nsec)),
                        btime,
                        extra,
                        flags: 0,
                    });
                }
                Err(rustix::io::Errno::NOSYS) | Err(rustix::io::Errno::PERM) => {
                    // No statx (old kernel / seccomp filter): use fstatat from now on.
                    NO_STATX.store(true, Ordering::Relaxed);
                }
                Err(e) => return Err(e.into()),
            }
        }
        let s = if flags.contains(AtFlags::EMPTY_PATH) {
            rustix::fs::fstat(&dirfd)?
        } else {
            rustix::fs::statat(&dirfd, name, flags)?
        };
        #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
        Ok(UStat {
            mode: s.st_mode as u32,
            size: s.st_size as u64,
            blocks: s.st_blocks as u64,
            nlink: s.st_nlink as u64,
            uid: s.st_uid as u32,
            gid: s.st_gid as u32,
            ino: s.st_ino as u64,
            dev: s.st_dev as u64,
            atime: ts(s.st_atime as i64, s.st_atime_nsec as i64),
            mtime: ts(s.st_mtime as i64, s.st_mtime_nsec as i64),
            btime: None,
            extra: Attributes::empty(),
            flags: 0,
        })
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn ustat_at(dirfd: impl AsFd, name: &OsStr, follow: bool) -> io::Result<UStat> {
    statx_impl::stat(dirfd, name, at_flags(follow))
}

/// Metadata of an open file.
#[cfg(target_os = "linux")]
pub(crate) fn ustat_fd(fd: impl AsFd) -> io::Result<UStat> {
    statx_impl::stat(fd, OsStr::new(""), AtFlags::EMPTY_PATH)
}

/// `st_flags` bits (BSD).
#[cfg(not(target_os = "linux"))]
pub(crate) const UF_IMMUTABLE: u32 = 0x2;
#[cfg(not(target_os = "linux"))]
pub(crate) const UF_COMPRESSED: u32 = 0x20;
#[cfg(not(target_os = "linux"))]
pub(crate) const UF_HIDDEN: u32 = 0x8000;
#[cfg(not(target_os = "linux"))]
pub(crate) const SF_IMMUTABLE: u32 = 0x0002_0000;

#[cfg(not(target_os = "linux"))]
pub(crate) fn ustat_at(dirfd: impl AsFd, name: &OsStr, follow: bool) -> io::Result<UStat> {
    Ok(from_stat(&rustix::fs::statat(&dirfd, name, at_flags(follow))?))
}

/// Metadata of an open file.
#[cfg(not(target_os = "linux"))]
pub(crate) fn ustat_fd(fd: impl AsFd) -> io::Result<UStat> {
    Ok(from_stat(&rustix::fs::fstat(&fd)?))
}

#[cfg(not(target_os = "linux"))]
fn from_stat(s: &rustix::fs::Stat) -> UStat {
    let flags = s.st_flags;
    let mut extra = Attributes::empty();
    if flags & (UF_IMMUTABLE | SF_IMMUTABLE) != 0 {
        extra |= Attributes::IMMUTABLE;
    }
    if flags & UF_COMPRESSED != 0 {
        extra |= Attributes::COMPRESSED;
    }
    if flags & UF_HIDDEN != 0 {
        extra |= Attributes::HIDDEN;
    }
    #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
    UStat {
        mode: s.st_mode as u32,
        size: s.st_size as u64,
        blocks: s.st_blocks as u64,
        nlink: s.st_nlink as u64,
        uid: s.st_uid as u32,
        gid: s.st_gid as u32,
        ino: s.st_ino as u64,
        dev: s.st_dev as u64,
        atime: ts(s.st_atime as i64, s.st_atime_nsec as i64),
        mtime: ts(s.st_mtime as i64, s.st_mtime_nsec as i64),
        btime: ts(s.st_birthtime as i64, s.st_birthtime_nsec as i64),
        extra,
        flags,
    }
}

type NameCache = Mutex<HashMap<u32, Option<Arc<str>>>>;

fn cached(
    cache: &'static OnceLock<NameCache>,
    id: u32,
    lookup: impl FnOnce(u32) -> Option<String>,
) -> Option<Arc<str>> {
    let c = cache.get_or_init(Default::default);
    if let Some(v) = c.lock().get(&id) {
        return v.clone();
    }
    let v: Option<Arc<str>> = Some(lookup(id).map(Arc::from).unwrap_or_else(|| Arc::from(id.to_string())));
    c.lock().insert(id, v.clone());
    v
}

/// Login name of `uid` (the number as text when unknown).
pub(crate) fn user_name(uid: u32) -> Option<Arc<str>> {
    static CACHE: OnceLock<NameCache> = OnceLock::new();
    cached(&CACHE, uid, |uid| {
        let mut buf = vec![0 as libc::c_char; 4096];
        for _ in 0..4 {
            // SAFETY: all-zero is a valid `passwd`.
            let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            // SAFETY: valid out-struct, scratch buffer of the length we pass, result pointer.
            let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut result) };
            if rc == libc::ERANGE {
                let n = buf.len() * 2;
                buf.resize(n, 0);
                continue;
            }
            if rc != 0 || result.is_null() || pw.pw_name.is_null() {
                return None;
            }
            // SAFETY: on success `pw_name` points at a NUL-terminated string inside `buf`.
            return Some(unsafe { CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned());
        }
        None
    })
}

/// Name of group `gid` (the number as text when unknown).
pub(crate) fn group_name(gid: u32) -> Option<Arc<str>> {
    static CACHE: OnceLock<NameCache> = OnceLock::new();
    cached(&CACHE, gid, |gid| {
        let mut buf = vec![0 as libc::c_char; 4096];
        for _ in 0..4 {
            // SAFETY: all-zero is a valid `group`.
            let mut gr: libc::group = unsafe { std::mem::zeroed() };
            let mut result: *mut libc::group = std::ptr::null_mut();
            // SAFETY: valid out-struct, scratch buffer of the length we pass, result pointer.
            let rc = unsafe { libc::getgrgid_r(gid, &mut gr, buf.as_mut_ptr(), buf.len(), &mut result) };
            if rc == libc::ERANGE {
                let n = buf.len() * 2;
                buf.resize(n, 0);
                continue;
            }
            if rc != 0 || result.is_null() || gr.gr_name.is_null() {
                return None;
            }
            // SAFETY: on success `gr_name` points at a NUL-terminated string inside `buf`.
            return Some(unsafe { CStr::from_ptr(gr.gr_name) }.to_string_lossy().into_owned());
        }
        None
    })
}

/// Effective uid of this process.
pub(crate) fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}
