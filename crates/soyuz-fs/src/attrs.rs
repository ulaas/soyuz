//! File attributes.
//!
//! * Windows: `SetFileAttributesW` for READONLY / HIDDEN / SYSTEM / ARCHIVE / TEMPORARY /
//!   OFFLINE / NOT_CONTENT_INDEXED, `FSCTL_SET_COMPRESSION` for COMPRESSED and
//!   `FSCTL_SET_SPARSE` for SPARSE.
//! * Unix: READONLY / EXECUTABLE / SETUID / SETGID / STICKY map to mode bits; IMMUTABLE is
//!   `FS_IOC_SETFLAGS` on Linux and `UF_IMMUTABLE` on macOS; HIDDEN is `UF_HIDDEN` on macOS
//!   (on Linux "hidden" means a leading dot, so it is a rename and reported `Unsupported`).
//! * Linux inode flags ([`InodeFlags`]: append-only, no-dump, no-atime, no-CoW, compress,
//!   immutable) are read and written with `FS_IOC_GETFLAGS` / `FS_IOC_SETFLAGS`
//!   ([`inode_flags`] / [`set_inode_flags`]); COMPRESSED and IMMUTABLE are also reachable
//!   through [`set_attributes`].

use std::path::Path;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use soyuz_core::{Attributes, FsError, FsErrorKind, FsResult};

bitflags! {
    /// Editable Linux inode flags (the `chattr` letters in the docs).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct InodeFlags: u32 {
        /// `a`: data can only be appended; no rename / delete (needs CAP_LINUX_IMMUTABLE).
        const APPEND = 1;
        /// `d`: skipped by dump-style backups.
        const NODUMP = 2;
        /// `A`: access times are not updated.
        const NOATIME = 4;
        /// `C`: no copy-on-write (btrfs; only takes effect on empty files, and on new
        /// files when set on a folder).
        const NOCOW = 8;
        /// `c`: transparently compressed (btrfs, ext4 with compression).
        const COMPRESS = 16;
        /// `i`: no change at all (needs CAP_LINUX_IMMUTABLE).
        const IMMUTABLE = 32;
    }
}

/// Linux inode flags of `path` (follows symlinks). `Unsupported` on other platforms and on
/// filesystems without `FS_IOC_GETFLAGS`.
#[track_caller]
pub fn inode_flags(path: &Path) -> FsResult<InodeFlags> {
    let _guard = crate::guard::check("inode_flags");
    #[cfg(target_os = "linux")]
    {
        let fd = iflags::open(path, "inode_flags")?;
        rustix::fs::ioctl_getflags(&fd)
            .map(iflags::from_raw)
            .map_err(|e| iflags::error("inode_flags", path, e, InodeFlags::empty()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "inode_flags",
            Some(path),
            "inode flags exist on Linux only",
        ))
    }
}

