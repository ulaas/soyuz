//! The OS appearance settings the app follows: the accent color and the dark / light
//! app mode.
//!
//! * Windows: the registry (advapi32, read-only).
//! * Linux: the XDG desktop portal (`org.freedesktop.portal.Settings`, namespace
//!   `org.freedesktop.appearance`, keys `color-scheme` and `accent-color`), read once and
//!   then kept current by a background thread listening to `SettingChanged`, so the
//!   app's 1.5 s poll only reads two atomics. Without a portal (or when it has no
//!   preference) KDE's `kdeglobals` and GTK's `settings.ini` are read instead (cached by
//!   modification time).
//! * macOS: the global preferences domain through CoreFoundation
//!   (`CFPreferencesCopyAppValue(…, kCFPreferencesAnyApplication)` after
//!   `CFPreferencesAppSynchronize`, cheap enough for the 1.5 s poll): `AppleInterfaceStyle`
//!   and `AppleAccentColor` (untested on macOS).
//! * Other platforms report `None` (the app then keeps its own defaults).

/// The system accent color as `[r, g, b]`.
///
/// Windows: `HKCU\Software\Microsoft\Windows\DWM\AccentColor` (`0xAABBGGRR`), falling
/// back to `ColorizationColor` (`0xAARRGGBB`). Linux: the portal's `accent-color`, else
/// KDE's `[General] AccentColor`. macOS: `AppleAccentColor` mapped to the system palette
/// (absent = "Multicolor", where every app keeps its own accent → `None`).
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
    #[cfg(target_os = "linux")]
    {
        linux::accent_color()
    }
    #[cfg(target_os = "macos")]
    {
        crate::sys::macos::cf::global_int("AppleAccentColor", true).and_then(mac_accent_rgb)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// `Some(true)` when apps should use the light theme, `Some(false)` for dark.
///
/// Windows: `HKCU\…\Themes\Personalize\AppsUseLightTheme`. Linux: the portal's
/// `color-scheme` (1 = dark, 2 = light; 0 = no preference falls through to the files),
/// else KDE's color scheme and GTK's `gtk-application-prefer-dark-theme` /
/// `-dark` theme name. macOS: `AppleInterfaceStyle` is `Dark` in dark mode (also while
/// "Auto" has switched to dark) and absent in light mode.
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
    #[cfg(target_os = "linux")]
    {
        linux::apps_use_light_theme()
    }
    #[cfg(target_os = "macos")]
    {
        let style = crate::sys::macos::cf::global_string("AppleInterfaceStyle", true);
        Some(!style.is_some_and(|s| s.eq_ignore_ascii_case("dark")))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// macOS `AppleAccentColor` (-1 graphite, 0 red, 1 orange, 2 yellow, 3 green, 4 blue,
/// 5 purple, 6 pink) → the system accent color (light-appearance values).
pub fn mac_accent_rgb(v: i64) -> Option<[u8; 3]> {
    Some(match v {
        -1 => [0x98, 0x98, 0x98],
        0 => [0xE0, 0x38, 0x3E],
        1 => [0xF7, 0x82, 0x1B],
        2 => [0xFF, 0xC7, 0x26],
        3 => [0x62, 0xBA, 0x46],
        4 => [0x00, 0x7A, 0xFF],
        5 => [0x95, 0x3D, 0x96],
        6 => [0xF7, 0x4F, 0x9E],
        _ => return None,
    })
}

/// `0xAABBGGRR` (DWM `AccentColor`) → `[r, g, b]`.
pub fn abgr_to_rgb(v: u32) -> [u8; 3] {
    [v as u8, (v >> 8) as u8, (v >> 16) as u8]
}

/// `0xAARRGGBB` (DWM `ColorizationColor`) → `[r, g, b]`.
pub fn argb_to_rgb(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// Portal `accent-color` (`(ddd)`, each in 0..=1; out of range = unset) → `[r, g, b]`.
pub fn portal_rgb(r: f64, g: f64, b: f64) -> Option<[u8; 3]> {
    let ok = |c: f64| (0.0..=1.0).contains(&c);
    (ok(r) && ok(g) && ok(b)).then(|| [r, g, b].map(|c| (c * 255.0).round() as u8))
}

/// Settings parsed from config files (pure; tested everywhere).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod files {
    /// `key=value` of `[group]` in an INI-style file.
    pub(crate) fn ini(text: &str, group: &str, key: &str) -> Option<String> {
        let mut in_group = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_group = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) == Some(group);
                continue;
            }
            if in_group
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == key
            {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
        None
    }

    fn rgb_list(v: &str) -> Option<[u8; 3]> {
        let mut it = v.split(',').map(|c| c.trim().parse::<u8>().ok());
        Some([it.next()??, it.next()??, it.next()??])
    }

    /// KDE `kdeglobals`: `(light?, accent)`. The mode comes from the color scheme's
    /// name, else from the luminance of `[Colors:Window] BackgroundNormal`.
    pub(crate) fn kde(text: &str) -> (Option<bool>, Option<[u8; 3]>) {
        let accent = ini(text, "General", "AccentColor").and_then(|v| rgb_list(&v));
        let light = match ini(text, "General", "ColorScheme") {
            Some(s) if !s.is_empty() => Some(!s.to_ascii_lowercase().contains("dark")),
            _ => ini(text, "Colors:Window", "BackgroundNormal")
                .and_then(|v| rgb_list(&v))
                .map(|[r, g, b]| 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b) >= 128.0),
        };
        (light, accent)
    }

    /// GTK `settings.ini`: `Some(false)` for a dark preference or a `*-dark` theme.
    pub(crate) fn gtk(text: &str) -> Option<bool> {
        if let Some(v) = ini(text, "Settings", "gtk-application-prefer-dark-theme") {
            match v.to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" => return Some(false),
                "0" | "false" | "no" => {}
                _ => {}
            }
        }
        let theme = ini(text, "Settings", "gtk-theme-name")?;
        let t = theme.to_ascii_lowercase();
        Some(!(t.ends_with("-dark") || t.ends_with(":dark") || t.contains("-dark-")))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;
    use std::sync::Once;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant, SystemTime};

    use zbus::zvariant::{OwnedValue, Value};

    use super::files;
    use crate::launch::linux::bus;

    /// `color-scheme`: `u32::MAX` = unknown (no portal yet / failed), else 0 / 1 / 2.
    static SCHEME: AtomicU32 = AtomicU32::new(u32::MAX);
    /// `accent-color` as `0x01RRGGBB`; 0 = unset.
    static ACCENT: AtomicU32 = AtomicU32::new(0);
    static START: Once = Once::new();

    const DEST: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const IFACE: &str = "org.freedesktop.portal.Settings";
    const NS: &str = "org.freedesktop.appearance";

    /// Unwrap nested variants (`Read` wraps the value twice).
    fn inner<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
        match v {
            Value::Value(b) => inner(b),
            o => o,
        }
    }

    fn apply(key: &str, v: &Value<'_>) {
        match (key, inner(v)) {
            ("color-scheme", Value::U32(s)) => SCHEME.store(*s, Ordering::Relaxed),
            ("accent-color", Value::Structure(st)) => {
                let f: Vec<f64> = st
                    .fields()
                    .iter()
                    .filter_map(|x| if let Value::F64(d) = inner(x) { Some(*d) } else { None })
                    .collect();
                let rgb = (f.len() == 3).then(|| super::portal_rgb(f[0], f[1], f[2])).flatten();
                ACCENT.store(
                    rgb.map_or(0, |[r, g, b]| 0x0100_0000 | u32::from_be_bytes([0, r, g, b])),
                    Ordering::Relaxed,
                );
            }
            _ => {}
        }
    }

    fn read_one(conn: &zbus::blocking::Connection, key: &str) -> Option<OwnedValue> {
        let call = |m: &str| -> Option<OwnedValue> {
            conn.call_method(Some(DEST), PATH, Some(IFACE), m, &(NS, key)).ok()?.body().deserialize::<OwnedValue>().ok()
        };
        call("ReadOne").or_else(|| call("Read"))
    }

    /// Read the current values, then follow `SettingChanged` until the bus goes away.
    fn watch() {
        let Some(conn) = bus::session() else { return };
        for key in ["color-scheme", "accent-color"] {
            if let Some(v) = read_one(&conn, key) {
                apply(key, &v);
            }
        }
        let rule = format!("type='signal',interface='{IFACE}',member='SettingChanged',path='{PATH}',arg0='{NS}'");
        let Ok(rule) = zbus::MatchRule::try_from(rule.as_str()) else { return };
        let Ok(iter) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(16)) else { return };
        for msg in iter {
            let Ok(msg) = msg else { break };
            if let Ok((ns, key, v)) = msg.body().deserialize::<(String, String, OwnedValue)>()
                && ns == NS
            {
                apply(&key, &v);
            }
        }
    }

    fn start() {
        START.call_once(|| {
            let _ = std::thread::Builder::new().name("soyuz-os-theme".into()).spawn(watch);
        });
    }

    /// File-based fallback values, re-read when a file's mtime changes (checked at most
    /// once a second).
    #[derive(Default)]
    struct FileCache {
        checked: Option<Instant>,
        stamps: Vec<Option<SystemTime>>,
        light: Option<bool>,
        accent: Option<[u8; 3]>,
    }

    static FILES: parking_lot::Mutex<Option<FileCache>> = parking_lot::Mutex::new(None);

    fn paths() -> Vec<PathBuf> {
        let c = crate::launch::linux::config_home();
        vec![c.join("kdeglobals"), c.join("gtk-4.0/settings.ini"), c.join("gtk-3.0/settings.ini")]
    }

    fn from_files() -> (Option<bool>, Option<[u8; 3]>) {
        let mut g = FILES.lock();
        let c = g.get_or_insert_with(FileCache::default);
        if c.checked.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return (c.light, c.accent);
        }
        let ps = paths();
        let stamps: Vec<Option<SystemTime>> =
            ps.iter().map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()).collect();
        if c.checked.is_none() || stamps != c.stamps {
            let read = |p: &PathBuf| std::fs::read_to_string(p).ok();
            let kde = read(&ps[0]).map(|t| files::kde(&t)).unwrap_or_default();
            let gtk = read(&ps[1]).and_then(|t| files::gtk(&t)).or_else(|| read(&ps[2]).and_then(|t| files::gtk(&t)));
            let on_kde =
                crate::launch::linux::desktop::current_desktops().iter().any(|d| d.eq_ignore_ascii_case("KDE"));
            c.light = if on_kde { kde.0.or(gtk) } else { gtk.or(kde.0) };
            c.accent = kde.1;
            c.stamps = stamps;
        }
        c.checked = Some(Instant::now());
        (c.light, c.accent)
    }

    pub(super) fn apps_use_light_theme() -> Option<bool> {
        start();
        match SCHEME.load(Ordering::Relaxed) {
            1 => Some(false),
            2 => Some(true),
            _ => from_files().0,
        }
    }

    pub(super) fn accent_color() -> Option<[u8; 3]> {
        start();
        match ACCENT.load(Ordering::Relaxed) {
            0 => from_files().1,
            v => {
                let [_, r, g, b] = v.to_be_bytes();
                Some([r, g, b])
            }
        }
    }

    /// Whether the portal answered (tests / diagnostics).
    #[cfg(test)]
    pub(super) fn portal_seen() -> bool {
        SCHEME.load(Ordering::Relaxed) != u32::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_accents() {
        assert_eq!(mac_accent_rgb(4), Some([0x00, 0x7A, 0xFF]));
        assert_eq!(mac_accent_rgb(-1), Some([0x98, 0x98, 0x98]));
        assert_eq!(mac_accent_rgb(7), None);
    }

    #[test]
    fn byte_orders() {
        assert_eq!(abgr_to_rgb(0xFFD77800), [0x00, 0x78, 0xD7]);
        assert_eq!(argb_to_rgb(0xC40078D7), [0x00, 0x78, 0xD7]);
        assert_eq!(portal_rgb(1.0, 0.5, 0.0), Some([255, 128, 0]));
        assert_eq!(portal_rgb(-1.0, -1.0, -1.0), None, "unset accent");
    }

    #[test]
    fn config_files() {
        let kde = "[General]\nColorScheme=BreezeDark\nAccentColor=61,174,233\n\n[Colors:Window]\nBackgroundNormal=239,240,241\n";
        assert_eq!(files::kde(kde), (Some(false), Some([61, 174, 233])));
        assert_eq!(files::kde("[Colors:Window]\nBackgroundNormal=239,240,241\n"), (Some(true), None));
        assert_eq!(files::kde("[Colors:Window]\nBackgroundNormal=32,35,38\n").0, Some(false));
        assert_eq!(files::kde(""), (None, None));
        assert_eq!(files::gtk("[Settings]\ngtk-application-prefer-dark-theme=1\n"), Some(false));
        assert_eq!(
            files::gtk("[Settings]\ngtk-application-prefer-dark-theme=0\ngtk-theme-name=Adwaita-dark\n"),
            Some(false)
        );
        assert_eq!(files::gtk("[Settings]\ngtk-theme-name=Adwaita\n"), Some(true));
        assert_eq!(files::gtk("[Settings]\n"), None);
    }

    #[cfg(windows)]
    #[test]
    fn reads_without_panicking() {
        // Values may be missing on CI images; only the call itself is exercised.
        let _ = accent_color();
        let _ = apps_use_light_theme();
    }

    /// `SOYUZ_DBUS_TESTS=1`: read the real portal settings of this session.
    #[cfg(target_os = "linux")]
    #[test]
    fn portal_settings_real() {
        if std::env::var_os("SOYUZ_DBUS_TESTS").is_none() {
            return;
        }
        let _ = apps_use_light_theme();
        let t = std::time::Instant::now();
        while !linux::portal_seen() && t.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        println!(
            "portal answered: {}, light: {:?}, accent: {:?}",
            linux::portal_seen(),
            apps_use_light_theme(),
            accent_color()
        );
        assert!(linux::portal_seen(), "no answer from org.freedesktop.portal.Settings");
    }
}
