//! Keys while the sidebar has focus (step T2.6). Stub: the core's dispatcher in the
//! sidebar context (`crate::unsupported::sidebar_focus` moves the focus back to the list
//! after the input that moved it).

use crossterm::event::Event;

use super::{TuiFocus, core_input, key_context};
use crate::app::App;

/// What the sidebar keeps between frames (its cursor row, …).
#[derive(Debug, Default)]
pub struct State {}

pub fn route(app: &mut App, events: &[Event]) {
    let (kctx, text) = key_context(TuiFocus::Sidebar);
    core_input(app, events, kctx, text);
}
