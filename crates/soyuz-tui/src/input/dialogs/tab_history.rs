//! The tab history dialog, "Where was I?" (`TuiDialog::TabHistory`, Phase T7, agent A):
//! every place the tab visited as a tree (`models::workspaces::history_rows`, rebuilt by
//! [`tick`]); Enter goes there (the GUI's `ui/workspaces/history.rs`).
//!
//! Skeleton (Phase T7 wave 0): `open` says "not available" until agent A fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::state::workspace::TabId;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {}

/// Open the dialog for `FrontendCmd::TabHistory` (tab `tab`, starting at pane `pane`).
pub fn open(app: &mut App, _tab: TabId, _pane: PaneId) {
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
