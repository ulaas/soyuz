//! Focus mode: one action (`Action::ToggleFocusMode`) hides the chrome around the
//! listing — the parts named by `[layout] focus_mode_hides` — and brings back exactly
//! what was shown before when it is toggled again. The breadcrumb / toolbar and the
//! listing always stay. Parts opened while focus mode is on stay open when it ends.

use crate::core::AppCore;
use crate::models::toasts::Toast;

/// Every part focus mode can hide, as `[layout] focus_mode_hides` names them.
pub const PARTS: &[&str] = &["sidebar", "status_bar", "preview", "bottom_panel", "activity", "ai_panel", "tabs"];

/// What was shown when focus mode started (restored when it ends).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    sidebar: bool,
    status: bool,
    preview: bool,
    bottom: bool,
    activity: bool,
    ai: bool,
}

/// The label of part `id` (Settings).
pub fn label(id: &str) -> &'static str {
    // i18n-dynamic: settings-views-focus-part-
    crate::i18n::t_dyn(&format!("settings-views-focus-part-{}", id.replace('_', "-")))
}

/// Focus mode is on.
pub fn active(app: &AppCore) -> bool {
    app.ui.focus_mode.is_some()
}

/// Focus mode is on and hides `part` (the frontends ask for parts they draw from their
/// own state, such as the tab strip).
pub fn hides(app: &AppCore, part: &str) -> bool {
    active(app) && app.config.layout.focus_mode_hides.iter().any(|p| p == part)
}

/// The sidebar as the session should remember it (what focus mode will restore).
pub fn sidebar_visible(app: &AppCore) -> bool {
    match &app.ui.focus_mode {
        Some(s) if hides(app, "sidebar") => s.sidebar,
        _ => app.ui.sidebar_visible,
    }
}

/// Turn focus mode on (hiding the configured parts) or off (restoring them).
pub fn toggle(app: &mut AppCore) {
    let on = |p: &str| app.config.layout.focus_mode_hides.iter().any(|x| x == p);
    let (sidebar, status, preview, bottom, activity, ai) =
        (on("sidebar"), on("status_bar"), on("preview"), on("bottom_panel"), on("activity"), on("ai_panel"));
    if let Some(s) = app.ui.focus_mode.take() {
        // Restore what was shown; anything opened meanwhile stays open.
        let ui = &mut app.ui;
        ui.sidebar_visible |= sidebar && s.sidebar;
        ui.status_visible |= status && s.status;
        ui.preview.open |= preview && s.preview;
        ui.bottom.open |= bottom && s.bottom;
        ui.bottom.activity_open |= activity && s.activity;
        if ai && s.ai && !app.ai.panel.open {
            // It was hidden in this window: it comes back here.
            super::owners::claim(app, super::owners::Panel::Assistant);
            app.ai.panel.open = true;
        }
        return;
    }
    // The assistant is shared: only hidden when it shows in this window.
    let ai_here = app.ai.panel.open && super::owners::here(app, super::owners::Panel::Assistant);
    let ui = &mut app.ui;
    let saved = Saved {
        sidebar: ui.sidebar_visible,
        status: ui.status_visible,
        preview: ui.preview.open,
        bottom: ui.bottom.open,
        activity: ui.bottom.activity_open,
        ai: ai_here,
    };
    if sidebar {
        ui.sidebar_visible = false;
        if ui.focus == crate::core::Focus::Sidebar {
            ui.focus = crate::core::Focus::List;
        }
    }
    if status {
        ui.status_visible = false;
    }
    if preview {
        ui.preview.open = false;
    }
    if bottom {
        ui.bottom.open = false;
    }
    if activity {
        ui.bottom.activity_open = false;
    }
    if ai && ai_here {
        app.ai.panel.open = false;
    }
    app.ui.focus_mode = Some(saved);
    let key = crate::keys::chord_label(&app.keymap, &soyuz_core::config::Action::ToggleFocusMode);
    let body = match key {
        Some(k) => tr!("actions-focus-mode-on-key", key = k),
        None => tr!("actions-focus-mode-on-body").to_string(),
    };
    app.ui.toasts.push(Toast::info(tr!("actions-focus-mode-on")).body(body));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_has_a_label_and_the_defaults_are_known() {
        for p in PARTS {
            assert!(!label(p).starts_with("settings-"), "{p}");
        }
        let d = soyuz_core::config::Config::default();
        assert!(d.layout.focus_mode_hides.iter().all(|p| PARTS.contains(&p.as_str())));
    }
}
