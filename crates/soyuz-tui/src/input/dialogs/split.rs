//! Split and join files (`TuiDialog::Split`, Phase T7, agent A): part size presets or a
//! custom size, the output folder, progress, cancel; draws `models::split::SplitModel`
//! (the GUI's `ui/tools/split.rs` window). Closing it while it runs cancels (`Drop`).
//!
//! Skeleton (Phase T7 wave 0): `open` says "not available" until agent A fills it.

use std::path::PathBuf;

use soyuz_appcore::core::AppCore;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {}

/// Open the dialog to split `path` (`ToolCmd::Split`).
pub fn open_split(app: &mut App, _path: PathBuf) {
    app.core.tui_unavailable();
}

/// Open the dialog to join the parts of `path` (`ToolCmd::Join`).
pub fn open_join(app: &mut App, _path: PathBuf) {
    app.core.tui_unavailable();
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, _s: &mut State, _i: &DialogInput) -> Outcome {
    Outcome::Close(None)
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, _s: &mut State, _m: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The dialog went away by itself.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

/// Every frame while it is open (`input::dialogs::tick`, from `view::prepare`): worker
/// results, rows rebuilt from the core.
pub fn tick(_core: &mut AppCore, _s: &mut State) {}
