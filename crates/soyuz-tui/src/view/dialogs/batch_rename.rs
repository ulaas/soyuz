//! The view of batch rename on the `rename` planner (`FrontendCmd::MetaDialog(MetaDialog::BatchRename)`) — Phase T3.
//!
//! Stub: the title and the form (`DialogVm::from_form`).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::batch_rename::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm::from_form(&s.title, &s.form)
}
