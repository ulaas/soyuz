//! The OS appearance settings the app follows: the accent color and the dark / light
//! app mode. Windows reads them from the registry (advapi32, read-only); other
//! platforms report `None` (the app then keeps its own defaults).

/// The system accent color as `[r, g, b]`.
///
/// Windows: `HKCU\Software\Microsoft\Windows\DWM\AccentColor` (`0xAABBGGRR`), falling
/// back to `ColorizationColor` (`0xAARRGGBB`).
pub fn accent_color() -> Option<[u8; 3]> {
    #[cfg(windows)]
    {
        use crate::sys::windows::registry::{Hive, get_dword};
        const DWM: &str = r"Software\Microsoft\Windows\DWM";
        if let Some(v) = get_dword(Hive::CurrentUser, DWM, "AccentColor") {
            return Some(abgr_to_rgb(v));
        }
        get_dword(Hive::CurrentUser, DWM, "ColorizationColor").map(argb_to_rgb)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// `Some(true)` when apps should use the light theme, `Some(false)` for dark.
///
/// Windows: `HKCU\…\Themes\Personalize\AppsUseLightTheme`.
pub fn apps_use_light_theme() -> Option<bool> {
    #[cfg(windows)]
    {
        use crate::sys::windows::registry::{Hive, get_dword};
        get_dword(
            Hive::CurrentUser,
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "AppsUseLightTheme",
        )
        .map(|v| v != 0)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// `0xAABBGGRR` (DWM `AccentColor`) → `[r, g, b]`.
pub fn abgr_to_rgb(v: u32) -> [u8; 3] {
    [v as u8, (v >> 8) as u8, (v >> 16) as u8]
}

/// `0xAARRGGBB` (DWM `ColorizationColor`) → `[r, g, b]`.
pub fn argb_to_rgb(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_orders() {
        assert_eq!(abgr_to_rgb(0xFFD77800), [0x00, 0x78, 0xD7]);
        assert_eq!(argb_to_rgb(0xC40078D7), [0x00, 0x78, 0xD7]);
    }

    #[cfg(windows)]
    #[test]
    fn reads_without_panicking() {
        // Values may be missing on CI images; only the call itself is exercised.
        let _ = accent_color();
        let _ = apps_use_light_theme();
    }
}
