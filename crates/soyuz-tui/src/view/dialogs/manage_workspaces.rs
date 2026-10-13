//! The view of the "Manage workspaces" dialog (`TuiDialog::ManageWorkspaces`, agent A).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::manage_workspaces::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm::default()
}
