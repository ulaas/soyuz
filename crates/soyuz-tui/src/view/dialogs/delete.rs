//! The view of the delete / purge / empty-trash confirmation (`DialogModel::DeleteConfirm`) — Phase T3.
//!
//! Stub: the Phase T1 text fallback (`crate::dialog::fallback`).

use soyuz_appcore::models::dialogs::DialogModel;

use super::DialogVm;
use crate::input::dialogs::ModelUi;

/// What the dialog shows.
pub fn vm(m: &DialogModel, _ui: &ModelUi) -> DialogVm {
    crate::dialog::fallback(m)
}
