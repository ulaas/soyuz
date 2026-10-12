//! The view of the dialog for editing the new names of an AI plan card (`TuiDialog::PlanNames`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-ai-panel fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::plan_names::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
