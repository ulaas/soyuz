//! A confirmation before a command (`FrontendCmd::Confirm`) — Phase T3.
//!
//! Stub: [`open`] shows the "not available in the terminal UI yet" toast of Phase T1.
//! Nothing is pushed until the dialog exists (see the shape of a kind file in `super`).

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;
use crate::widgets::Form;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub form: Form,
}

/// Open the dialog (from `crate::frontend`).
pub fn open(app: &mut App, title: String, message: String, confirm: String, on_confirm: Cmd) {
    let _ = (title, message, confirm, on_confirm);
    app.unavailable(tr!("tui-feature-confirm"));
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    super::form_input(&mut s.form, i)
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, _s: &mut State, _e: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The dialog went away by itself (its pane or files are gone, …).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
