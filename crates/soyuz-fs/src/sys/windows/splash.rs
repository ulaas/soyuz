//! Startup placeholder window (roadmap R5): a plain user32 / GDI popup painted with the
//! theme background and the logo within a few tens of milliseconds of process start,
//! while the GPU renderer initialises (the D3D12 driver load alone takes ~200 ms). It
//! lives on its own thread with its own message loop and is closed once the real window
//! has presented its first frame (the real window then covers it, so closing it is not
//! visible). No shell API is involved.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, CreateSolidBrush, DIB_RGB_COLORS, DeleteObject, EndPaint,
    FillRect, GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, PAINTSTRUCT, SRCCOPY,
    StretchDIBits, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IDC_APPSTARTING, LoadCursorW, MSG,
    PostMessageW, PostQuitMessage, RegisterClassExW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    ShowWindow, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WNDCLASSEXW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::wide_str;

/// Where the placeholder goes (the real window's expected rectangle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    /// Centered on the primary monitor, this inner size in points (eframe `centered`).
    Centered { width: f32, height: f32 },
    /// The work area of the primary monitor (a maximized window).
    Maximized,
}

struct Paint {
    /// RGB background.
    bg: [u8; 3],
    /// Logo pre-composited over the background: BGRA, top-down.
    logo: Option<(Vec<u8>, i32, i32)>,
}

static PAINT: Mutex<Option<Paint>> = Mutex::new(None);

/// The open placeholder window.
pub(crate) struct Splash {
    hwnd: AtomicIsize,
}

impl Splash {
    /// Close the placeholder (from any thread).
    pub(crate) fn close(&self) {
        let h = self.hwnd.swap(0, Ordering::SeqCst);
        if h != 0 {
            // SAFETY: posting to a window handle owned by the splash thread; a stale
            // handle only makes the call fail.
            unsafe { PostMessageW(h as HWND, WM_CLOSE, 0, 0) };
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            // SAFETY: plain-old-data out-struct for BeginPaint.
            let mut ps: PAINTSTRUCT = unsafe { zeroed() };
            // SAFETY: `hwnd` is the window being painted; `ps` lives across Begin/EndPaint.
            let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
            if let Ok(guard) = PAINT.lock()
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
                    // SAFETY: plain-old-data header; negative height = top-down rows.
                    let mut bmi: BITMAPINFO = unsafe { zeroed() };
                    bmi.bmiHeader = BITMAPINFOHEADER {
                        biSize: size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: *w,
                        biHeight: -*h,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB,
                        // SAFETY: the remaining header fields are plain integers.
                        ..unsafe { zeroed() }
                    };
                    // SAFETY: `bits` holds w * h BGRA pixels as described by `bmi`.
                    unsafe {
                        StretchDIBits(
                            hdc,
                            x,
                            y,
                            *w,
                            *h,
                            0,
                            0,
                            *w,
                            *h,
                            bits.as_ptr().cast(),
                            &bmi,
                            DIB_RGB_COLORS,
                            SRCCOPY,
                        )
                    };
                }
            }
            // SAFETY: matches the BeginPaint above.
            unsafe { EndPaint(hwnd, &ps) };
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
/// Returns `None` when the window cannot be created.
pub(crate) fn open(
    place: Place,
    bg: [u8; 3],
    logo: Option<(Vec<u8>, u32, u32)>,
    painted: Box<dyn FnOnce() + Send>,
) -> Option<Splash> {
    if let Ok(mut g) = PAINT.lock() {
        *g = Some(Paint { bg, logo: logo.map(|(rgba, w, h)| (composite(&rgba, bg), w as i32, h as i32)) });
    }
    let (tx, rx) = mpsc::channel::<isize>();
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
                // SAFETY: a system cursor (null instance).
                hCursor: unsafe { LoadCursorW(null_mut(), IDC_APPSTARTING) },
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
                let _ = tx.send(0);
                return;
            }
            // SAFETY: plain-old-data out-struct with its size set.
            let mut mi: MONITORINFO = unsafe { zeroed() };
            mi.cbSize = size_of::<MONITORINFO>() as u32;
            // SAFETY: primary monitor handle, writable MONITORINFO.
            unsafe { GetMonitorInfoW(MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY), &mut mi) };
            // SAFETY: our window, on the primary monitor.
            let scale = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
            let (x, y, w, h) = rect(place, mi.rcMonitor, mi.rcWork, scale);
            // SAFETY: our window; show without activating so focus goes to the real one.
            unsafe {
                SetWindowPos(hwnd, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                UpdateWindow(hwnd);
            }
            painted();
            let _ = tx.send(hwnd as isize);
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
        })
        .ok()?;
    let hwnd = rx.recv_timeout(std::time::Duration::from_millis(500)).ok().filter(|h| *h != 0)?;
    Some(Splash { hwnd: AtomicIsize::new(hwnd) })
}

/// Physical rectangle `(x, y, w, h)` of the placeholder. It is inset by a few pixels
/// from the real window's expected rectangle so the real window always covers it.
pub(crate) fn rect(place: Place, monitor: RECT, work: RECT, scale: f32) -> (i32, i32, i32, i32) {
    const INSET: i32 = 6;
    match place {
        Place::Maximized => (
            work.left + INSET,
            work.top + INSET,
            work.right - work.left - 2 * INSET,
            work.bottom - work.top - 2 * INSET,
        ),
        Place::Centered { width, height } => {
            let mw = monitor.right - monitor.left;
            let mh = monitor.bottom - monitor.top;
            let w = ((width * scale).round() as i32).min(mw);
            let h = ((height * scale).round() as i32).min(mh);
            (monitor.left + (mw - w) / 2 + INSET, monitor.top + (mh - h) / 2 + INSET, w - 2 * INSET, h - 2 * INSET)
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
        assert_eq!(rect(Place::Centered { width: 1280.0, height: 800.0 }, mon, work, 1.5), (326, 126, 1908, 1188));
        assert_eq!(rect(Place::Maximized, mon, work, 1.5), (6, 6, 2548, 1380));
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
