//! Keys and mouse of the editor panel (`TuiFocus::Editor`) — Phase T4.
//!
//! Stubs: the editor never has focus yet ([`has_focus`]) and clicks / wheel on its
//! targets (`EditorTab`, `EditorClose`, `EditorCell`) do nothing. The core's own editor
//! keys (`editors::handles`) already run through the key map.

use crossterm::event::{Event, MouseEvent};
use soyuz_appcore::core::AppCore;

use super::InputState;
use crate::app::App;
use crate::layout::HitTarget;

/// What the editor keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The editor has the keyboard.
    pub focused: bool,
}

/// Whether the editor has the keyboard (it must be shown).
pub fn has_focus(core: &AppCore, st: &InputState) -> bool {
    st.editor.focused && crate::view::editor::placement(core).is_some()
}

/// Keys while the editor has focus.
pub fn route(app: &mut App, events: &[Event]) {
    app.input.editor.focused = false;
    super::list::route(app, events);
}

/// A mouse press on one of the editor's targets.
pub fn click(_app: &mut App, _target: HitTarget, _m: &MouseEvent) {}

/// The wheel over one of the editor's targets (`rows`: negative scrolls up).
pub fn wheel(_app: &mut App, _target: HitTarget, _rows: i32) {}
