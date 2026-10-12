//! The view of job prompts (`DialogModel::Prompt`): conflicts with apply-to-all, errors, not enough space, trash unsupported, read-only items, archive passwords — Phase T3.
//!
//! Stub: the Phase T1 text fallback (`crate::dialog::fallback`).

use soyuz_appcore::models::dialogs::DialogModel;

use super::DialogVm;
use crate::input::dialogs::ModelUi;

/// What the dialog shows.
pub fn vm(m: &DialogModel, _ui: &ModelUi) -> DialogVm {
    crate::dialog::fallback(m)
}
