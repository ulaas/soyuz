//! Raw Windows shims over kernel32 / advapi32 / ntdll (`windows-sys`). No shell or COM APIs.
//!
//! Every path handed to the OS goes through [`crate::path::to_wide_long`] (absolute,
//! `\\?\`-prefixed, NUL-terminated) so long paths and trailing dots/spaces work.

pub(crate) mod chrome;
pub(crate) mod console;
pub(crate) mod fs;
pub(crate) mod locale;
pub(crate) mod ntdir;
pub(crate) mod registry;
pub(crate) mod security;
pub(crate) mod shadow;
pub(crate) mod shares;
pub(crate) mod usn;
pub(crate) mod volume;
pub(crate) mod wintrust;

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};

/// NUL-terminated long (`\\?\`) form of `p`.
pub(crate) fn wide_path(p: &Path) -> Vec<u16> {
    crate::path::to_wide_long(p)
}

/// NUL-terminated UTF-16 of an arbitrary string (no path processing).
pub(crate) fn wide_str(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// UTF-16 buffer up to the first NUL.
pub(crate) fn from_wide_nul(buf: &[u16]) -> OsString {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..end])
}

pub(crate) fn last_err() -> io::Error {
    io::Error::last_os_error()
}

/// Owned kernel handle closed on drop.
#[derive(Debug)]
pub(crate) struct Handle(HANDLE);

impl Handle {
    /// Wrap a handle returned by a `Create*`/`Open*` call; invalid handles become the
    /// thread's last OS error.
    pub(crate) fn new(h: HANDLE) -> io::Result<Handle> {
        if h.is_null() || h == INVALID_HANDLE_VALUE { Err(last_err()) } else { Ok(Handle(h)) }
    }

    pub(crate) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid handle we own exclusively; it is closed exactly once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

// SAFETY: kernel handles are process-wide and may be used/closed from any thread.
unsafe impl Send for Handle {}

/// 100 ns ticks between 1601-01-01 and 1970-01-01.
const EPOCH_DIFF_TICKS: u64 = 116_444_736_000_000_000;

pub(crate) fn ft_u64(ft: &FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

pub(crate) fn u64_ft(t: u64) -> FILETIME {
    FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 }
}

/// FILETIME ticks → `SystemTime` (`None` for 0, i.e. "not set").
pub(crate) fn ticks_to_system(t: u64) -> Option<SystemTime> {
    if t == 0 {
        return None;
    }
    let to_dur = |ticks: u64| Duration::new(ticks / 10_000_000, ((ticks % 10_000_000) * 100) as u32);
    if t >= EPOCH_DIFF_TICKS {
        UNIX_EPOCH.checked_add(to_dur(t - EPOCH_DIFF_TICKS))
    } else {
        UNIX_EPOCH.checked_sub(to_dur(EPOCH_DIFF_TICKS - t))
    }
}

/// `SystemTime` → FILETIME ticks (clamped to the representable range, never 0).
pub(crate) fn system_to_ticks(t: SystemTime) -> u64 {
    let ticks = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => EPOCH_DIFF_TICKS
            .saturating_add(d.as_secs().saturating_mul(10_000_000).saturating_add(u64::from(d.subsec_nanos() / 100))),
        Err(e) => {
            let d = e.duration();
            EPOCH_DIFF_TICKS.saturating_sub(
                d.as_secs().saturating_mul(10_000_000).saturating_add(u64::from(d.subsec_nanos() / 100)),
            )
        }
    };
    ticks.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_round_trip() {
        let t = UNIX_EPOCH + Duration::new(1_600_000_000, 123_456_700);
        let ticks = system_to_ticks(t);
        assert_eq!(ticks_to_system(ticks), Some(t));
        assert_eq!(ticks_to_system(0), None);
        assert_eq!(ticks_to_system(EPOCH_DIFF_TICKS), Some(UNIX_EPOCH));
        let ft = u64_ft(ticks);
        assert_eq!(ft_u64(&ft), ticks);
        let before = UNIX_EPOCH - Duration::from_secs(86_400);
        assert_eq!(ticks_to_system(system_to_ticks(before)), Some(before));
    }

    #[test]
    fn filetime_far_future_saturates() {
        // Seconds near u64::MAX / 1e7 plus sub-second ticks used to overflow the `+`.
        if let Some(far) = UNIX_EPOCH.checked_add(Duration::new(u64::MAX / 10_000_000, 999_999_900)) {
            assert_eq!(system_to_ticks(far), u64::MAX);
        }
    }

    #[test]
    fn wide_helpers() {
        assert_eq!(wide_str("ab"), vec![97, 98, 0]);
        assert_eq!(from_wide_nul(&[97, 0, 98]), OsString::from("a"));
        assert_eq!(from_wide_nul(&[97, 98]), OsString::from("ab"));
    }
}
