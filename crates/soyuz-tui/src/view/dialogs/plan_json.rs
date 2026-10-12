//! The view of the dialog for the multi-line JSON arguments of an AI plan card (with "Edit in
//! $EDITOR") (`TuiDialog::PlanJson`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W3 AI composer + JSON edit fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::DialogVm;
use crate::input::dialogs::plan_json::State;

/// What the dialog shows.
pub fn vm(_core: &AppCore, _s: &State) -> DialogVm {
    DialogVm { title: tr!("tui-not-available").into(), ..DialogVm::default() }
}
