//! The view of the split / join dialog (`TuiDialog::Split`, agent A).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::split::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm::default()
}
