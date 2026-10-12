//! Keys and mouse of the preview pane (`TuiFocus::Preview`) and the preview scroll
//! actions — Phase T4.
//!
//! Stubs: the pane never has focus yet ([`has_focus`]), [`scroll`] (`PreviewScroll*` /
//! `PreviewPage*`) says the preview is not available, and clicks / wheel on its targets
//! (`PreviewScroll`, `ArchiveRow`) do nothing.

use crossterm::event::{Event, MouseEvent};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::frontend::ScrollStep;
use soyuz_appcore::tr;

use super::InputState;
use crate::app::App;
use crate::layout::HitTarget;

/// What the preview pane keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The pane has the keyboard.
    pub focused: bool,
    /// First line shown.
    pub scroll: usize,
}

/// Whether the pane has the keyboard (it must be shown).
pub fn has_focus(core: &AppCore, st: &InputState) -> bool {
    st.preview.focused && crate::view::preview::shown(core)
}

/// Scroll the preview (the `PreviewScroll*` / `PreviewPage*` actions).
pub fn scroll(app: &mut App, step: ScrollStep) {
    let _ = step;
    app.unavailable(tr!("tui-feature-preview"));
}

/// Keys while the pane has focus.
pub fn route(app: &mut App, events: &[Event]) {
    app.input.preview.focused = false;
    super::list::route(app, events);
}

/// A mouse press on one of the pane's targets.
pub fn click(_app: &mut App, _target: HitTarget, _m: &MouseEvent) {}

/// The wheel over one of the pane's targets (`rows`: negative scrolls up).
pub fn wheel(_app: &mut App, _target: HitTarget, _rows: i32) {}
