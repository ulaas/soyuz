//! The view of the unlock dialog (`TuiDialog::UnlockDevice`, agent B).

use soyuz_appcore::core::AppCore;

use super::DialogVm;
use crate::input::dialogs::unlock_device::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm::default()
}
