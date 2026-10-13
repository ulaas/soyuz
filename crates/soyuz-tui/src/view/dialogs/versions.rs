//! The view of the previous versions dialog (`TuiDialog::Versions`, agent B).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::versions::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm::default()
}