/// Set and clear Linux inode flags (`FS_IOC_SETFLAGS`). Flags the filesystem silently
/// drops are reported as `Unsupported`; `PermissionDenied` says whether
/// CAP_LINUX_IMMUTABLE (immutable / append-only) or ownership is missing.
#[track_caller]
pub fn set_inode_flags(path: &Path, set: InodeFlags, clear: InodeFlags) -> FsResult<()> {
    let _guard = crate::guard::check("set_inode_flags");
    if set.is_empty() && clear.is_empty() {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        iflags::apply(path, set, clear)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "set_inode_flags",
            Some(path),
            "inode flags exist on Linux only",
        ))
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod iflags {
    use std::path::Path;

    use rustix::fd::OwnedFd;
    use rustix::fs::{CWD, IFlags, Mode, OFlags};
    use rustix::io::Errno;
    use soyuz_core::{FsError, FsErrorKind, FsResult};

    use super::InodeFlags;
    use crate::err;

    const MAP: [(InodeFlags, IFlags); 6] = [
        (InodeFlags::APPEND, IFlags::APPEND),
        (InodeFlags::NODUMP, IFlags::NODUMP),
        (InodeFlags::NOATIME, IFlags::NOATIME),
        (InodeFlags::NOCOW, IFlags::NOCOW),
        (InodeFlags::COMPRESS, IFlags::COMPRESSED),
        (InodeFlags::IMMUTABLE, IFlags::IMMUTABLE),
    ];

    pub(crate) fn from_raw(f: IFlags) -> InodeFlags {
        MAP.iter().filter(|(_, k)| f.contains(*k)).fold(InodeFlags::empty(), |a, (o, _)| a | *o)
    }

    pub(crate) fn to_raw(f: InodeFlags) -> IFlags {
        MAP.iter().filter(|(o, _)| f.contains(*o)).fold(IFlags::empty(), |a, (_, k)| a | *k)
    }

    /// Open `path` for the flag ioctls. Regular files and directories only: opening a
    /// device node can have side effects (tape rewind, modem hang-up).
    pub(crate) fn open(path: &Path, op: &'static str) -> FsResult<OwnedFd> {
        let st = crate::sys::unix::ustat_at(CWD, path.as_os_str(), true).map_err(err::map(op, path))?;
        if !matches!(st.mode & 0o170000, 0o100000 | 0o040000) {
            return Err(err::unsupported(op, path, "inode flags apply to files and folders only"));
        }
        rustix::fs::openat(CWD, path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty())
            .map_err(|e| err::io(op, path, e.into()))
    }

    /// Readable error for a failed flag ioctl that changes `touched`.
    pub(crate) fn error(op: &'static str, path: &Path, e: Errno, touched: InodeFlags) -> FsError {
        match e {
            Errno::PERM | Errno::ACCESS => {
                let why = if touched.intersects(InodeFlags::IMMUTABLE | InodeFlags::APPEND) {
                    "changing the immutable or append-only flag needs CAP_LINUX_IMMUTABLE (root)"
                } else {
                    "only the owner (or root) can change these flags, and an immutable file must be made mutable first"
                };
                FsError::with_message(
                    FsErrorKind::PermissionDenied,
                    op,
                    Some(path),
                    format!("{why} (os error {})", e.raw_os_error()),
                )
            }
            Errno::NOTTY | Errno::OPNOTSUPP | Errno::INVAL | Errno::NOSYS => FsError::with_message(
                FsErrorKind::Unsupported,
                op,
                Some(path),
                format!("this filesystem does not support {} (os error {})", describe(touched), e.raw_os_error()),
            ),
            e => err::io(op, path, e.into()),
        }
    }

    fn describe(f: InodeFlags) -> String {
        if f.is_empty() {
            return "inode flags".into();
        }
        let names: Vec<&str> = f
            .iter_names()
            .map(|(n, _)| match n {
                "APPEND" => "append-only",
                "NODUMP" => "no-dump",
                "NOATIME" => "no-atime",
                "NOCOW" => "no-copy-on-write",
                "COMPRESS" => "compression",
                "IMMUTABLE" => "immutable",
                other => other,
            })
            .collect();
        format!("the {} flag", names.join(" / "))
    }

    pub(crate) fn apply(path: &Path, set: InodeFlags, clear: InodeFlags) -> FsResult<()> {
        const OP: &str = "set_inode_flags";
        let clear = clear - set;
        let fd = open(path, OP)?;
        let cur = rustix::fs::ioctl_getflags(&fd).map_err(|e| error(OP, path, e, set | clear))?;
        let new = (cur | to_raw(set)) - to_raw(clear);
        if new == cur {
            return Ok(());
        }
        rustix::fs::ioctl_setflags(&fd, new).map_err(|e| error(OP, path, e, set | clear))?;
        // Some filesystems accept the call but drop flags they do not implement.
        let got = rustix::fs::ioctl_getflags(&fd).map(from_raw).unwrap_or_else(|_| from_raw(new));
        let ignored = (set - got) | (clear & got);
        if !ignored.is_empty() {
            let mut msg = format!("the filesystem ignored {}", describe(ignored));
            if ignored.contains(InodeFlags::NOCOW) {
                msg.push_str(" (no-copy-on-write can only be set on empty files)");
            }
            return Err(FsError::with_message(FsErrorKind::Unsupported, OP, Some(path), msg));
        }
        Ok(())
    }
}

/// Attribute bits to set and to clear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AttrSet {
    pub set: Attributes,
    pub clear: Attributes,
}

/// Apply `a`; unsupported bits → `Err(Unsupported)` before anything is changed.
pub fn set_attributes(path: &Path, a: &AttrSet) -> FsResult<()> {
    let both = a.set | a.clear;
    if both.is_empty() {
        return Ok(());
    }
    let unsupported = both.difference(imp::SUPPORTED);
    if !unsupported.is_empty() {
        return Err(FsError::with_message(
            FsErrorKind::Unsupported,
            "set_attributes",
            Some(path),
            format!("these attributes cannot be changed on this platform: {unsupported:?}"),
        ));
    }
    imp::apply(path, a)
}

#[cfg(windows)]
mod imp {
    use std::fs::OpenOptions;
    use std::path::Path;

