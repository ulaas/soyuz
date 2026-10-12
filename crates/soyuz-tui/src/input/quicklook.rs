//! Keys of Quick Look (`TuiFocus::QuickLook`) — Phase T4.
//!
//! Stub: Quick Look is never open in the terminal UI yet (`crate::unsupported` closes it
//! with a toast), so [`open`] is `false`.

use crossterm::event::{Event, MouseEvent};
use soyuz_appcore::core::AppCore;

use super::InputState;
use crate::app::App;

/// What Quick Look keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// First line shown.
    pub scroll: usize,
}

/// Whether the overlay is open (T4: `core.ui.quicklook.open`).
pub fn open(_core: &AppCore, _st: &InputState) -> bool {
    false
}

/// Keys while it is open.
pub fn route(app: &mut App, events: &[Event]) {
    super::list::route(app, events);
}

/// A mouse event while it is open (a press outside closes it).
pub fn mouse(_app: &mut App, _m: &MouseEvent) {}
