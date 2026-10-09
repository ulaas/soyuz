//! Owned top-level windows for the app's native dialogs and popup menus: find the
//! `HWND` of a freshly created window of this thread, make it owned by another window
//! (no taskbar button, always above its owner, minimized with it), tool-window style for
//! popups, placement in physical pixels, enable / disable (modal owners) and the work
//! area of a monitor.
//!
//! Only user32 / dwmapi / gdi32 calls (no shell APIs).

use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetActiveWindow, IsWindowEnabled};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GWL_EXSTYLE, GWLP_HWNDPARENT, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, HWND_TOP, IsIconic, IsWindow, IsWindowVisible, SW_HIDE, SW_SHOW, SW_SHOWNA, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

const DWMWCP_ROUNDSMALL: i32 = 3;
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: `lparam` is the `&mut Vec<isize>` passed by `thread_windows` below, alive
    // for the whole (synchronous) enumeration.
    let out = unsafe { &mut *(lparam as *mut Vec<isize>) };
    out.push(hwnd as isize);
    1
}

/// Top-level windows created by the calling thread.
pub(crate) fn thread_windows() -> Vec<isize> {
    let mut out: Vec<isize> = Vec::new();
    // SAFETY: the callback only pushes into `out`, which outlives the call.
    unsafe {
        EnumThreadWindows(GetCurrentThreadId(), Some(collect), (&raw mut out) as LPARAM);
    }
    out
}

