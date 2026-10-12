//! The view of the dialog for recording a key chord on the key bindings page
//! (`TuiDialog::Keybind`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-settings-editor fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::keybind::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
