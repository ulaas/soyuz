//! The drop shelf (`TuiDialog::Shelf`, Phase T7, agent A; owner decision 6): the paths on
//! the shelf (`models::tools::with_shelf`) with copy / move here (as jobs), remove and
//! clear. [`sync`] opens it when `Action::ToggleShelf` turned the shelf on (and closes it
//! when it went off).
//!
//! Skeleton (Phase T7 wave 0): `open` says "not available" until agent A fills it.

use soyuz_appcore::core::AppCore;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {}

/// Open the shelf dialog.
pub fn open(app: &mut App) {
    app.core.tui_unavailable();
}

/// Every `crate::tools::drain` (before the tool queue): follow the core's shelf flag
/// (`models::tools::shelf_open`, flipped by `Action::ToggleShelf`). Skeleton: the shelf
/// goes off again with a "not available" toast.
pub fn sync(app: &mut App) {
    if soyuz_appcore::models::tools::shelf_open() {
        soyuz_appcore::models::tools::with_shelf(|s| s.open = false);
        app.core.tui_unavailable();
    }
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
