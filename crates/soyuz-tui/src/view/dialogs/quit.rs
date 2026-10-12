//! The view of quit with running jobs (`DialogModel::Quit`) — Phase T3.
//!
//! Stub: the Phase T1 text fallback (`crate::dialog::fallback`).

use soyuz_appcore::models::dialogs::DialogModel;

use super::DialogVm;
use crate::input::dialogs::ModelUi;

/// What the dialog shows.
pub fn vm(m: &DialogModel, _ui: &ModelUi) -> DialogVm {
    crate::dialog::fallback(m)
}
