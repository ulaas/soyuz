//! Window chrome helpers for the app's custom title bar (roadmap B1): DWM attributes
//! (corner preference, dark mode, Windows 11 system backdrop) and a comctl32 window
//! subclass that reports the maximize button to the OS (`WM_NCHITTEST` → `HTMAXBUTTON`)
//! so Windows 11 shows its Snap Layouts flyout.
//!
//! Only dwmapi, user32, kernel32 and comctl32 are used (no shell APIs). The comctl32 and
//! ntdll entry points are resolved at run time with `GetProcAddress`, so a missing export
//! degrades to "feature unavailable" instead of failing to start.

use std::ffi::c_void;
use std::mem::{size_of, transmute, zeroed};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{
    BLACK_BRUSH, FillRect, GetMonitorInfoW, GetStockObject, HBRUSH, HDC, InvalidateRect, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromWindow, ScreenToClient,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetClientRect, GetWindowLongPtrW, GetWindowPlacement, HTCLIENT, HTMAXBUTTON, IsWindowVisible, IsZoomed,
    SIZE_MAXIMIZED, SM_CXFRAME, SM_CXPADDEDBORDER, SM_CYFRAME, STYLESTRUCT, SW_HIDE, SW_MAXIMIZE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetWindowLongPtrW, SetWindowPlacement,
    SetWindowPos, ShowWindow, WINDOWPLACEMENT, WM_ERASEBKGND, WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST,
    WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SIZE, WM_STYLECHANGING,
    WS_MAXIMIZE, WS_SYSMENU,
};

use super::wide_str;

const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
const DWMWCP_DEFAULT: i32 = 0;
const DWMWCP_DONOTROUND: i32 = 1;
const DWMWCP_ROUND: i32 = 2;

type SubclassProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT;
type SetWindowSubclassFn = unsafe extern "system" fn(HWND, Option<SubclassProc>, usize, usize) -> i32;
type DefSubclassProcFn = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
type RemoveWindowSubclassFn = unsafe extern "system" fn(HWND, Option<SubclassProc>, usize) -> i32;

#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    csd: [u16; 128],
}
type RtlGetVersionFn = unsafe extern "system" fn(*mut OsVersionInfoW) -> i32;

/// Look up `name` in `dll` (loading it if needed).
fn proc_address(dll: &str, name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
    debug_assert!(name.ends_with(&[0]));
    let w = wide_str(dll);
    // SAFETY: NUL-terminated UTF-16 module name; system DLLs stay loaded for the process.
    let module = unsafe { LoadLibraryW(w.as_ptr()) };
    if module.is_null() {
        return None;
    }
    // SAFETY: valid module handle and a NUL-terminated ASCII export name.
    unsafe { GetProcAddress(module, name.as_ptr()) }
}

/// Windows build number (`RtlGetVersion` is not subject to manifest version lies).
pub(crate) fn os_build() -> u32 {
    static BUILD: OnceLock<u32> = OnceLock::new();
    *BUILD.get_or_init(|| {
        let Some(f) = proc_address("ntdll.dll", b"RtlGetVersion\0") else { return 0 };
        // SAFETY: `RtlGetVersion` has exactly this signature.
        let f: RtlGetVersionFn = unsafe { transmute::<unsafe extern "system" fn() -> isize, RtlGetVersionFn>(f) };
        // SAFETY: plain-old-data struct, size field filled in before the call.
        let mut info: OsVersionInfoW = unsafe { zeroed() };
        info.size = size_of::<OsVersionInfoW>() as u32;
        // SAFETY: valid, correctly sized out-struct.
        if unsafe { f(&mut info) } == 0 { info.build } else { 0 }
    })
}

fn set_attr_i32(hwnd: isize, attr: u32, value: i32) -> bool {
    if hwnd == 0 {
        return false;
    }
    // SAFETY: `hwnd` is a window handle owned by this process; the value pointer and size
    // describe a live i32.
    let hr = unsafe {
        DwmSetWindowAttribute(hwnd as HWND, attr, (&raw const value).cast::<c_void>(), size_of::<i32>() as u32)
    };
    hr >= 0
}

