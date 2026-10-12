//! Keys and mouse of the folder compare and sync screen, a full-screen overlay (Phase T5).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-compare-screen (profiles: W3) fills it. The
//! signatures are the contract the central files (`input/mod.rs`, `input/mouse.rs`, `frontend.rs`,
//! `tools.rs`) call.

use std::path::PathBuf;

use crossterm::event::{Event, MouseEvent};
use soyuz_appcore::core::AppCore;

use super::InputState;
use crate::app::App;
use crate::layout::HitTarget;

/// What this part keeps between frames (`InputState::compare`).
#[derive(Debug, Default)]
pub struct State {
    /// The screen is open (it owns the keyboard: `TuiFocus::Tool`).
    pub open: bool,
}

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

/// Open the compare screen on `left` and `right` (`ToolCmd::Compare`, from
/// `crate::tools::drain`). Stub: "not available".
pub fn open(app: &mut App, _left: PathBuf, _right: Option<PathBuf>) {
    app.core.tui_unavailable();
}

/// Open the compare screen with saved sync profile `name` (`ToolCmd::CompareProfile`).
/// Stub: "not available".
pub fn open_profile(app: &mut App, _name: String) {
    app.core.tui_unavailable();
}

/// The sync profiles (edit, schedule, run; `ToolCmd::SyncProfiles`). Stub: "not
/// available".
pub fn open_profiles(app: &mut App) {
    app.core.tui_unavailable();
}
