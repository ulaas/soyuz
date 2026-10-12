//! Keys and mouse of the jobs panel (`TuiFocus::Jobs`) — Phase T3.
//!
//! Stubs: the panel never has focus yet ([`has_focus`]), [`focus`] (the
//! `FocusActivityPanel` action) says the panel is not available, and clicks / wheel on
//! its targets (`BottomTab`, `JobRow`, `JobButton`) do nothing.

use crossterm::event::{Event, MouseEvent};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::InputState;
use crate::app::App;
use crate::layout::HitTarget;
use crate::widgets::SelectList;

/// What the jobs panel keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The panel has the keyboard.
    pub focused: bool,
    /// Cursor and scroll of the job list.
    pub list: SelectList,
}

/// Whether the panel has the keyboard (it must be shown).
pub fn has_focus(core: &AppCore, st: &InputState) -> bool {
    st.jobs.focused && crate::view::jobs::shown(core)
}

/// `FocusActivityPanel`: open the panel and give it the keyboard.
pub fn focus(app: &mut App) {
    app.unavailable(tr!("tui-feature-jobs"));
}

/// Keys while the panel has focus.
pub fn route(app: &mut App, events: &[Event]) {
    app.input.jobs.focused = false;
    super::list::route(app, events);
}

/// A mouse press on one of the panel's targets.
pub fn click(_app: &mut App, _target: HitTarget, _m: &MouseEvent) {}

/// The wheel over one of the panel's targets (`rows`: negative scrolls up).
pub fn wheel(_app: &mut App, _target: HitTarget, _rows: i32) {}
