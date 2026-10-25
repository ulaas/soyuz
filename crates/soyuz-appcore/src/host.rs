//! The frontend seam: what app logic outside `ui/` may ask of the window system.
//!
//! The egui GUI implements [`Host`] on `soyuz_app::host::EguiHost`; headless tests use
//! [`NullHost`]; the terminal UI (roadmap Phase T) gets its own. The host only has side effects: facts that
//! change per frame (a close request, text-field focus) travel with the input instead.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Wake-up callback for background threads: asks the frontend for another frame.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A wake-up that does nothing (tests, and services started before a frontend exists).
pub fn no_wake() -> Wake {
    Arc::new(|| {})
}

/// Light / dark / follow-the-system, without a toolkit type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemePref {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemePref {
    /// `[appearance] theme`: `dark`, `light`, anything else (`system`, a theme name)
    /// follows the system.
    pub fn from_config(theme: &str) -> ThemePref {
        match theme.trim().to_ascii_lowercase().as_str() {
            "dark" => ThemePref::Dark,
            "light" => ThemePref::Light,
            _ => ThemePref::System,
        }
    }

    /// The name stored in the session.
    pub fn name(self) -> &'static str {
        match self {
            ThemePref::Dark => "dark",
            ThemePref::Light => "light",
            ThemePref::System => "system",
        }
    }
}

/// Which window a [`WindowCmd`] is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowId {
    /// The window the current frame belongs to.
    #[default]
    Current,
    /// The main (root) window of the process.
    Root,
    /// A window of this process (`MAIN` is the root window).
    Window(crate::state::workspace_windows::WindowId),
}

/// A request to the window system.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowCmd {
    Close,
    /// Veto the close request of this frame.
    CancelClose,
    Focus,
    /// Un-minimize and bring to the front for a launch another process handed over
    /// (`services::instance`): the launcher granted us the foreground, so no synthetic
    /// input is needed (winit's `Focus` presses Alt on Windows).
    Raise,
    Minimized(bool),
    Maximized(bool),
    Title(String),
    /// Move the window's outer top-left corner to this screen point.
    OuterPosition(crate::geom::Point),
    /// Resize the window's inner area.
    InnerSize(crate::geom::Vec2),
}

/// What app logic may ask of the frontend.
pub trait Host {
    /// A wake-up callback background threads can keep.
    fn wake(&self) -> Wake;
    /// Draw another frame soon.
    fn redraw(&self);
    /// Draw another frame after `after`.
    fn redraw_after(&self, after: Duration);
    /// Put plain text on the clipboard.
    fn copy_text(&self, text: String);
    /// Apply a theme preference.
    fn set_theme(&self, pref: ThemePref);
    /// Send a command to a window.
    fn window(&self, id: WindowId, cmd: WindowCmd);
    /// The active configuration changed from `old` to `new`: re-apply what the frontend
    /// derives from it (styles, fonts, scale, input timings).
    fn config_changed(&self, old: &soyuz_core::config::Config, new: &soyuz_core::config::Config);
}

/// Hook a frontend installs to see text that is about to be shown (file names), e.g. to
/// load fallback fonts for the scripts it needs. Process-wide; the first install wins.
static TEXT_SHOWN: OnceLock<fn(&str)> = OnceLock::new();

/// Install the [`note_text_shown`] hook (later calls are ignored).
pub fn set_text_shown_hook(hook: fn(&str)) {
    let _ = TEXT_SHOWN.set(hook);
}

/// `text` (a file name) is about to be shown: tell the frontend (no-op without a hook).
pub fn note_text_shown(text: &str) {
    if let Some(hook) = TEXT_SHOWN.get() {
        hook(text);
    }
}

/// A host with no window: wake-ups and commands go nowhere, copied text is kept.
#[derive(Default)]
pub struct NullHost {
    pub copied: std::sync::Mutex<Vec<String>>,
    pub windows: std::sync::Mutex<Vec<(WindowId, WindowCmd)>>,
}

impl Host for NullHost {
    fn wake(&self) -> Wake {
        no_wake()
    }
    fn redraw(&self) {}
    fn redraw_after(&self, _after: Duration) {}
    fn copy_text(&self, text: String) {
        self.copied.lock().unwrap_or_else(|e| e.into_inner()).push(text);
    }
    fn set_theme(&self, _pref: ThemePref) {}
    fn window(&self, id: WindowId, cmd: WindowCmd) {
        self.windows.lock().unwrap_or_else(|e| e.into_inner()).push((id, cmd));
    }
    fn config_changed(&self, _old: &soyuz_core::config::Config, _new: &soyuz_core::config::Config) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_host_records_side_effects() {
        let h = NullHost::default();
        h.copy_text("a".into());
        h.window(WindowId::Root, WindowCmd::Focus);
        (h.wake())();
        assert_eq!(*h.copied.lock().unwrap(), vec!["a".to_string()]);
        assert_eq!(*h.windows.lock().unwrap(), vec![(WindowId::Root, WindowCmd::Focus)]);
    }
}
