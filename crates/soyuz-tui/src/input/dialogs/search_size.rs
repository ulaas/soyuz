//! The dialog for the search bar's size filter (min / max) (`TuiDialog::SearchSize`).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W3-search-bar fills it. Nothing opens it yet; if
//! something did, Esc closes it.

use crossterm::event::KeyCode;
use soyuz_appcore::core::AppCore;

use super::{DialogInput, DialogMouse, Outcome};

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, _s: &mut State, i: &DialogInput) -> Outcome {
    if i.key.is_some_and(|k| k.code == KeyCode::Esc) { Outcome::Close(None) } else { Outcome::Keep }
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, _s: &mut State, _m: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The dialog went away by itself.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
