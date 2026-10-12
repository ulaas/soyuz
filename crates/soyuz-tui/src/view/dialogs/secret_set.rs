//! The view of the dialog for "Set secret…": a credential written to the OS credential store (only
//! the `credential:<name>` reference goes into the config) (`TuiDialog::SecretSet`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W3 settings tables fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::secret_set::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