/// `DWMWA_WINDOW_CORNER_PREFERENCE`: `Some(true)` round, `Some(false)` square, `None` default.
pub(crate) fn set_corner_preference(hwnd: isize, round: Option<bool>) -> bool {
    let v = match round {
        Some(true) => DWMWCP_ROUND,
        Some(false) => DWMWCP_DONOTROUND,
        None => DWMWCP_DEFAULT,
    };
    set_attr_i32(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, v)
}

/// Put a window that has not been shown yet into the maximized state without showing it:
/// `ShowWindow(SW_MAXIMIZE)` always shows (and activates) a window, so instead the window
/// gets `WS_MAXIMIZE` and the rectangle Windows gives a maximized window (the monitor's
/// work area plus the frame overhang), while its restored placement stays as it is. The
/// next ordinary show opens it maximized, once. A visible window is simply maximized.
pub(crate) fn maximize_hidden(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    let h = hwnd as HWND;
    // SAFETY: plain user32 calls on a window handle owned by this process with live
    // out-structs; a stale handle only makes them fail.
    unsafe {
        if IsWindowVisible(h) != 0 {
            ShowWindow(h, SW_MAXIMIZE);
            return IsZoomed(h) != 0;
        }
        let mut wp: WINDOWPLACEMENT = zeroed();
        wp.length = size_of::<WINDOWPLACEMENT>() as u32;
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        if GetWindowPlacement(h, &mut wp) == 0
            || GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi) == 0
        {
            return false;
        }
        let dpi = GetDpiForWindow(h).max(96);
        let pad = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
        let fx = GetSystemMetricsForDpi(SM_CXFRAME, dpi) + pad;
        let fy = GetSystemMetricsForDpi(SM_CYFRAME, dpi) + pad;
        let style = GetWindowLongPtrW(h, GWL_STYLE);
        SetWindowLongPtrW(h, GWL_STYLE, style | WS_MAXIMIZE as isize);
        let (x, y, w, ht) = maximized_rect(mi.rcWork, fx, fy);
        SetWindowPos(h, std::ptr::null_mut(), x, y, w, ht, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        // Keep the restored rectangle the window had (moving it may have replaced it).
        wp.showCmd = SW_HIDE as u32;
        wp.flags = 0;
        SetWindowPlacement(h, &wp);
        // Windows sends no `WM_SIZE` when the client size happens to stay the same, and
        // the window's owner (winit) learns the maximized state from it.
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(h, &mut rc);
        let size = ((rc.bottom - rc.top) as u32 & 0xffff) << 16 | ((rc.right - rc.left) as u32 & 0xffff);
        SendMessageW(h, WM_SIZE, SIZE_MAXIMIZED as WPARAM, size as LPARAM);
        IsZoomed(h) != 0
    }
}

/// `(x, y, w, h)` of a window maximized on a monitor with work area `work`: the frame
/// (`fx` / `fy` pixels) hangs over the work area's edges.
fn maximized_rect(work: RECT, fx: i32, fy: i32) -> (i32, i32, i32, i32) {
    (work.left - fx, work.top - fy, work.right - work.left + 2 * fx, work.bottom - work.top + 2 * fy)
}

/// `DWMWA_USE_IMMERSIVE_DARK_MODE` (tints the system backdrop and the frame).
pub(crate) fn set_dark_mode(hwnd: isize, dark: bool) -> bool {
    set_attr_i32(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, i32::from(dark))
}

/// `DWMWA_SYSTEMBACKDROP_TYPE` (1 none, 2 mica, 3 acrylic, 4 tabbed). For a material
/// other than none the frame is extended over the whole client area so the material
/// shows through transparent pixels.
pub(crate) fn set_backdrop(hwnd: isize, kind: i32) -> bool {
    if hwnd == 0 {
        return false;
    }
    let m = if kind > 1 { -1 } else { 0 };
    let margins = MARGINS { cxLeftWidth: m, cxRightWidth: m, cyTopHeight: m, cyBottomHeight: m };
    // SAFETY: valid window handle and a live MARGINS struct.
    let _ = unsafe { DwmExtendFrameIntoClientArea(hwnd as HWND, &margins) };
    set_attr_i32(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, kind)
}

