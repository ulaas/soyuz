//! The view of the dialog for the form of one element of an array / map of tables in the settings
//! editor (`TuiDialog::TableRow`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W3 settings tables fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::table_row::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
