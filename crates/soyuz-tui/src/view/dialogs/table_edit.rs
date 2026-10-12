//! The view of the dialog for the rows of an array / map of tables in the settings editor (add,
//! duplicate, remove, reorder) (`TuiDialog::TableEdit`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W3 settings tables fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::table_edit::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
