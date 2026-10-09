//! Startup placeholder window (roadmap R5): a plain user32 / GDI popup painted with the
//! theme background and the logo within a few tens of milliseconds of process start,
//! while the GPU renderer initialises (the D3D12 driver load alone takes ~200 ms). It
//! lives on its own thread with its own message loop and is closed once the real window
//! has presented its first frame (the real window then covers it, so closing it is not
//! visible). In frames mode (`[performance] cpu_first_frame`) it takes the real window's
//! exact client rectangle, shows CPU-rendered UI frames handed over by
//! [`Splash::present`] and collects pointer input for them. No shell API is involved.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, CreateSolidBrush, DIB_RGB_COLORS, DeleteObject, EndPaint,
    FillRect, GetMonitorInfoW, HDC, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
    PAINTSTRUCT, SRCCOPY, StretchDIBits, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IDC_APPSTARTING, IDC_ARROW,
    LoadCursorW, MA_NOACTIVATE, MSG, PostMessageW, PostQuitMessage, RegisterClassExW, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, ShowWindow, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::wide_str;
use crate::splash::Input;

/// Where the placeholder goes (the real window's expected rectangle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    /// Centered on the primary monitor, this inner size in points (eframe `centered`).
    Centered { width: f32, height: f32 },
    /// The work area of the primary monitor (a maximized window).
    Maximized,
    /// At this position and inner size in points (`SOYUZ_WINDOW_POS`).
    At { x: f32, y: f32, width: f32, height: f32 },
}

struct Paint {
    /// RGB background.
    bg: [u8; 3],
    /// Logo pre-composited over the background: BGRA, top-down.
    logo: Option<(Vec<u8>, i32, i32)>,
}

static PAINT: Mutex<Option<Paint>> = Mutex::new(None);

/// A full client-area picture handed over by [`Splash::present`] (BGRA, top-down).
struct Frame {
    bgra: Vec<u8>,
    w: i32,
    h: i32,
    painted: Option<Box<dyn FnOnce() + Send>>,
}

static FRAME: Mutex<Option<Frame>> = Mutex::new(None);
/// Pointer input over the placeholder (frames mode).
static INPUT: Mutex<Vec<Input>> = Mutex::new(Vec::new());
static COLLECT_INPUT: AtomicBool = AtomicBool::new(false);

/// The open placeholder window.
pub(crate) struct Splash {
    hwnd: AtomicIsize,
    /// Client size in physical pixels.
    pub(crate) size: (i32, i32),
    /// Monitor scale factor (DPI / 96).
    pub(crate) scale: f32,
}

impl Splash {
    /// Close the placeholder (from any thread).
    pub(crate) fn close(&self) {
        let h = self.hwnd.swap(0, Ordering::SeqCst);
        if h != 0 {
            COLLECT_INPUT.store(false, Ordering::Relaxed);
            // SAFETY: posting to a window handle owned by the splash thread; a stale
            // handle only makes the call fail.
            unsafe { PostMessageW(h as HWND, WM_CLOSE, 0, 0) };
        }
    }

    /// Show `bgra` (`w × h`, top-down: the client size) instead of the logo; `painted`
    /// runs on the placeholder thread right after it was drawn.
    pub(crate) fn present(&self, bgra: Vec<u8>, w: i32, h: i32, painted: Option<Box<dyn FnOnce() + Send>>) {
        let hwnd = self.hwnd.load(Ordering::SeqCst);
        if hwnd == 0 || w <= 0 || h <= 0 || bgra.len() != w as usize * h as usize * 4 {
            return;
        }
        if let Ok(mut g) = FRAME.lock() {
            *g = Some(Frame { bgra, w, h, painted });
        }
        // SAFETY: invalidating a window owned by the placeholder thread is allowed from
        // any thread; a stale handle only makes the call fail.
        unsafe { InvalidateRect(hwnd as HWND, null(), 0) };
    }

