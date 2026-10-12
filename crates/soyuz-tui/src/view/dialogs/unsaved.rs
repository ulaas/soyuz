//! The view of "Save changes?" (`FrontendCmd::EditorUnsaved`) — Phase T4.
//!
//! Stub: the title and the form (`DialogVm::from_form`).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::unsaved::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm::from_form(&s.title, &s.form)
}
