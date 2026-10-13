//! The "Save workspace as…" dialog (`TuiDialog::SaveWorkspace`, Phase T7, agent A): a name
//! field prefilled with the core's suggestion; Save queues `WsCmd::Save` (the GUI's
//! `ui/workspaces/dialogs.rs` `SaveWorkspaceDialog`).
//!
//! Skeleton (Phase T7 wave 0): `open` says "not available" until agent A fills it.

use soyuz_appcore::core::AppCore;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {}

/// Open the dialog for `FrontendCmd::SaveWorkspace` with the suggested `name`.
pub fn open(app: &mut App, _name: String) {
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