// --- Snap Layouts subclass ----------------------------------------------------------

const SUBCLASS_ID: usize = 0x534F_595A; // "SOYZ"

struct Comctl {
    set: SetWindowSubclassFn,
    def: DefSubclassProcFn,
    remove: RemoveWindowSubclassFn,
}

fn comctl() -> Option<&'static Comctl> {
    static C: OnceLock<Option<Comctl>> = OnceLock::new();
    C.get_or_init(|| {
        let set = proc_address("comctl32.dll", b"SetWindowSubclass\0")?;
        let def = proc_address("comctl32.dll", b"DefSubclassProc\0")?;
        let remove = proc_address("comctl32.dll", b"RemoveWindowSubclass\0")?;
        // SAFETY: the three exports have exactly these signatures (commctrl.h).
        unsafe {
            Some(Comctl {
                set: transmute::<unsafe extern "system" fn() -> isize, SetWindowSubclassFn>(set),
                def: transmute::<unsafe extern "system" fn() -> isize, DefSubclassProcFn>(def),
                remove: transmute::<unsafe extern "system" fn() -> isize, RemoveWindowSubclassFn>(remove),
            })
        }
    })
    .as_ref()
}

/// Maximize button rect in client pixels (left, top, right, bottom); `ENABLED` gates it.
static RECT: [AtomicI32; 4] = [AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0), AtomicI32::new(0)];
static ENABLED: AtomicBool = AtomicBool::new(false);
/// Erase the GDI surface to black so an extended DWM frame (system backdrop) shows.
static BLACK_BG: AtomicBool = AtomicBool::new(false);
static HOVER: AtomicBool = AtomicBool::new(false);
static PRESSED: AtomicBool = AtomicBool::new(false);
static CLICKED: AtomicBool = AtomicBool::new(false);
static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

fn wake() {
    if let Some(w) = WAKE.get() {
        w();
    }
}

fn set_flag(flag: &AtomicBool, v: bool) {
    if flag.swap(v, Ordering::Relaxed) != v {
        wake();
    }
}

