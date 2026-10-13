//! The "Manage workspaces" dialog (`TuiDialog::ManageWorkspaces`, Phase T7, agent A): the
//! saved workspaces (`models::workspaces::store_snapshot`) with open, edit (name, hotkey),
//! move up and delete (the GUI's `ManageWorkspacesDialog`).
//!
//! Skeleton (Phase T7 wave 0): `open` says "not available" until agent A fills it.

use soyuz_appcore::core::AppCore;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {}

/// Open the dialog for `FrontendCmd::ManageWorkspaces`.
pub fn open(app: &mut App) {
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
