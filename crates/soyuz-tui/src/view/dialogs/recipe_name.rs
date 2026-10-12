//! The view of the dialog for the name of a saved AI recipe (`TuiDialog::RecipeName`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-ai-panel fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::recipe_name::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
