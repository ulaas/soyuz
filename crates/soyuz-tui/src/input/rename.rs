//! Keys of the inline rename field (`TuiFocus::Rename`, the core's `fileops.rename`) —
//! Phase T3.
//!
//! Stub: the field is never active yet ([`active`]), so the keys stay with the file list
//! as in Phase T2.

use crossterm::event::{Event, MouseEvent};
use soyuz_appcore::core::AppCore;

use super::InputState;
use crate::app::App;
use crate::widgets::LineEdit;

/// What the rename field keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// Cursor and scroll of the field.
    pub edit: LineEdit,
}

/// Whether an inline rename owns the keyboard (T3: `core.fileops.rename.is_some()`).
pub fn active(_core: &AppCore, _st: &InputState) -> bool {
    false
}

/// Keys while the field is active.
pub fn route(app: &mut App, events: &[Event]) {
    super::list::route(app, events);
}

/// A mouse press on the field (`RenameField`).
pub fn click(_app: &mut App, _m: &MouseEvent) {}
