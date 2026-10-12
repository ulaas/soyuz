//! The view of the dialog for the search bar's size filter (min / max) (`TuiDialog::SearchSize`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W3-search-bar fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::search_size::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