    use soyuz_core::{Attributes, FsResult};

    use super::AttrSet;
    use crate::err;
    use crate::list::imp::raw_from_attrs;
    use crate::sys::windows::fs as wfs;

    pub(super) const SUPPORTED: Attributes = Attributes::READONLY
        .union(Attributes::HIDDEN)
        .union(Attributes::SYSTEM)
        .union(Attributes::ARCHIVE)
        .union(Attributes::TEMPORARY)
        .union(Attributes::OFFLINE)
        .union(Attributes::NOT_CONTENT_INDEXED)
        .union(Attributes::COMPRESSED)
        .union(Attributes::SPARSE);
    const SETTABLE: u32 = 0x3127;

    pub(super) fn apply(path: &Path, a: &AttrSet) -> FsResult<()> {
        if a.set.contains(Attributes::COMPRESSED) || a.clear.contains(Attributes::COMPRESSED) {
            wfs::set_compression(path, a.set.contains(Attributes::COMPRESSED))
                .map_err(err::map("set_attributes", path))?;
        }
        if a.set.contains(Attributes::SPARSE) || a.clear.contains(Attributes::SPARSE) {
            let f = OpenOptions::new().read(true).write(true).open(path).map_err(err::map("set_attributes", path))?;
            wfs::set_sparse(&f, a.set.contains(Attributes::SPARSE)).map_err(err::map("set_attributes", path))?;
        }
        let current = wfs::get_attributes(path).map_err(err::map("set_attributes", path))?;
        let set = raw_from_attrs(a.set);
        let clear = raw_from_attrs(a.clear);
        if set == 0 && clear == 0 {
            return Ok(());
        }
        let new = ((current & SETTABLE) | set) & !clear;
        if new == current & SETTABLE {
            return Ok(());
        }
        wfs::set_attributes(path, if new == 0 { 0x80 } else { new }).map_err(err::map("set_attributes", path))
    }
}

#[cfg(unix)]
mod imp {
    use std::path::Path;

    use rustix::fs::{AtFlags, CWD, Mode};
    #[cfg(target_os = "linux")]
    use soyuz_core::FsError;
    use soyuz_core::{Attributes, FsResult};

    use super::AttrSet;
    use crate::err;
    use crate::sys::unix as us;

    const MODE_BITS: Attributes = Attributes::READONLY
        .union(Attributes::EXECUTABLE)
        .union(Attributes::SETUID)
        .union(Attributes::SETGID)
        .union(Attributes::STICKY);

    #[cfg(target_os = "macos")]
    pub(super) const SUPPORTED: Attributes = MODE_BITS.union(Attributes::IMMUTABLE).union(Attributes::HIDDEN);
    #[cfg(target_os = "linux")]
    pub(super) const SUPPORTED: Attributes = MODE_BITS.union(Attributes::IMMUTABLE).union(Attributes::COMPRESSED);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub(super) const SUPPORTED: Attributes = MODE_BITS.union(Attributes::IMMUTABLE);

