//! The frontend seam: what app logic outside `ui/` may ask of the window system.
//!
//! The GUI implements [`Host`] on `egui::Context`; headless tests use [`NullHost`]; the
//! terminal UI (roadmap Phase T) gets its own. The host only has side effects: facts that
//! change per frame (a close request, text-field focus) travel with the input instead.

use std::any::Any;
use std::sync::Arc;
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

/// Which window a [`WindowCmd`] is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowId {
    /// The window the current frame belongs to.
    #[default]
    Current,
    /// The main (root) window of the process.
    Root,
}

/// A request to the window system.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowCmd {
    Close,
    /// Veto the close request of this frame.
    CancelClose,
    Focus,
    Minimized(bool),
    Maximized(bool),
    Title(String),
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
    /// Transitional escape hatch while modules move out of `ui/` (Phase T0): lets a
    /// GUI-only callee get the toolkit back. Removed when `soyuz-appcore` is extracted.
    fn as_any(&self) -> &dyn Any;
}

/// A [`Wake`] that repaints `ctx` (for GUI code that starts background work).
pub fn egui_wake(ctx: &egui::Context) -> Wake {
    let c = ctx.clone();
    Arc::new(move || c.request_repaint())
}

/// The egui context behind `host` (only for GUI callees during the T0 migration).
pub fn egui_ctx(host: &dyn Host) -> &egui::Context {
    host.as_any().downcast_ref::<egui::Context>().expect("egui frontend")
}

impl From<ThemePref> for egui::ThemePreference {
    fn from(p: ThemePref) -> Self {
        match p {
            ThemePref::System => egui::ThemePreference::System,
            ThemePref::Dark => egui::ThemePreference::Dark,
            ThemePref::Light => egui::ThemePreference::Light,
        }
    }
}

impl From<egui::ThemePreference> for ThemePref {
    fn from(p: egui::ThemePreference) -> Self {
        match p {
            egui::ThemePreference::System => ThemePref::System,
            egui::ThemePreference::Dark => ThemePref::Dark,
            egui::ThemePreference::Light => ThemePref::Light,
        }
    }
}

/// The egui frontend's host is the context itself (transitional: a newtype once the core
/// moves to its own crate, where this impl would break the orphan rule).
impl Host for egui::Context {
    fn wake(&self) -> Wake {
        egui_wake(self)
    }
    fn redraw(&self) {
        egui::Context::request_repaint(self);
    }
    fn redraw_after(&self, after: Duration) {
        egui::Context::request_repaint_after(self, after);
    }
    fn copy_text(&self, text: String) {
        egui::Context::copy_text(self, text);
    }
    fn set_theme(&self, pref: ThemePref) {
        egui::Context::set_theme(self, egui::ThemePreference::from(pref));
    }
    fn window(&self, id: WindowId, cmd: WindowCmd) {
        let cmd = match cmd {
            WindowCmd::Close => egui::ViewportCommand::Close,
            WindowCmd::CancelClose => egui::ViewportCommand::CancelClose,
            WindowCmd::Focus => egui::ViewportCommand::Focus,
            WindowCmd::Minimized(b) => egui::ViewportCommand::Minimized(b),
            WindowCmd::Maximized(b) => egui::ViewportCommand::Maximized(b),
            WindowCmd::Title(t) => egui::ViewportCommand::Title(t),
        };
        match id {
            WindowId::Current => self.send_viewport_cmd(cmd),
            WindowId::Root => self.send_viewport_cmd_to(egui::ViewportId::ROOT, cmd),
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
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
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_pref_round_trips_through_egui() {
        for p in [ThemePref::System, ThemePref::Dark, ThemePref::Light] {
            assert_eq!(ThemePref::from(egui::ThemePreference::from(p)), p);
        }
    }

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
