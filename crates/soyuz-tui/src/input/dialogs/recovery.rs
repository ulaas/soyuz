//! Interrupted jobs from the journal: resume, roll back or discard (`FrontendCmd::RecoveredJobs`) — Phase T3.
//!
//! Stub: [`open`] shows the Phase T1 toast (the jobs stay in the journal for the GUI).
//! Nothing is pushed until the dialog exists (see the shape of a kind file in `super`).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_jobs::journal::Recovered;

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
pub fn open(app: &mut App, recovered: Vec<Recovered>) {
    // Left in the journal for the GUI to resume or roll back.
    let count: usize = recovered.iter().map(|r| r.jobs.len()).sum();
    app.core.ui.toasts.push(Toast::warning(tr!("tui-recovered-jobs", count = count)).sticky());
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