/// Window text of `hwnd` (empty on failure).
pub(crate) fn window_title(hwnd: isize) -> String {
    if hwnd == 0 {
        return String::new();
    }
    // SAFETY: plain query on a window handle.
    let n = unsafe { GetWindowTextLengthW(hwnd as HWND) };
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    // SAFETY: the buffer holds `n + 1` UTF-16 units.
    let got = unsafe { GetWindowTextW(hwnd as HWND, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

/// The first top-level window of this thread whose title is exactly `title`.
pub(crate) fn find_thread_window(title: &str) -> Option<isize> {
    thread_windows().into_iter().find(|&h| window_title(h) == title)
}

/// Outer window rect `[left, top, right, bottom]` in physical pixels.
pub(crate) fn window_rect(hwnd: isize) -> Option<[i32; 4]> {
    if hwnd == 0 {
        return None;
    }
    // SAFETY: valid out-struct.
    let mut r: RECT = unsafe { zeroed() };
    // SAFETY: query on a window handle with a live RECT.
    (unsafe { GetWindowRect(hwnd as HWND, &mut r) } != 0).then_some([r.left, r.top, r.right, r.bottom])
}

/// Make `hwnd` owned by `owner` (no taskbar button, stays above the owner, hidden with
/// it when it minimizes). `owner = 0` removes the owner.
pub(crate) fn set_owner(hwnd: isize, owner: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    // SAFETY: GWLP_HWNDPARENT on a top-level window sets its owner.
    unsafe { SetWindowLongPtrW(hwnd as HWND, GWLP_HWNDPARENT, owner) };
    true
}

/// Current owner of `hwnd` (0: none).
pub(crate) fn owner(hwnd: isize) -> isize {
    if hwnd == 0 {
        return 0;
    }
    // SAFETY: plain query.
    unsafe { GetWindowLongPtrW(hwnd as HWND, GWLP_HWNDPARENT) }
}

/// `WS_EX_TOOLWINDOW` (no taskbar button, not in Alt+Tab) for popups.
pub(crate) fn set_tool_window(hwnd: isize, on: bool) -> bool {
    if hwnd == 0 {
        return false;
    }
    // SAFETY: style query / update on a window owned by this process.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE);
        let new = if on {
            (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize)
        } else {
            ex & !(WS_EX_TOOLWINDOW as isize)
        };
        if new != ex {
            SetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE, new);
            SetWindowPos(
                hwnd as HWND,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
    true
}

/// Move (and with `size`, resize) `hwnd` to `[x, y, w, h]` physical pixels without
/// activating it or changing the z-order.
pub(crate) fn set_window_rect(hwnd: isize, r: [i32; 4], size: bool) -> bool {
    if hwnd == 0 {
        return false;
    }
    let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
    if !size {
        flags |= SWP_NOSIZE;
    }
    // SAFETY: plain window placement call.
    unsafe { SetWindowPos(hwnd as HWND, std::ptr::null_mut(), r[0], r[1], r[2].max(1), r[3].max(1), flags) != 0 }
}

/// Raise `hwnd` to the top of its z-order band (owned windows stay above their owner).
pub(crate) fn bring_to_top(hwnd: isize) {
    if hwnd != 0 {
        // SAFETY: plain window placement call.
        unsafe { SetWindowPos(hwnd as HWND, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
    }
}

/// Show `hwnd`; `activate` makes it the active window (keyboard focus).
pub(crate) fn show_window(hwnd: isize, activate: bool) {
    if hwnd == 0 {
        return;
    }
    // SAFETY: plain window calls.
    unsafe {
        ShowWindow(hwnd as HWND, if activate { SW_SHOW } else { SW_SHOWNA });
        if activate {
            SetForegroundWindow(hwnd as HWND);
        }
    }
}

pub(crate) fn hide_window(hwnd: isize) {
    if hwnd != 0 {
        // SAFETY: plain window call.
        unsafe { ShowWindow(hwnd as HWND, SW_HIDE) };
    }
}

pub(crate) fn is_visible(hwnd: isize) -> bool {
    // SAFETY: plain query.
    hwnd != 0 && unsafe { IsWindowVisible(hwnd as HWND) } != 0
}

pub(crate) fn is_window(hwnd: isize) -> bool {
    // SAFETY: plain query (stale handles return false).
    hwnd != 0 && unsafe { IsWindow(hwnd as HWND) } != 0
}

pub(crate) fn is_minimized(hwnd: isize) -> bool {
    // SAFETY: plain query.
    hwnd != 0 && unsafe { IsIconic(hwnd as HWND) } != 0
}

/// Enable / disable mouse and keyboard input to `hwnd` (modal owners).
pub(crate) fn enable_window(hwnd: isize, on: bool) {
    if hwnd != 0 {
        // SAFETY: plain window call.
        unsafe { EnableWindow(hwnd as HWND, i32::from(on)) };
    }
}

pub(crate) fn is_enabled(hwnd: isize) -> bool {
    // SAFETY: plain query.
    hwnd == 0 || unsafe { IsWindowEnabled(hwnd as HWND) } != 0
}

/// The active window of this thread (0: none).
pub(crate) fn active_window() -> isize {
    // SAFETY: plain query.
    unsafe { GetActiveWindow() as isize }
}

/// DPI of the monitor `hwnd` is on (96 = 100 %; 0 when unknown).
pub(crate) fn window_dpi(hwnd: isize) -> u32 {
    if hwnd == 0 {
        return 0;
    }
    // SAFETY: plain query.
    unsafe { GetDpiForWindow(hwnd as HWND) }
}

/// `(work area, monitor rect)` as `[left, top, right, bottom]` physical pixels of the
/// monitor at (or nearest to) the physical point `(x, y)`.
pub(crate) fn monitor_at(x: i32, y: i32) -> Option<([i32; 4], [i32; 4])> {
    // SAFETY: plain query; MONITOR_DEFAULTTONEAREST always yields a monitor.
    let mon = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    if mon.is_null() {
        return None;
    }
    // SAFETY: plain-old-data struct with its size filled in.
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    // SAFETY: valid monitor handle and a correctly sized out-struct.
    if unsafe { GetMonitorInfoW(mon, &mut info) } == 0 {
        return None;
    }
    let w = info.rcWork;
    let m = info.rcMonitor;
    Some(([w.left, w.top, w.right, w.bottom], [m.left, m.top, m.right, m.bottom]))
}

/// Small rounded corners (Windows 11 menus / flyouts).
pub(crate) fn set_small_corners(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    let v = DWMWCP_ROUNDSMALL;
    // SAFETY: valid window handle; pointer and size describe a live i32.
    let hr = unsafe {
        windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute(
            hwnd as HWND,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const v).cast(),
            size_of::<i32>() as u32,
        )
    };
    hr >= 0
}