    /// Pointer input received since the last call.
    pub(crate) fn take_input(&self) -> Vec<Input> {
        INPUT.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
    }
}

/// Draw `bits` (`w × h` BGRA, top-down) at `(x, y)`.
fn blit(hdc: HDC, x: i32, y: i32, bits: &[u8], w: i32, h: i32) {
    // SAFETY: plain-old-data header; negative height = top-down rows.
    let mut bmi: BITMAPINFO = unsafe { zeroed() };
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        // SAFETY: the remaining header fields are plain integers.
        ..unsafe { zeroed() }
    };
    // SAFETY: `bits` holds w * h BGRA pixels as described by `bmi` (checked by callers).
    unsafe { StretchDIBits(hdc, x, y, w, h, 0, 0, w, h, bits.as_ptr().cast(), &bmi, DIB_RGB_COLORS, SRCCOPY) };
}

/// Pointer message → [`Input`] (client coordinates; `None` for other messages).
fn pointer_input(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<Input> {
    let x = i32::from((lp & 0xffff) as u16 as i16);
    let y = i32::from(((lp >> 16) & 0xffff) as u16 as i16);
    Some(match msg {
        WM_MOUSEMOVE => Input::Move { x, y },
        WM_LBUTTONDOWN => Input::Button { x, y, button: 0, down: true },
        WM_LBUTTONUP => Input::Button { x, y, button: 0, down: false },
        WM_RBUTTONDOWN => Input::Button { x, y, button: 1, down: true },
        WM_RBUTTONUP => Input::Button { x, y, button: 1, down: false },
        WM_MBUTTONDOWN => Input::Button { x, y, button: 2, down: true },
        WM_MBUTTONUP => Input::Button { x, y, button: 2, down: false },
        // Wheel positions are screen coordinates; the last move gives the spot.
        WM_MOUSEWHEEL => Input::Wheel { notches: f32::from(((wp >> 16) & 0xffff) as u16 as i16) / 120.0 },
        _ => return None,
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            // SAFETY: plain-old-data out-struct for BeginPaint.
            let mut ps: PAINTSTRUCT = unsafe { zeroed() };
            // SAFETY: `hwnd` is the window being painted; `ps` lives across Begin/EndPaint.
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            // A presented UI frame replaces the logo.
            let frame = FRAME.lock().ok().and_then(|mut g| {
                let f = g.as_mut()?;
                blit(hdc, 0, 0, &f.bgra, f.w, f.h);
                Some(f.painted.take())
            });
            if frame.is_none()
                && let Ok(guard) = PAINT.lock()
                && let Some(p) = guard.as_ref()
            {
                let mut rc: RECT = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                // SAFETY: valid window, writable RECT.
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc) };
                let color = u32::from(p.bg[0]) | (u32::from(p.bg[1]) << 8) | (u32::from(p.bg[2]) << 16);
                // SAFETY: the brush is created, used for one fill and deleted.
                unsafe {
                    let brush = CreateSolidBrush(color);
                    FillRect(hdc, &rc, brush);
                    DeleteObject(brush);
                }
                if let Some((bits, w, h)) = &p.logo {
                    let x = (rc.right - rc.left - w) / 2;
                    let y = (rc.bottom - rc.top - h) / 2;
                    blit(hdc, x, y, bits, *w, *h);
                }
            }
            // SAFETY: matches the BeginPaint above.
            unsafe { EndPaint(hwnd, &ps) };
            if let Some(Some(cb)) = frame {
                cb();
            }
            0
        }
        // Clicks must not activate the placeholder: focus belongs to the real window.
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN
        | WM_MBUTTONUP | WM_MOUSEWHEEL
            if COLLECT_INPUT.load(Ordering::Relaxed) =>
        {
            if let Some(ev) = pointer_input(msg, wp, lp)
                && let Ok(mut v) = INPUT.lock()
            {
                v.push(ev);
            }
            0
        }
        WM_CLOSE => {
            // SAFETY: destroying our own window on its thread.
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: default handling with the arguments we were given.
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// `rgba` (straight alpha) composited over `bg` as top-down BGRA.
pub(crate) fn composite(rgba: &[u8], bg: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.as_chunks::<4>().0 {
        let a = u32::from(px[3]);
        let mix = |c: u8, b: u8| ((u32::from(c) * a + u32::from(b) * (255 - a) + 127) / 255) as u8;
        out.extend_from_slice(&[mix(px[2], bg[2]), mix(px[1], bg[1]), mix(px[0], bg[0]), 255]);
    }
    out
}

/// Open the placeholder on its own thread; `painted` runs once its first paint is done.
/// `frames`: it will show real UI frames ([`Splash::present`]), so it takes the exact
/// client rectangle of the real window (no inset) and collects pointer input.
/// Returns `None` when the window cannot be created.
pub(crate) fn open(
    place: Place,
    bg: [u8; 3],
    logo: Option<(Vec<u8>, u32, u32)>,
    painted: Box<dyn FnOnce() + Send>,
    frames: bool,
) -> Option<Splash> {
    if let Ok(mut g) = PAINT.lock() {
        *g = Some(Paint { bg, logo: logo.map(|(rgba, w, h)| (composite(&rgba, bg), w as i32, h as i32)) });
    }
    COLLECT_INPUT.store(frames, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel::<(isize, i32, i32, f32)>();
    std::thread::Builder::new()
        .name("soyuz-splash".into())
        .spawn(move || {
            // SAFETY: per-thread DPI awareness so the rectangle is in physical pixels,
            // like the real window's (winit makes the process per-monitor aware later).
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
            let class = wide_str("SoyuzStartupPlaceholder");
            // SAFETY: null = this module.
            let hinst = unsafe { GetModuleHandleW(null()) };
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: hinst,
                // SAFETY: a system cursor (null instance). Showing live UI frames the
                // window is usable, so no "starting" cursor then.
                hCursor: unsafe { LoadCursorW(null_mut(), if frames { IDC_ARROW } else { IDC_APPSTARTING }) },
                lpszClassName: class.as_ptr(),
                // SAFETY: remaining fields are optional and valid when zeroed.
                ..unsafe { zeroed() }
            };
            // SAFETY: `wc` and the class name outlive the call.
            unsafe { RegisterClassExW(&wc) };
            let title = wide_str("Soyuz");
            // SAFETY: registered class, NUL-terminated strings; a hidden tool window
            // (no taskbar button, never activated) sized below.
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    null_mut(),
                    null_mut(),
                    hinst,
                    null(),
                )
            };
            if hwnd.is_null() {
                let _ = tx.send((0, 0, 0, 1.0));
                return;
            }
            // SAFETY: plain-old-data out-struct with its size set.
            let mut mi: MONITORINFO = unsafe { zeroed() };
            mi.cbSize = size_of::<MONITORINFO>() as u32;
            // SAFETY: primary monitor handle, writable MONITORINFO.
            unsafe { GetMonitorInfoW(MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY), &mut mi) };
            // SAFETY: our window, on the primary monitor.
            let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
            let (x, y, w, h) = rect(place, mi.rcMonitor, mi.rcWork, scale, if frames { 0 } else { INSET });
            // SAFETY: our window; show without activating so focus goes to the real one.
            unsafe {
                SetWindowPos(hwnd, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                UpdateWindow(hwnd);
            }
            painted();
            let _ = tx.send((hwnd as isize, w, h, scale));
            // SAFETY: plain-old-data message struct; standard loop until WM_QUIT.
            let mut msg: MSG = unsafe { zeroed() };
            // SAFETY: `msg` is a writable MSG; null = any window of this thread.
            while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
                // SAFETY: a message just retrieved for this thread.
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if let Ok(mut g) = FRAME.lock() {
                *g = None;
            }
        })
        .ok()?;
    let (hwnd, w, h, scale) = rx.recv_timeout(std::time::Duration::from_millis(500)).ok().filter(|r| r.0 != 0)?;
    Some(Splash { hwnd: AtomicIsize::new(hwnd), size: (w, h), scale })
}