fn in_rect(x: i32, y: i32) -> bool {
    let r = |i: usize| RECT[i].load(Ordering::Relaxed);
    x >= r(0) && x < r(2) && y >= r(1) && y < r(3)
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    let Some(c) = comctl() else { return 0 };
    // SAFETY: forwarding the message we were given to the next window procedure.
    let next = || unsafe { (c.def)(hwnd, msg, wparam, lparam) };
    match msg {
        WM_ERASEBKGND if BLACK_BG.load(Ordering::Relaxed) => {
            // SAFETY: zeroed RECT is valid; `wparam` is the HDC DefWindowProc handed us.
            unsafe {
                let mut rc = zeroed();
                GetClientRect(hwnd, &mut rc);
                FillRect(wparam as HDC, &rc, GetStockObject(BLACK_BRUSH) as HBRUSH);
            }
            1
        }
        // With the frame extended over the client area DWM would draw its own caption
        // buttons for a WS_SYSMENU window; keep the style off while the backdrop is on.
        WM_STYLECHANGING if BLACK_BG.load(Ordering::Relaxed) && wparam as i32 == GWL_STYLE => {
            // SAFETY: for WM_STYLECHANGING, lparam points to a live STYLESTRUCT.
            unsafe { (*(lparam as *mut STYLESTRUCT)).styleNew &= !WS_SYSMENU };
            next()
        }
        WM_NCHITTEST => {
            let r = next();
            if r == HTCLIENT as LRESULT && ENABLED.load(Ordering::Relaxed) {
                let mut pt = POINT { x: (lparam & 0xFFFF) as i16 as i32, y: ((lparam >> 16) & 0xFFFF) as i16 as i32 };
                // SAFETY: valid window handle and a live POINT.
                if unsafe { ScreenToClient(hwnd, &mut pt) } != 0 && in_rect(pt.x, pt.y) {
                    return HTMAXBUTTON as LRESULT;
                }
            }
            r
        }
        WM_NCMOUSEMOVE => {
            let over = wparam == HTMAXBUTTON as WPARAM;
            set_flag(&HOVER, over);
            if !over {
                set_flag(&PRESSED, false);
            } else {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: correctly sized struct naming our own window.
                unsafe { TrackMouseEvent(&mut tme) };
            }
            next()
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam == HTMAXBUTTON as WPARAM => {
            set_flag(&PRESSED, true);
            0
        }
        WM_NCLBUTTONUP if wparam == HTMAXBUTTON as WPARAM => {
            if PRESSED.swap(false, Ordering::Relaxed) {
                CLICKED.store(true, Ordering::Relaxed);
            }
            wake();
            0
        }
        WM_NCMOUSELEAVE => {
            set_flag(&HOVER, false);
            set_flag(&PRESSED, false);
            next()
        }
        WM_MOUSEMOVE => {
            set_flag(&HOVER, false);
            next()
        }
        WM_NCDESTROY => {
            ENABLED.store(false, Ordering::Relaxed);
            // SAFETY: removing the subclass we installed on this window.
            unsafe { (c.remove)(hwnd, Some(subclass_proc), SUBCLASS_ID) };
            next()
        }
        _ => next(),
    }
}

/// Install the maximize-button hit-test subclass on `hwnd` (call on the window's
/// thread). `wake` is called (from the window thread) when the hover / pressed state
/// changes or the button is clicked. Returns false when comctl32's subclass API is
/// unavailable.
pub(crate) fn install_subclass(hwnd: isize, black_bg: bool, wake_fn: Box<dyn Fn() + Send + Sync>) -> bool {
    let Some(c) = comctl() else { return false };
    if hwnd == 0 {
        return false;
    }
    BLACK_BG.store(black_bg, Ordering::Relaxed);
    let _ = WAKE.set(wake_fn);
    // SAFETY: valid window handle of this thread; `subclass_proc` lives for the program.
    let ok = unsafe { (c.set)(hwnd as HWND, Some(subclass_proc), SUBCLASS_ID, 0) != 0 };
    if ok && black_bg {
        let hwnd = hwnd as HWND;
        // SAFETY: valid window handle of this thread; the style change goes through our
        // WM_STYLECHANGING filter; a null rect invalidates the whole client area.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_SYSMENU as isize));
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
    ok
}

/// Update the maximize button rect in client pixels (`None` disables the hit test).
pub(crate) fn set_max_button_rect(rect: Option<[i32; 4]>) {
    match rect {
        Some(r) => {
            for (slot, v) in RECT.iter().zip(r) {
                slot.store(v, Ordering::Relaxed);
            }
            ENABLED.store(true, Ordering::Relaxed);
        }
        None => {
            ENABLED.store(false, Ordering::Relaxed);
            HOVER.store(false, Ordering::Relaxed);
            PRESSED.store(false, Ordering::Relaxed);
        }
    }
}

/// `(hovered, pressed, clicked since the last call)` of the OS-level maximize button.
pub(crate) fn max_button_state() -> (bool, bool, bool) {
    (HOVER.load(Ordering::Relaxed), PRESSED.load(Ordering::Relaxed), CLICKED.swap(false, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximized_rect_overhangs_the_work_area() {
        let rect = |left, top, right, bottom| RECT { left, top, right, bottom };
        // 2560 x 1440 monitor, 48 px taskbar at the bottom, 8 px frame overhang.
        assert_eq!(maximized_rect(rect(0, 0, 2560, 1392), 8, 8), (-8, -8, 2576, 1408));
        // Second monitor to the right with the taskbar at the top.
        assert_eq!(maximized_rect(rect(2560, 40, 4480, 1080), 8, 8), (2552, 32, 1936, 1056));
    }
}
