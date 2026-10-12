//! The view of select by pattern (`FrontendCmd::SelectByPattern`) — Phase T3.
//!
//! Stub: the title and the form (`DialogVm::from_form`).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::pattern::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm::from_form(&s.title, &s.form)
}
