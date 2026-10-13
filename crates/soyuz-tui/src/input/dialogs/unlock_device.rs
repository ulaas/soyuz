//! The unlock dialog of a locked device container (`TuiDialog::UnlockDevice`, Phase T7,
//! agent B): a masked passphrase / recovery key field (Alt+V shows it), unlocking on a
//! worker (`models::devices`); the text is wiped on submit and on drop (`zeroize`). The
//! GUI's `ui/dialogs/unlock_volume.rs`.
//!
//! Skeleton (Phase T7 wave 0): `open` says "not available" until agent B fills it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::state::pane::PaneId;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {}

/// Open the dialog for `FrontendCmd::UnlockDevice` (mount into `pane` once unlocked).
pub fn open(app: &mut App, _object: String, _name: String, _kind: String, _pane: PaneId) {
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
