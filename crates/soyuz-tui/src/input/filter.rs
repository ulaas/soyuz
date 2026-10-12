//! Keys while the filter field is open (step T2.8): the core first
//! (`KeyContext::Omnibar`, `text_focused`), then a `LineEdit` on the pane's filter with
//! the unconsumed events.

use crossterm::event::Event;

use super::{TuiFocus, core_input, key_context};
use crate::app::App;

/// What the filter field keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The filter field owns the keyboard (armed by the core's `ui.filter_focus`).
    pub editing: bool,
}

/// Before routing: the core asked for the filter field (`Action::Filter`). T2.8 arms
/// `State::editing` here; until then the request is dropped.
pub fn arm(app: &mut App) {
    if app.core.ui.filter_focus {
        app.core.ui.filter_focus = false;
    }
}

pub fn route(app: &mut App, events: &[Event]) {
    let (kctx, text) = key_context(TuiFocus::Filter);
    core_input(app, events, kctx, text);
}
