//! The view of the "Save workspace as…" dialog (`TuiDialog::SaveWorkspace`, agent A).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::save_workspace::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm::default()
}
