//! Previous versions of a file or folder (`TuiDialog::Versions`, Phase T7, agent B): the
//! list from `models::versions::VersionsModel` with open, compare, copy to and restore.
//! Without administrator rights it shows a message only (owner decision 2: no elevated
//! relaunch from the terminal UI).
//!
//! Skeleton (Phase T7 wave 0): `open` says "not available" until agent B fills it.

use std::path::PathBuf;
use std::time::SystemTime;

use soyuz_appcore::core::AppCore;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {}

/// Open the dialog for `ToolCmd::Versions` (`current`: the size and modified time now).
pub fn open(app: &mut App, _path: PathBuf, _is_dir: bool, _current: Option<(u64, Option<SystemTime>)>) {
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
