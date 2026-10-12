//! Mouse input (step T2.9): clicks are looked up in the last frame's
//! `crate::layout::Hits` (`app.hits`), the wheel scrolls `pane.scroll_y` (rows) without
//! moving the cursor. Runs before the keyboard routing. Stub: mouse events are ignored.

use crossterm::event::Event;

use crate::app::App;

/// What the mouse keeps between frames (drag, double-click timing, …).
#[derive(Debug, Default)]
pub struct State {}

pub fn route(_app: &mut App, _events: &[Event]) {}
