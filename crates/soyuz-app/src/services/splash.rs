//! Startup placeholder (roadmap R5, `[appearance] startup_placeholder`, Windows).
//!
//! Creating the GPU renderer costs ~200 ms before the first frame on this hardware (the
//! D3D12 user-mode driver load during adapter enumeration), and eframe keeps the window
//! hidden until that first frame. To show *something* right away, `main` opens a plain
//! native window (`soyuz_fs::splash`: user32 / GDI on its own thread) with the theme's
//! window background and the logo, at the rectangle the real window will take. It is
//! closed when the real window has presented its first frame ([`close`], called from
//! the startup frame counter in `devscript::begin`); the real window opens on top of it
//! and the placeholder is inset by a few pixels, so closing it is not visible.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde::Deserialize;
use soyuz_core::config::Config;
use soyuz_fs::splash::{Place, Splash};

static SPLASH: Mutex<Option<Splash>> = Mutex::new(None);

/// eframe's saved window state (`app.ron`, key `window`), the fields we need.
#[derive(Deserialize, Default)]
struct WindowState {
    #[serde(default)]
    fullscreen: bool,
    #[serde(default)]
    maximized: bool,
    #[serde(default)]
    inner_size_points: Option<Size>,
}

#[derive(Deserialize)]
struct Size {
    x: f32,
    y: f32,
}

/// Where eframe will put the window: `None` = don't show a placeholder (fullscreen).
pub fn place_from_app_ron(text: Option<&str>, default: (f32, f32)) -> Option<Place> {
    let state = text
        .and_then(|t| ron::from_str::<HashMap<String, String>>(t).ok())
        .and_then(|m| m.get("window").and_then(|w| ron::from_str::<WindowState>(w).ok()))
        .unwrap_or_default();
    if state.fullscreen {
        return None;
    }
    if state.maximized {
        return Some(Place::Maximized);
    }
    let (width, height) = state.inner_size_points.map_or(default, |s| (s.x, s.y));
    Some(Place::Centered { width, height })
}

#[derive(Deserialize)]
struct SessionTheme {
    theme: Option<String>,
}

/// Dark or light, like the app decides it (`session.toml` theme, else `[appearance]
/// theme`, `system` = the OS app theme).
fn is_dark(config: &Config, session: Option<&str>) -> bool {
    let pref = session
        .and_then(|t| toml::from_str::<SessionTheme>(t).ok())
        .and_then(|s| s.theme)
        .unwrap_or_else(|| config.appearance.theme.clone());
    match crate::theme::preference_from_config(&pref) {
        egui::ThemePreference::Dark => true,
        egui::ThemePreference::Light => false,
        egui::ThemePreference::System => !soyuz_fs::os_theme::apps_use_light_theme().unwrap_or(false),
    }
}

/// Show the placeholder (no-op when off, for secondary windows and off Windows).
/// `default_size` is the viewport's default inner size in points.
pub fn open(config: &Config, secondary: bool, default_size: (f32, f32)) {
    if !cfg!(windows) || !config.appearance.startup_placeholder || secondary {
        return;
    }
    let dir = soyuz_core::config::config_dir();
    let app_ron = std::fs::read_to_string(dir.join("app.ron")).ok();
    let Some(place) = place_from_app_ron(app_ron.as_deref(), default_size) else { return };
    let session = std::fs::read_to_string(dir.join("session.toml")).ok();
    let (dark, light, _) = crate::theme::resolve(&config.appearance);
    let p = if is_dark(config, session.as_deref()) { dark } else { light };
    let bg = [p.bg_window.r(), p.bg_window.g(), p.bg_window.b()];
    let icon = crate::app_icon::icon_data(96);
    let painted = Box::new(|| crate::devscript::startup_mark("placeholder painted"));
    let splash = Splash::open(place, bg, Some((icon.rgba, icon.width, icon.height)), painted);
    *SPLASH.lock() = splash;
}

/// Close the placeholder (the real window is visible now). Idempotent.
pub fn close() {
    if let Some(s) = SPLASH.lock().take() {
        s.close();
        crate::devscript::startup_mark("placeholder closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_follows_the_saved_window() {
        assert_eq!(place_from_app_ron(None, (1280.0, 800.0)), Some(Place::Centered { width: 1280.0, height: 800.0 }));
        let saved = r#"{"window":"(inner_position_pixels:Some((x:10.0,y:20.0)),outer_position_pixels:Some((x:10.0,y:20.0)),fullscreen:false,maximized:false,inner_size_points:Some((x:1500.0,y:900.0)))","egui":"()"}"#;
        assert_eq!(
            place_from_app_ron(Some(saved), (1280.0, 800.0)),
            Some(Place::Centered { width: 1500.0, height: 900.0 })
        );
        let max = saved.replace("maximized:false", "maximized:true");
        assert_eq!(place_from_app_ron(Some(&max), (1280.0, 800.0)), Some(Place::Maximized));
        let full = saved.replace("fullscreen:false", "fullscreen:true");
        assert_eq!(place_from_app_ron(Some(&full), (1280.0, 800.0)), None);
        assert_eq!(
            place_from_app_ron(Some("not ron"), (1280.0, 800.0)),
            Some(Place::Centered { width: 1280.0, height: 800.0 })
        );
    }

    #[test]
    fn theme_comes_from_the_session_first() {
        let mut c = Config::default();
        c.appearance.theme = "light".into();
        assert!(!is_dark(&c, None));
        assert!(is_dark(&c, Some("version = 1\ntheme = \"dark\"\n")));
        c.appearance.theme = "dark".into();
        assert!(!is_dark(&c, Some("theme = \"light\"")));
    }
}
