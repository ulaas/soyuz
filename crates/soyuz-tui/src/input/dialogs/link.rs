//! New links: the core's "new link" prompt (`DialogModel::NewLink`) and the link dialog
//! the core requests for the selection (`FrontendCmd::MetaDialog(MetaDialog::Link)`,
//! the GUI's `dialogs::links`) — Phase T3.
//!
//! Stubs: the Phase T1 text fallback for the prompt; [`open`] shows the "not available in
//! the terminal UI yet" toast. Nothing is pushed until the dialog exists (see the shape
//! of a kind file in `super`).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::models::properties::LinkKind;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, ModelUi, Outcome};
use crate::app::App;
use crate::widgets::Form;

// ---- the core prompt (`DialogModel::NewLink`) -----------------------------------------

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(_m: &DialogModel) -> ModelUi {
    ModelUi::default()
}

/// A key press or paste on the prompt.
pub fn on_model_input(m: &mut DialogModel, _ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    super::fallback_input(m, i)
}

/// A mouse event on the prompt.
pub fn on_model_mouse(_m: &mut DialogModel, _ui: &mut ModelUi, _e: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The prompt went away by itself.
pub fn model_stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}

// ---- the terminal UI dialog (`MetaDialog::Link`) ---------------------------------------

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub form: Form,
}

/// Open the link dialog of kind `kind` for the focused pane's selection.
pub fn open(app: &mut App, kind: LinkKind) {
    let _ = kind;
    app.unavailable(tr!("tui-feature-links"));
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    super::form_input(&mut s.form, i)
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, _s: &mut State, _e: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The dialog went away by itself.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
