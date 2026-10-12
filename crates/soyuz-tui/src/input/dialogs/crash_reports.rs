//! Crash reports from the last run (`FrontendCmd::CrashReports`) — Phase T3.
//!
//! Stub: [`open`] shows the Phase T1 toast with the newest report and keeps the reports.
//! Nothing is pushed until the dialog exists (see the shape of a kind file in `super`).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::services::crash::{self, CrashReport, RecoveryAction};
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;
use crate::widgets::Form;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub form: Form,
}

/// Open the dialog (from `crate::frontend`).
pub fn open(app: &mut App, reports: Vec<CrashReport>) {
    if let Some(newest) = reports.first() {
        let path = soyuz_fs::strip_verbatim(&newest.path).display().to_string();
        let path = crate::dialog::safe(&path);
        app.core.ui.toasts.push(Toast::warning(tr!("tui-crashed", path = path)).sticky());
    }
    // Offered once, like the GUI's "Keep" (the reports stay on disk).
    crash::request(RecoveryAction::KeepReports(reports));
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    super::form_input(&mut s.form, i)
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, _s: &mut State, _e: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The dialog went away by itself (its pane or files are gone, …).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
