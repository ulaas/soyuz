//! Single-instance launch handoff: the session id and elevation of this process (they
//! scope the instance pipe name; named pipes are machine-global) and the foreground
//! handover between the launcher and the running window. advapi32 / user32 only.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HANDLE, HWND};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_QUERY, TokenElevation, TokenSessionId,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, OpenProcessToken,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, SW_RESTORE,
    SetForegroundWindow, ShowWindow,
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
/// `true` when it is the foreground window afterwards.
pub(crate) fn foreground(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    let h = hwnd as HWND;
    // SAFETY: plain window calls on a handle of this process (stale handles fail harmlessly).
    unsafe {
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        SetForegroundWindow(h) != 0 && GetForegroundWindow() == h
    }
}

/// The foreground window (0: none).
pub(crate) fn foreground_window() -> isize {
    // SAFETY: plain query.
    unsafe { GetForegroundWindow() as isize }
}

/// Milliseconds since the last keyboard or mouse input in this session.
pub(crate) fn ms_since_input() -> Option<u32> {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: `info` is a valid out structure with its size set.
    unsafe { (GetLastInputInfo(&mut info) != 0).then(|| GetTickCount().wrapping_sub(info.dwTime)) }
}

/// A refused [`foreground`], for a launch the user asked for: if `front` still has the
/// foreground, share its thread's input queue for the moment it takes to move the focus.
/// Without this the window sits on screen while the keys go to the other program. No
/// synthetic input. `true` when `hwnd` is the foreground window afterwards.
pub(crate) fn take_foreground_from(hwnd: isize, front: isize) -> bool {
    if hwnd == 0 || front == 0 {
        return false;
    }
    let h = hwnd as HWND;
    // SAFETY: plain window and thread calls (stale handles fail harmlessly); the input
    // queues are detached again before returning.
    unsafe {
        let now = GetForegroundWindow();
        if now as isize != front {
            log::info!("instance: not taking the foreground: {:#x} is in front, not {front:#x}", now as isize);
            return false;
        }
        let front_thread = GetWindowThreadProcessId(now, null_mut());
        let ours = GetCurrentThreadId();
        if front_thread == 0 || front_thread == ours {
            return false;
        }
        let attached = AttachThreadInput(ours, front_thread, 1) != 0;
        SetForegroundWindow(h);
        BringWindowToTop(h);
        SetFocus(h);
        if attached {
            AttachThreadInput(ours, front_thread, 0);
        }
        let took = GetForegroundWindow() == h;
        log::info!("instance: took the foreground from {front:#x}: {took} (input queues shared: {attached})");
        took
    }
}
