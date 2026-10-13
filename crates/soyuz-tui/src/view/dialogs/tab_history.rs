//! The view of the tab history dialog (`TuiDialog::TabHistory`, agent A).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::tab_history::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm::default()
}
