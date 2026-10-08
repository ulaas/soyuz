//! Timestamps: `SetFileTime` on a `FILE_WRITE_ATTRIBUTES` handle (Windows, creation time
//! included), `utimensat` with `UTIME_OMIT` (unix), `setattrlist(ATTR_CMN_CRTIME)` for the
//! birth time on macOS.

use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use soyuz_core::FsResult;
use soyuz_core::time::serde_rfc3339;

/// Timestamps to set; `None` leaves a timestamp unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimesSet {
    #[serde(with = "serde_rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<SystemTime>,
    #[serde(with = "serde_rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub created: Option<SystemTime>,
    #[serde(with = "serde_rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub accessed: Option<SystemTime>,
}

/// Set timestamps (`created` on Windows and macOS; ignored on Linux, which has no API to
/// change the birth time). `follow = false` changes a symlink itself.
pub fn set_times(path: &Path, t: &TimesSet, follow: bool) -> FsResult<()> {
    if t.modified.is_none() && t.accessed.is_none() && t.created.is_none() {
        return Ok(());
    }
    imp::set_times(path, t, follow)
}

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use soyuz_core::FsResult;

    use super::TimesSet;
    use crate::err;
    use crate::sys::windows::{fs as wfs, system_to_ticks};

    pub(super) fn set_times(path: &Path, t: &TimesSet, follow: bool) -> FsResult<()> {
        let c = t.created.map(system_to_ticks);
        let a = t.accessed.map(system_to_ticks);
        let m = t.modified.map(system_to_ticks);
        wfs::set_times(path, follow, c, a, m).map_err(err::map("set_times", path))
    }
}

#[cfg(unix)]
mod imp {
    use std::path::Path;

    use rustix::fs::{AtFlags, CWD, Timespec, Timestamps, UTIME_OMIT};
    use soyuz_core::FsResult;

    use super::TimesSet;
    use crate::err;
    use crate::sys::unix::to_timespec;

    pub(super) fn set_times(path: &Path, t: &TimesSet, follow: bool) -> FsResult<()> {
        let omit = Timespec { tv_sec: 0, tv_nsec: UTIME_OMIT };
        if t.modified.is_some() || t.accessed.is_some() {
            let times = Timestamps {
                last_access: t.accessed.map(to_timespec).unwrap_or(omit),
                last_modification: t.modified.map(to_timespec).unwrap_or(omit),
            };
            let flags = if follow { AtFlags::empty() } else { AtFlags::SYMLINK_NOFOLLOW };
            rustix::fs::utimensat(CWD, path, &times, flags).map_err(|e| err::io("set_times", path, e.into()))?;
        }
        #[cfg(target_os = "macos")]
        if let Some(c) = t.created {
            crate::sys::macos::set_birthtime(path, c).map_err(err::map("set_times", path))?;
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::Path;

    use soyuz_core::{FsError, FsResult};

    use super::TimesSet;

    pub(super) fn set_times(path: &Path, _t: &TimesSet, _follow: bool) -> FsResult<()> {
        Err(FsError::unsupported("set_times", Some(path)))
    }
}
