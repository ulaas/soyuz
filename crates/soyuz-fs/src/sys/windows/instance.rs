//! Single-instance launch handoff: the session id and elevation of this process (they
//! scope the instance pipe name; named pipes are machine-global) and the foreground
//! handover between the launcher and the running window. advapi32 / user32 only.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HANDLE, HWND};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_QUERY, TokenElevation, TokenSessionId,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
};

use super::Handle;

/// A fixed-size value of the current process token.
fn token_value<T: Copy + Default>(class: TOKEN_INFORMATION_CLASS) -> Option<T> {
    let mut token: HANDLE = null_mut();
    // SAFETY: valid pseudo-handle and out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = Handle::new(token).ok()?;
    let mut value = T::default();
    let mut len = 0u32;
    let size = std::mem::size_of::<T>() as u32;
    // SAFETY: `value` is a `T`-sized buffer of the type the class returns.
    let ok = unsafe { GetTokenInformation(token.raw(), class, (&mut value as *mut T).cast(), size, &mut len) };
    (ok != 0).then_some(value)
}

/// Remote Desktop / fast-user-switching session of this process.
pub(crate) fn session_id() -> Option<u32> {
    token_value::<u32>(TokenSessionId)
}

/// The process runs with an elevated (administrator) token.
pub(crate) fn is_elevated() -> bool {
    #[derive(Clone, Copy, Default)]
    struct Elevation(u32);
    const _: () = assert!(std::mem::size_of::<Elevation>() == std::mem::size_of::<TOKEN_ELEVATION>());
    token_value::<Elevation>(TokenElevation).is_some_and(|e| e.0 != 0)
}

/// Let process `pid` take the foreground (the launcher calls it for the running window
/// it hands its request to; only works while the caller may set the foreground itself).
pub(crate) fn allow_set_foreground(pid: u32) -> bool {
    // SAFETY: plain call.
    unsafe { AllowSetForegroundWindow(pid) != 0 }
}

/// Restore `hwnd` if minimized and make it the foreground window (no synthetic input).
pub(crate) fn foreground(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    // SAFETY: plain window calls on a handle of this process (stale handles fail harmlessly).
    unsafe {
        if IsIconic(hwnd as HWND) != 0 {
            ShowWindow(hwnd as HWND, SW_RESTORE);
        }
        SetForegroundWindow(hwnd as HWND) != 0
    }
}
