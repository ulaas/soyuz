//! Keys and mouse of the search bar and its status row (Phase T5).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W3-search-bar fills it. The signatures are the contract
//! the central files (`input/mod.rs`, `input/mouse.rs`, `frontend.rs`, `tools.rs`) call.

use crossterm::event::{Event, MouseEvent};
use soyuz_appcore::core::AppCore;

use super::InputState;
use crate::app::App;
use crate::layout::HitTarget;

/// What this part keeps between frames (`InputState::search`).
#[derive(Debug, Default)]
pub struct State {}

/// It owns the keyboard this frame (`input::focus`).
pub fn has_focus(_core: &AppCore, _st: &InputState) -> bool {
    false
}

/// This frame's events while it has the focus.
pub fn route(_app: &mut App, _events: &[Event]) {}

/// A mouse event over one of its click targets (`input::mouse` dispatches by target);
/// returns whether it was taken.
pub fn on_mouse(_app: &mut App, _target: HitTarget, _ev: &MouseEvent) -> bool {
    false
}

/// While `true`, every mouse event goes to [`on_mouse`] wherever it is (a drag that
/// leaves the targets, an overlay's capture); the target is `HitTarget::Overlay` when
/// nothing is under the pointer.
pub fn captures_mouse(_core: &AppCore, _st: &InputState) -> bool {
    false
}

/// Consume the core's request to focus the search bar (`ui.search.focus`, set by
/// `Search` / `SearchContent` / Refine), like `filter::arm`. Called every frame before
/// the routing. Stub: closes the bar again and says "not available".
pub fn arm(app: &mut App) {
    if app.core.ui.search.focus {
        app.core.ui.search.focus = false;
        soyuz_appcore::models::search::close(&mut app.core);
        app.core.tui_unavailable();
    }
}
