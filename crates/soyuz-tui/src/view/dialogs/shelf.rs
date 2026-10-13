//! The view of the drop shelf dialog (`TuiDialog::Shelf`, agent A).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::shelf::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm::default()
}
