//! Native window-chrome helpers for the app's custom title bar (roadmap B1).
//!
//! The raw calls live in `sys/windows/chrome.rs` (dwmapi / user32 / comctl32, no shell
//! APIs) because `unsafe` is confined to `sys/*`. Every function takes the window as a
//! raw `HWND` value and is a harmless no-op (returning `false`) on other platforms and
//! on Windows versions without the feature.

/// System backdrop material (`DWMWA_SYSTEMBACKDROP_TYPE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    None,
    Mica,
    Acrylic,
}

/// Windows build number (0 when unknown or not on Windows).
pub fn os_build() -> u32 {
    #[cfg(windows)]
    {
        crate::sys::windows::chrome::os_build()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Windows 11 or later (rounded corners, system backdrops, Snap Layouts).
pub fn is_windows_11() -> bool {
    os_build() >= 22000
}

/// Maximize a window that has not been shown yet without it appearing (Windows; see
/// `sys::windows::chrome::maximize_hidden`). Returns whether it is maximized now.
pub fn maximize_hidden(hwnd: isize) -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::chrome::maximize_hidden(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}

/// Corner preference: `Some(true)` round, `Some(false)` square, `None` system default.
pub fn set_corner_preference(hwnd: isize, round: Option<bool>) -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::chrome::set_corner_preference(hwnd, round)
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, round);
        false
    }
}

/// Dark frame / backdrop tint.
pub fn set_dark_mode(hwnd: isize, dark: bool) -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::chrome::set_dark_mode(hwnd, dark)
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, dark);
        false
    }
}

/// Apply a system backdrop (Windows 11 22H2+, build 22621). The window must be created
/// transparent for the material to show through.
pub fn set_backdrop(hwnd: isize, kind: Backdrop) -> bool {
    #[cfg(windows)]
    {
        if os_build() < 22621 {
            return false;
        }
        let v = match kind {
            Backdrop::None => 1,
            Backdrop::Mica => 2,
            Backdrop::Acrylic => 3,
        };
        crate::sys::windows::chrome::set_backdrop(hwnd, v)
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, kind);
        false
    }
}

/// Install the window subclass (call on the window thread): the Snap Layouts hit test
/// (active once [`set_max_button_rect`] publishes a rect) and, with `backdrop`, a black
/// GDI background so an extended frame shows the system backdrop. `wake` runs when the
/// OS-level maximize button changes state.
pub fn install_window_hooks(hwnd: isize, backdrop: bool, wake: Box<dyn Fn() + Send + Sync>) -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::chrome::install_subclass(hwnd, backdrop, wake)
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, backdrop, wake);
        false
    }
}

/// The maximize button rect in client pixels `[left, top, right, bottom]`; `None` turns
/// the hit test off (e.g. while a different chrome is active).
pub fn set_max_button_rect(rect: Option<[i32; 4]>) {
    #[cfg(windows)]
    crate::sys::windows::chrome::set_max_button_rect(rect);
    #[cfg(not(windows))]
    let _ = rect;
}

/// State of the OS-level maximize button: `(hovered, pressed, clicked since last call)`.
pub fn max_button_state() -> (bool, bool, bool) {
    #[cfg(windows)]
    {
        crate::sys::windows::chrome::max_button_state()
    }
    #[cfg(not(windows))]
    {
        (false, false, false)
    }
}

// --- Owned windows (native dialogs / popup menus) ------------------------------------

/// Owned top-level windows of the app's native dialogs and popup menus. Rects are
/// `[left, top, right, bottom]` (or `[x, y, w, h]` where noted) in physical pixels.
/// Every function is a no-op (`None` / `false` / `0`) off Windows.
pub mod owned {
    #[cfg(windows)]
    use crate::sys::windows::owned as sys;

    macro_rules! forward {
        ($(#[$m:meta])* fn $name:ident($($a:ident: $t:ty),*) -> $r:ty = $default:expr) => {
            $(#[$m])*
            pub fn $name($($a: $t),*) -> $r {
                #[cfg(windows)]
                {
                    sys::$name($($a),*)
                }
                #[cfg(not(windows))]
                {
                    $(let _ = $a;)*
                    $default
                }
            }
        };
    }

    forward!(
        /// The first top-level window of the calling thread titled exactly `title`.
        fn find_thread_window(title: &str) -> Option<isize> = None
    );
    forward!(
        /// Top-level windows created by the calling thread.
        fn thread_windows() -> Vec<isize> = Vec::new()
    );
    forward!(
        /// Outer window rect `[l, t, r, b]`.
        fn window_rect(hwnd: isize) -> Option<[i32; 4]> = None
    );
    forward!(
        /// Make `hwnd` owned by `owner` (no taskbar button, above its owner, minimized with it).
        fn set_owner(hwnd: isize, owner: isize) -> bool = false
    );
    forward!(
        /// Current owner window (0: none).
        fn owner(hwnd: isize) -> isize = 0
    );
    forward!(
        /// Tool-window style (no taskbar button, not in Alt+Tab).
        fn set_tool_window(hwnd: isize, on: bool) -> bool = false
    );
    forward!(
        /// Move to `[x, y, w, h]` (`size = false`: keep the size) without activating.
        fn set_window_rect(hwnd: isize, rect: [i32; 4], size: bool) -> bool = false
    );
    forward!(
        /// Raise within its z-order band.
        fn bring_to_top(hwnd: isize) -> () = ()
    );
    forward!(
        /// Show; `activate` gives it the keyboard focus.
        fn show_window(hwnd: isize, activate: bool) -> () = ()
    );
    forward!(fn hide_window(hwnd: isize) -> () = ());
    forward!(fn is_visible(hwnd: isize) -> bool = false);
    forward!(fn is_window(hwnd: isize) -> bool = false);
    forward!(fn is_minimized(hwnd: isize) -> bool = false);
    forward!(
        /// Enable / disable input (modal owners).
        fn enable_window(hwnd: isize, on: bool) -> () = ()
    );
    forward!(fn is_enabled(hwnd: isize) -> bool = true);
    forward!(
        /// The calling thread's active window (0: none).
        fn active_window() -> isize = 0
    );
    forward!(
        /// DPI of the window's monitor (96 = 100 %; 0 unknown).
        fn window_dpi(hwnd: isize) -> u32 = 0
    );
    forward!(
        /// `(work area, monitor rect)` `[l, t, r, b]` of the monitor at / nearest to `(x, y)`.
        fn monitor_at(x: i32, y: i32) -> Option<([i32; 4], [i32; 4])> = None
    );
    forward!(
        /// Small rounded DWM corners (Windows 11 menus).
        fn set_small_corners(hwnd: isize) -> bool = false
    );
}
