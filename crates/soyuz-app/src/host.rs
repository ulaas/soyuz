//! The egui frontend's [`Host`]: [`EguiHost`] wraps the `egui::Context` (a newtype because
//! `Host` lives in `soyuz-appcore`, so the orphan rule forbids `impl Host for
//! egui::Context`). Re-exports the core seam so `crate::host::…` paths keep working.

use std::sync::Arc;
use std::time::Duration;

pub use soyuz_appcore::host::*;

/// The egui context as the core's [`Host`]. Cheap to make: `egui::Context` is an `Arc`.
#[derive(Clone)]
pub struct EguiHost(pub egui::Context);

impl EguiHost {
    /// Wrap `ctx` (clones the handle).
    pub fn new(ctx: &egui::Context) -> Self {
        EguiHost(ctx.clone())
    }

    /// The wrapped context.
    pub fn ctx(&self) -> &egui::Context {
        &self.0
    }
}

/// A [`Wake`] that repaints `ctx` (for GUI code that starts background work).
pub fn egui_wake(ctx: &egui::Context) -> Wake {
    let c = ctx.clone();
    Arc::new(move || c.request_repaint())
}

/// [`ThemePref`] → egui's theme preference.
pub fn to_egui_theme(p: ThemePref) -> egui::ThemePreference {
    match p {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Dark => egui::ThemePreference::Dark,
        ThemePref::Light => egui::ThemePreference::Light,
    }
}

/// egui's theme preference → [`ThemePref`].
pub fn from_egui_theme(p: egui::ThemePreference) -> ThemePref {
    match p {
        egui::ThemePreference::System => ThemePref::System,
        egui::ThemePreference::Dark => ThemePref::Dark,
        egui::ThemePreference::Light => ThemePref::Light,
    }
}

impl Host for EguiHost {
    fn wake(&self) -> Wake {
        egui_wake(&self.0)
    }
    fn redraw(&self) {
        self.0.request_repaint();
    }
    fn redraw_after(&self, after: Duration) {
        self.0.request_repaint_after(after);
    }
    fn copy_text(&self, text: String) {
        self.0.copy_text(text);
    }
    fn set_theme(&self, pref: ThemePref) {
        self.0.set_theme(to_egui_theme(pref));
    }
    fn window(&self, id: WindowId, cmd: WindowCmd) {
        let vp = match id {
            WindowId::Current => self.0.viewport_id(),
            WindowId::Root => egui::ViewportId::ROOT,
            WindowId::Window(w) => crate::ui::workspaces::windows::viewport_id(w),
        };
        let cmd = match cmd {
            WindowCmd::Raise => return crate::ui::native::raise::request(&self.0, vp),
            WindowCmd::Close => egui::ViewportCommand::Close,
            WindowCmd::CancelClose => egui::ViewportCommand::CancelClose,
            WindowCmd::Focus => egui::ViewportCommand::Focus,
            WindowCmd::Minimized(b) => egui::ViewportCommand::Minimized(b),
            WindowCmd::Maximized(b) => egui::ViewportCommand::Maximized(b),
            WindowCmd::Title(t) => egui::ViewportCommand::Title(t),
            WindowCmd::OuterPosition(p) => egui::ViewportCommand::OuterPosition(crate::ui::geom::pos(p)),
            WindowCmd::InnerSize(s) => egui::ViewportCommand::InnerSize(crate::ui::geom::vec(s)),
        };
        self.0.send_viewport_cmd_to(vp, cmd);
    }
    fn config_changed(&self, old: &soyuz_core::config::Config, c: &soyuz_core::config::Config) {
        let ctx = &self.0;
        if old.appearance != c.appearance || old.fonts != c.fonts || old.theme != c.theme {
            for w in crate::theme::install(ctx, &c.appearance, &c.fonts, &c.theme.colors) {
                log::warn!("{w}");
            }
            let scale = if c.appearance.ui_scale > 0.25 { c.appearance.ui_scale.clamp(0.5, 3.0) } else { 1.0 };
            if (ctx.zoom_factor() - scale).abs() > f32::EPSILON {
                ctx.set_zoom_factor(scale);
            }
        }
        crate::app::apply_live_settings(ctx, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_pref_round_trips_through_egui() {
        for p in [ThemePref::System, ThemePref::Dark, ThemePref::Light] {
            assert_eq!(from_egui_theme(to_egui_theme(p)), p);
        }
    }
}
