//! Core states the terminal UI has no view for yet: reset them right after the frame's
//! input (so the keyboard keeps working) and say so with a toast.
//!
//! One function per feature. The step that builds a feature's view deletes its function
//! and its call in [`reset_all`] (keep the comment line above the call: the orchestrator
//! tidies up; this keeps parallel deletions from touching the same lines), and the
//! feature's `tui-feature-*` key in both `tui.ftl` files when nothing else uses it.

use soyuz_appcore::tr;

use crate::app::App;

/// Run every reset.
pub fn reset_all(app: &mut App) {
    // Later phases: settings.
    settings(app);

    // Later phases: Quick Look.
    quicklook(app);

    // Later phases: terminal panel focus.
    terminal_focus(app);
}

fn settings(app: &mut App) {
    if app.core.ui.settings_open {
        app.core.ui.settings_open = false;
        app.unavailable(tr!("tui-feature-settings"));
    }
}

fn quicklook(app: &mut App) {
    if app.core.ui.quicklook.open {
        soyuz_appcore::models::quicklook::close(&mut app.core);
        app.unavailable(tr!("tui-feature-quicklook"));
    }
}

fn terminal_focus(app: &mut App) {
    if soyuz_appcore::models::terminal::owns_keyboard(&app.core) {
        app.core.ui.terminal.focused = false;
        app.unavailable(tr!("tui-feature-terminal"));
    }
}