/// How far the logo placeholder is inset from the real window's rectangle, so the real
/// window always covers it.
const INSET: i32 = 6;

/// Physical rectangle `(x, y, w, h)` of the placeholder: the real window's expected
/// rectangle, inset by `inset` pixels on every side.
pub(crate) fn rect(place: Place, monitor: RECT, work: RECT, scale: f32, inset: i32) -> (i32, i32, i32, i32) {
    match place {
        Place::Maximized => (
            work.left + inset,
            work.top + inset,
            work.right - work.left - 2 * inset,
            work.bottom - work.top - 2 * inset,
        ),
        Place::At { x, y, width, height } => {
            let px = |v: f32| (v * scale).round() as i32;
            (px(x) + inset, px(y) + inset, px(width) - 2 * inset, px(height) - 2 * inset)
        }
        Place::Centered { width, height } => {
            let mw = monitor.right - monitor.left;
            let mh = monitor.bottom - monitor.top;
            let w = ((width * scale).round() as i32).min(mw);
            let h = ((height * scale).round() as i32).min(mh);
            (monitor.left + (mw - w) / 2 + inset, monitor.top + (mh - h) / 2 + inset, w - 2 * inset, h - 2 * inset)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_rect_matches_eframe_centering() {
        let mon = RECT { left: 0, top: 0, right: 2560, bottom: 1440 };
        let work = RECT { left: 0, top: 0, right: 2560, bottom: 1392 };
        // 1280 x 800 points at 150 %: 1920 x 1200 px, centered, inset by 6 px.
        assert_eq!(
            rect(Place::Centered { width: 1280.0, height: 800.0 }, mon, work, 1.5, INSET),
            (326, 126, 1908, 1188)
        );
        assert_eq!(rect(Place::Maximized, mon, work, 1.5, INSET), (6, 6, 2548, 1380));
        // Frames mode: the exact client rectangle.
        assert_eq!(rect(Place::Centered { width: 1280.0, height: 800.0 }, mon, work, 1.5, 0), (320, 120, 1920, 1200));
        let at = Place::At { x: -4000.0, y: -4000.0, width: 1280.0, height: 800.0 };
        assert_eq!(rect(at, mon, work, 1.5, 0), (-6000, -6000, 1920, 1200));
    }

    #[test]
    fn pointer_messages_map_to_input() {
        let lp = ((-5i16 as u16 as isize) << 16) | 300;
        assert_eq!(pointer_input(WM_LBUTTONDOWN, 0, lp), Some(Input::Button { x: 300, y: -5, button: 0, down: true }));
        assert_eq!(pointer_input(WM_MOUSEMOVE, 0, 0x0010_0020), Some(Input::Move { x: 0x20, y: 0x10 }));
        assert_eq!(
            pointer_input(WM_MOUSEWHEEL, (-240i16 as u16 as usize) << 16, 0),
            Some(Input::Wheel { notches: -2.0 })
        );
        assert_eq!(pointer_input(WM_PAINT, 0, 0), None);
    }

    #[test]
    fn logo_is_composited_over_the_background() {
        // Opaque red, transparent pixel, half-transparent white over a dark background.
        let rgba = [255, 0, 0, 255, 9, 9, 9, 0, 255, 255, 255, 128];
        let out = composite(&rgba, [16, 17, 21]);
        assert_eq!(&out[..4], &[0, 0, 255, 255]);
        assert_eq!(&out[4..8], &[21, 17, 16, 255]);
        assert_eq!(&out[8..12], &[138, 136, 136, 255]);
    }
}
