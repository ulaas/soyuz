//! Properties of the selection (`FrontendCmd::Properties`) and of a folder from the tree
//! (`FrontendCmd::FolderProperties`) — Phase T3.
//!
//! Stub: [`open`] / [`open_folder`] show the "not available in the terminal UI yet" toast
//! of Phase T1. Nothing is pushed until the dialog exists (see the shape of a kind file
//! in `super`).

use std::path::PathBuf;

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::properties::{Start, Tab};
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::EntryInfo;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;
use crate::widgets::Form;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub form: Form,
}

/// Open the properties of `entries` (pane `pane`) on `tab`.
pub fn open(app: &mut App, pane: PaneId, entries: Vec<EntryInfo>, tab: Tab, start: Option<Start>) {
    let _ = (pane, entries, tab, start);
    app.unavailable(tr!("tui-feature-properties"));
}

/// Open the properties of folder `dir` (the folder tree).
pub fn open_folder(app: &mut App, dir: PathBuf) {
    let _ = dir;
    app.unavailable(tr!("tui-feature-properties"));
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    super::form_input(&mut s.form, i)
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, _s: &mut State, _e: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The dialog went away by itself.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
