//! The view of the dialog for a two-way sync conflict of the compare screen
//! (`TuiDialog::CompareConflict`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-compare-screen fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::compare_conflict::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
