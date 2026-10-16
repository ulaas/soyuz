//! Native window integration for the custom title bar, Windows 11 only:
//!
//! * rounded DWM corners, with the close button's top-right corner rounded to match;
//! * `[appearance] backdrop = none | mica | acrylic`: a system backdrop behind translucent
//!   window / sidebar fills. The window has to be created transparent (and wgpu has to
//!   present through a DirectComposition visual), so the choice is read at startup;
//! * `[appearance] snap_layouts`: the maximize button reports `HTMAXBUTTON` to the OS
//!   (comctl32 window subclass) so hovering it opens the Snap Layouts flyout.
//!
//! Everything is a no-op on other platforms. The raw calls live in `soyuz_fs::chrome`.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use egui::{Context, Rect};
use soyuz_core::config::{Backdrop, Config};

/// DWM corner radius of a top-level window on Windows 11 (points).
pub const DWM_CORNER_RADIUS: u8 = 8;

static HWND: AtomicIsize = AtomicIsize::new(0);
static BACKDROP_ACTIVE: AtomicBool = AtomicBool::new(false);
static SNAP_ACTIVE: AtomicBool = AtomicBool::new(false);
static RESTORE_MAXIMIZED: AtomicBool = AtomicBool::new(false);
static WIN11: OnceLock<bool> = OnceLock::new();

/// Windows 11 (rounded window corners, Snap Layouts).
pub fn is_windows_11() -> bool {
    *WIN11.get_or_init(soyuz_fs::chrome::is_windows_11)
}

/// The backdrop to create the window for: `Some` only on Windows 11 22H2+ with a
/// material configured. `main.rs` makes the window transparent for it.
pub fn startup_backdrop(config: &Config) -> Option<Backdrop> {
    if !cfg!(windows) || config.appearance.backdrop == Backdrop::None {
        return None;
    }
    (soyuz_fs::chrome::os_build() >= 22621).then_some(config.appearance.backdrop)
}

/// A system backdrop is showing: window fills are translucent and the clear color is
/// fully transparent.
pub fn backdrop_active() -> bool {
    BACKDROP_ACTIVE.load(Ordering::Relaxed)
}

/// The OS draws the maximize button state (Snap Layouts subclass installed).
pub fn snap_active() -> bool {
    SNAP_ACTIVE.load(Ordering::Relaxed)
}

/// Mark the backdrop as active before the theme is installed (startup only).
pub fn set_backdrop_active(on: bool) {
    BACKDROP_ACTIVE.store(on, Ordering::Relaxed);
}

/// The saved window state is maximized, but the window is created restored: winit shows a
/// hidden window while maximizing it (with eframe's restore steps three blank flashes and
/// taskbar buttons before the first frame), so [`init`] maximizes it out of sight instead.
pub fn set_restore_maximized(on: bool) {
    RESTORE_MAXIMIZED.store(on, Ordering::Relaxed);
}

/// Attach to the native window (call from `SoyuzApp::new`). `transparent`: the window
/// was created for [`startup_backdrop`].
pub fn init(cc: &eframe::CreationContext<'_>, config: &Config, transparent: bool) {
    let Some(hwnd) = win32_hwnd(cc) else { return };
    HWND.store(hwnd, Ordering::Relaxed);
    if RESTORE_MAXIMIZED.swap(false, Ordering::Relaxed) {
        let ok = soyuz_fs::chrome::maximize_hidden(hwnd);
        log::debug!("restored maximized: {ok}");
    }
    if is_windows_11() {
        soyuz_fs::chrome::set_corner_preference(hwnd, Some(true));
    }
    let dark = cc.egui_ctx.theme() == egui::Theme::Dark;
    soyuz_fs::chrome::set_dark_mode(hwnd, dark);
    if transparent && let Some(b) = startup_backdrop(config) {
        let kind = match b {
            Backdrop::Mica => soyuz_fs::chrome::Backdrop::Mica,
            Backdrop::Acrylic => soyuz_fs::chrome::Backdrop::Acrylic,
            Backdrop::None => soyuz_fs::chrome::Backdrop::None,
        };
        let ok = soyuz_fs::chrome::set_backdrop(hwnd, kind);
        log::debug!("DWM backdrop {b:?}: {ok}");
        if !ok {
            log::warn!("system backdrop {b:?} could not be applied");
            set_backdrop_active(false);
        }
    }
    let snap = config.appearance.snap_layouts && config.appearance.custom_titlebar && is_windows_11();
    if snap || backdrop_active() {
        let ctx = cc.egui_ctx.clone();
        let ok =
            soyuz_fs::chrome::install_window_hooks(hwnd, backdrop_active(), Box::new(move || ctx.request_repaint()));
        SNAP_ACTIVE.store(ok && snap, Ordering::Relaxed);
    }
}

#[cfg(windows)]
fn win32_hwnd(cc: &eframe::CreationContext<'_>) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match cc.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

#[cfg(not(windows))]
fn win32_hwnd(_cc: &eframe::CreationContext<'_>) -> Option<isize> {
    None
}

/// The main window (0 before [`init`] or off Windows).
pub fn main_hwnd() -> isize {
    HWND.load(Ordering::Relaxed)
}

/// Per-frame sync: follow the theme for the DWM frame tint.
pub fn sync_theme(ctx: &Context) {
    let hwnd = HWND.load(Ordering::Relaxed);
    if hwnd == 0 {
        return;
    }
    let dark = ctx.theme() == egui::Theme::Dark;
    let key = egui::Id::unique("soyuz.native.dark");
    if ctx.data(|d| d.get_temp::<bool>(key)) != Some(dark) {
        ctx.data_mut(|d| d.insert_temp(key, dark));
        let ok = soyuz_fs::chrome::set_dark_mode(hwnd, dark);
        log::debug!("DWM dark mode {dark}: {ok}");
    }
}

/// Publish the maximize button rect (`None`: no own controls this frame) and return the
/// OS-side `(hovered, pressed, clicked)` state.
pub fn max_button(ctx: &Context, rect: Option<Rect>, enabled: bool) -> (bool, bool, bool) {
    if !snap_active() {
        return (false, false, false);
    }
    let ppp = ctx.pixels_per_point();
    let px = rect.filter(|_| enabled).map(|r| {
        [
            (r.left() * ppp).round() as i32,
            (r.top() * ppp).round() as i32,
            (r.right() * ppp).round() as i32,
            (r.bottom() * ppp).round() as i32,
        ]
    });
    soyuz_fs::chrome::set_max_button_rect(px);
    soyuz_fs::chrome::max_button_state()
}
