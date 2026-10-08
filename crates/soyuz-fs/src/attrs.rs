//! File attributes.
//!
//! * Windows: `SetFileAttributesW` for READONLY / HIDDEN / SYSTEM / ARCHIVE / TEMPORARY /
//!   OFFLINE / NOT_CONTENT_INDEXED, `FSCTL_SET_COMPRESSION` for COMPRESSED and
//!   `FSCTL_SET_SPARSE` for SPARSE.
//! * Unix: READONLY / EXECUTABLE / SETUID / SETGID / STICKY map to mode bits; IMMUTABLE is
//!   `FS_IOC_SETFLAGS` on Linux and `UF_IMMUTABLE` on macOS; HIDDEN is `UF_HIDDEN` on macOS
//!   (on Linux "hidden" means a leading dot, so it is a rename and reported `Unsupported`).

use std::path::Path;

use serde::{Deserialize, Serialize};
use soyuz_core::{Attributes, FsError, FsErrorKind, FsResult};

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
    #[cfg(not(target_os = "macos"))]
    pub(super) const SUPPORTED: Attributes = MODE_BITS.union(Attributes::IMMUTABLE);

    pub(super) fn apply(path: &Path, a: &AttrSet) -> FsResult<()> {
        let st = us::ustat_at(CWD, path.as_os_str(), true).map_err(err::map("set_attributes", path))?;
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
        if a.set.contains(Attributes::IMMUTABLE) || a.clear.contains(Attributes::IMMUTABLE) {
            set_immutable(path, a.set.contains(Attributes::IMMUTABLE), st.flags)?;
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
        use rustix::fs::{IFlags, OFlags};
        let fd = rustix::fs::openat(CWD, path, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty())
            .map_err(|e| err::io("set_attributes", path, e.into()))?;
        let cur = rustix::fs::ioctl_getflags(&fd).map_err(|e| err::io("set_attributes", path, e.into()))?;
        let new = if on { cur | IFlags::IMMUTABLE } else { cur - IFlags::IMMUTABLE };
        if new != cur {
            rustix::fs::ioctl_setflags(&fd, new).map_err(|e| {
                let mut fe = err::io("set_attributes", path, e.into());
                fe.message = format!("{} (changing the immutable flag needs CAP_LINUX_IMMUTABLE)", fe.message);
                fe
            })?;
        }
        Ok(())
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
