//! The view of the dialog for the smart folder editor and its delete confirmation
//! (`FrontendCmd::Smart`) (`TuiDialog::Smart`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-locations fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::smart::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
