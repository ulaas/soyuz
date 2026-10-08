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
