//! Job prompts (`DialogModel::Prompt`): conflicts with apply-to-all, errors, not enough space, trash unsupported, read-only items, archive passwords — Phase T3.
//!
//! Stub: the Phase T1 text fallback of `crate::dialog` (see the shape of a kind file in
//! `super`).

use soyuz_appcore::models::dialogs::DialogModel;

use super::{DialogInput, DialogMouse, ModelUi, Outcome};

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(_m: &DialogModel) -> ModelUi {
    ModelUi::default()
}

/// A key press or paste.
pub fn on_input(m: &mut DialogModel, _ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    super::fallback_input(m, i)
}

/// A mouse event.
pub fn on_mouse(_m: &mut DialogModel, _ui: &mut ModelUi, _e: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The prompt went away by itself.
pub fn stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}
