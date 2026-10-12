//! Keys and mouse of the settings editor and its key bindings page, a full-screen overlay (Phase
//! T6).
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-settings-editor (tables and secrets: W3) fills it.
//! The signatures are the contract the central files (`input/mod.rs`, `input/mouse.rs`,
//! `frontend.rs`, `tools.rs`) call.

use crossterm::event::{Event, MouseEvent};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::settings::Section;

use super::InputState;
use crate::app::App;
use crate::layout::HitTarget;

/// What this part keeps between frames (`InputState::settings`).
#[derive(Debug, Default)]
pub struct State {
    /// The editor is open (it owns the keyboard: `TuiFocus::Settings`).
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

/// Open the settings editor, on `section` when given (`FrontendCmd::OpenSettings`).
/// Stub: "not available" (the core's `ui.settings_open` stays off, so the palette keeps
/// working).
pub fn open(app: &mut App, _section: Option<Section>) {
    app.core.ui.settings_open = false;
    app.core.tui_unavailable();
}

/// Close the settings editor (`FrontendCmd::CloseSettings`), writing a pending save.
pub fn close(_app: &mut App) {}
