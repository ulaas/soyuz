//! macOS shims for calls rustix does not wrap: `getfsstat`, `chflags`/`fchflags`,
//! `setattrlist(ATTR_CMN_CRTIME)` and the ACL part of `fcopyfile`; plus CoreFoundation /
//! LaunchServices / CoreText / AppKit glue in the submodules (all untested on macOS:
//! compile-checked from Linux only).

pub(crate) mod cf;
pub(crate) mod fonts;
pub(crate) mod launch;
pub(crate) mod pasteboard;

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub(crate) const MNT_RDONLY: u32 = libc::MNT_RDONLY as u32;
pub(crate) const MNT_LOCAL: u32 = libc::MNT_LOCAL as u32;
pub(crate) const MNT_DONTBROWSE: u32 = libc::MNT_DONTBROWSE as u32;

/// One mounted filesystem from `getfsstat`.
#[derive(Clone, Debug)]
pub(crate) struct MountEntry {
    pub mount: PathBuf,
    pub fstype: String,
    pub flags: u32,
    pub total: u64,
    pub free: u64,
}

fn cstr_field(chars: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// All mounts (`getfsstat(MNT_NOWAIT)`: cached statistics, never blocks on dead shares).
pub(crate) fn mounts() -> Vec<MountEntry> {
    // SAFETY: a null buffer asks for the number of mounted filesystems.
    let n = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if n <= 0 {
        return Vec::new();
    }
    let cap = n as usize + 8;
    // SAFETY: all-zero is a valid `statfs`.
    let mut buf: Vec<libc::statfs> = (0..cap).map(|_| unsafe { std::mem::zeroed() }).collect();
    let bytes = (cap * std::mem::size_of::<libc::statfs>()) as libc::c_int;
    // SAFETY: buffer of `cap` statfs entries, size passed in bytes.
    let got = unsafe { libc::getfsstat(buf.as_mut_ptr(), bytes, libc::MNT_NOWAIT) };
    if got <= 0 {
        return Vec::new();
    }
    buf.truncate(got as usize);
    buf.iter()
        .map(|s| {
            let bsize = u64::from(s.f_bsize);
            MountEntry {
                mount: PathBuf::from(cstr_field(&s.f_mntonname)),
                fstype: cstr_field(&s.f_fstypename),
                flags: s.f_flags,
                total: s.f_blocks.saturating_mul(bsize),
                free: s.f_bavail.saturating_mul(bsize),
            }
        })
        .collect()
}

fn cpath(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// `chflags(path, flags)` (follows symlinks).
pub(crate) fn set_path_flags(path: &Path, flags: u32) -> io::Result<()> {
    let c = cpath(path)?;
    // SAFETY: NUL-terminated path.
    if unsafe { libc::chflags(c.as_ptr(), flags) } == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// `fchflags(fd, flags)`; errors are ignored (best effort while copying).
pub(crate) fn set_fd_flags(f: &File, flags: u32) {
    // SAFETY: valid open file descriptor.
    unsafe {
        libc::fchflags(f.as_raw_fd(), flags);
    }
}

/// Birth (creation) time via `setattrlist(ATTR_CMN_CRTIME)`; symlinks are not followed.
pub(crate) fn set_birthtime(path: &Path, t: SystemTime) -> io::Result<()> {
    let c = cpath(path)?;
    let ts = crate::sys::unix::to_timespec(t);
    let mut spec = libc::timespec { tv_sec: ts.tv_sec as libc::time_t, tv_nsec: ts.tv_nsec as libc::c_long };
    // SAFETY: all-zero is a valid `attrlist`.
    let mut al: libc::attrlist = unsafe { std::mem::zeroed() };
    al.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    al.commonattr = libc::ATTR_CMN_CRTIME;
    // SAFETY: NUL-terminated path; attribute list and buffer (one timespec) of the size we pass.
    let rc = unsafe {
        libc::setattrlist(
            c.as_ptr(),
            (&raw mut al).cast(),
            (&raw mut spec).cast(),
            std::mem::size_of::<libc::timespec>(),
            libc::FSOPT_NOFOLLOW,
        )
    };
    if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// Copy the ACL of `src` to `dst` (`fcopyfile(COPYFILE_ACL)`); best effort.
pub(crate) fn copy_acl(src: &File, dst: &File) {
    let Ok(state) = rustix::fs::copyfile_state_alloc() else { return };
    // SAFETY: `state` was just allocated and is freed exactly once below.
    unsafe {
        let _ = rustix::fs::fcopyfile(src, dst, state, rustix::fs::CopyfileFlags::ACL);
        let _ = rustix::fs::copyfile_state_free(state);
    }
}
