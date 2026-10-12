//! The views of the new-link prompt (`DialogModel::NewLink`) and of the link dialog
//! (`MetaDialog::Link`) — Phase T3.
//!
//! Stubs: the Phase T1 text fallback; the title and the form.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::dialogs::DialogModel;

use super::DialogVm;
use crate::input::dialogs::ModelUi;
use crate::input::dialogs::link::State;

/// What the core prompt shows.
pub fn model_vm(m: &DialogModel, _ui: &ModelUi) -> DialogVm {
    crate::dialog::fallback(m)
}

/// What the terminal UI dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    DialogVm::from_form(&s.title, &s.form)
}