    pub(super) fn apply(path: &Path, a: &AttrSet) -> FsResult<()> {
        let st = us::ustat_at(CWD, path.as_os_str(), true).map_err(err::map("set_attributes", path))?;
        let set_imm = a.set.contains(Attributes::IMMUTABLE);
        let clear_imm = a.clear.contains(Attributes::IMMUTABLE) && !set_imm;
        // Making an item mutable comes first (an immutable item refuses chmod), making it
        // immutable comes last.
        if clear_imm {
            set_immutable(path, false, st.flags)?;
        }
        if (a.set | a.clear).intersects(MODE_BITS) {
            let old = st.mode & 0o7777;
            let mut m = old;
            let toggle = |m: &mut u32, attr: Attributes, set_bits: u32, clear_bits: u32| {
                if a.set.contains(attr) {
                    *m = (*m & !clear_bits) | set_bits;
                } else if a.clear.contains(attr) {
                    *m &= !clear_bits;
                }
            };
            // READONLY: remove all write bits / restore owner write.
            if a.set.contains(Attributes::READONLY) {
                m &= !0o222;
            } else if a.clear.contains(Attributes::READONLY) {
                m |= 0o200;
            }
            // EXECUTABLE: +x wherever the item is readable.
            if a.set.contains(Attributes::EXECUTABLE) {
                m |= (m & 0o444) >> 2;
            } else if a.clear.contains(Attributes::EXECUTABLE) {
                m &= !0o111;
            }
            toggle(&mut m, Attributes::SETUID, 0o4000, 0o4000);
            toggle(&mut m, Attributes::SETGID, 0o2000, 0o2000);
            toggle(&mut m, Attributes::STICKY, 0o1000, 0o1000);
            if m != old {
                rustix::fs::chmodat(CWD, path, Mode::from_raw_mode(m as _), AtFlags::empty())
                    .map_err(|e| err::io("set_attributes", path, e.into()))?;
            }
        }
        #[cfg(target_os = "linux")]
        if (a.set | a.clear).contains(Attributes::COMPRESSED) {
            let c = super::InodeFlags::COMPRESS;
            let (s, cl) = if a.set.contains(Attributes::COMPRESSED) {
                (c, super::InodeFlags::empty())
            } else {
                (super::InodeFlags::empty(), c)
            };
            super::iflags::apply(path, s, cl).map_err(|e| FsError { op: "set_attributes".into(), ..e })?;
        }
        if set_imm {
            set_immutable(path, true, st.flags)?;
        }
        #[cfg(target_os = "macos")]
        if a.set.contains(Attributes::HIDDEN) || a.clear.contains(Attributes::HIDDEN) {
            let flags =
                if a.set.contains(Attributes::HIDDEN) { st.flags | us::UF_HIDDEN } else { st.flags & !us::UF_HIDDEN };
            crate::sys::macos::set_path_flags(path, flags).map_err(err::map("set_attributes", path))?;
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn set_immutable(path: &Path, on: bool, _flags: u32) -> FsResult<()> {
        let f = super::InodeFlags::IMMUTABLE;
        let (s, c) = if on { (f, super::InodeFlags::empty()) } else { (super::InodeFlags::empty(), f) };
        super::iflags::apply(path, s, c).map_err(|e| FsError { op: "set_attributes".into(), ..e })
    }

    #[cfg(target_os = "macos")]
    fn set_immutable(path: &Path, on: bool, flags: u32) -> FsResult<()> {
        let new = if on { flags | us::UF_IMMUTABLE } else { flags & !us::UF_IMMUTABLE };
        crate::sys::macos::set_path_flags(path, new).map_err(err::map("set_attributes", path))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn set_immutable(path: &Path, _on: bool, _flags: u32) -> FsResult<()> {
        Err(err::unsupported("set_attributes", path, "the immutable flag is not supported here"))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::Path;

    use soyuz_core::{Attributes, FsResult};

    use super::AttrSet;

    pub(super) const SUPPORTED: Attributes = Attributes::empty();

    pub(super) fn apply(_path: &Path, _a: &AttrSet) -> FsResult<()> {
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn inode_flags_round_trip_or_report_cleanly() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        std::fs::write(&f, b"x").unwrap();
        // tmpfs / btrfs / ext4 implement NOATIME and NODUMP; others must say Unsupported.
        match set_inode_flags(&f, InodeFlags::NOATIME | InodeFlags::NODUMP, InodeFlags::empty()) {
            Ok(()) => {
                let got = inode_flags(&f).unwrap();
                assert!(got.contains(InodeFlags::NOATIME | InodeFlags::NODUMP), "{got:?}");
                set_inode_flags(&f, InodeFlags::empty(), InodeFlags::NOATIME).unwrap();
                let got = inode_flags(&f).unwrap();
                assert!(!got.contains(InodeFlags::NOATIME) && got.contains(InodeFlags::NODUMP), "{got:?}");
            }
            Err(e) => assert_eq!(e.kind, FsErrorKind::Unsupported, "{e}"),
        }
        // Immutable / append-only need CAP_LINUX_IMMUTABLE: a clear explanation otherwise.
        if crate::sys::unix::euid() != 0 {
            let e = set_inode_flags(&f, InodeFlags::IMMUTABLE, InodeFlags::empty()).unwrap_err();
            assert!(matches!(e.kind, FsErrorKind::PermissionDenied | FsErrorKind::Unsupported), "{e}");
            if e.kind == FsErrorKind::PermissionDenied {
                assert!(e.message.contains("CAP_LINUX_IMMUTABLE"), "{e}");
            }
            let e =
                set_attributes(&f, &AttrSet { set: Attributes::IMMUTABLE, clear: Attributes::empty() }).unwrap_err();
            assert_eq!(e.op, "set_attributes");
        }
    }

    #[test]
    fn device_nodes_are_refused() {
        let e = inode_flags(Path::new("/dev/null")).unwrap_err();
        assert_eq!(e.kind, FsErrorKind::Unsupported);
    }
}
