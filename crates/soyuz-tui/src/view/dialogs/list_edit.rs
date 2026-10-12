//! The view of the dialog for editing a list / map value of the settings editor
//! (`TuiDialog::ListEdit`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-settings-editor fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::list_edit::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
