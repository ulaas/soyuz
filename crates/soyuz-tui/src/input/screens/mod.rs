//! Keys and mouse of the full-screen tools (Phase T7, agent C): the disk usage treemap
//! ([`treemap`]) and the duplicate finder ([`dupes`]), overlays like the compare screen.
//!
//! One screen at a time (owner decision 3): opening one closes the other and the compare
//! screen (its scan stops), and opening the compare screen closes it
//! (`crate::input::compare` calls [`close`]). "Reveal" / "Open in pane" only hide it
//! ([`State::hidden`]): the model keeps its results, gets no keys and is not drawn;
//! opening the same tool on the same folder shows it again.
//!
//! While shown it owns the keyboard (`TuiFocus::Tool`, like the compare screen) and every
//! mouse event (`crate::input::mouse` hands its targets here: `Screen*`, `Treemap*`,
//! `Dupes*`, `ConfirmBar`, and everything while [`captures_mouse`]). Popup menu rows run
//! through `MenuCmd::Tool(id)` → [`menu_cmd`].
//!
//! Skeleton (Phase T7 wave 0): the open functions say "not available" until agent C
//! fills them.

pub mod dupes;
pub mod treemap;

use std::path::PathBuf;

use crossterm::event::{Event, MouseEvent};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Host;
use soyuz_core::config::KeyContext;

use super::InputState;
use crate::app::App;
use crate::layout::HitTarget;

/// What the tool screens keep between frames (`InputState::screens`).
#[derive(Debug, Default)]
pub struct State {
    /// The open screen, if any.
    pub screen: Option<Screen>,
    /// The screen's model lives on (Reveal / Open in pane) but has no focus and is not
    /// drawn.
    pub hidden: bool,
}

/// The open full-screen tool.
#[derive(Debug)]
pub enum Screen {
    Treemap(treemap::State),
    Dupes(dupes::State),
}

/// A screen is open and not hidden.
pub fn shown(st: &State) -> bool {
    st.screen.is_some() && !st.hidden
}

/// It owns the keyboard this frame (`input::focus`).
pub fn has_focus(_core: &AppCore, st: &InputState) -> bool {
    shown(&st.screens)
}

/// While it shows, every mouse event is its own.
pub fn captures_mouse(_core: &AppCore, st: &InputState) -> bool {
    shown(&st.screens)
}

/// Show `screen`, closing the compare screen and any other tool screen first (their scans
/// stop).
pub fn install(app: &mut App, screen: Screen) {
    crate::input::compare::close(app);
    close(app);
    app.input.screens = State { screen: Some(screen), hidden: false };
}

/// Open the disk usage treemap on `root` (`ToolCmd::Treemap`, from `crate::tools::drain`).
pub fn open_treemap(app: &mut App, _root: PathBuf) {
    app.core.tui_unavailable();
}

/// Open the duplicate finder on `roots` (`ToolCmd::Duplicates`).
pub fn open_dupes(app: &mut App, _roots: Vec<PathBuf>) {
    app.core.tui_unavailable();
}

/// Close the screen; dropping its state stops a running scan.
pub fn close(app: &mut App) {
    app.input.screens = State::default();
}

/// This frame's events while it has the focus. Skeleton: the core's bindings in the
/// dialog context (Quit still works).
pub fn route(app: &mut App, events: &[Event]) {
    let _ = super::core_input(app, events, KeyContext::Dialog, true);
}

/// A mouse event over one of its click targets (or anywhere while it shows); returns
/// whether it was taken.
pub fn on_mouse(app: &mut App, _target: HitTarget, _ev: &MouseEvent) -> bool {
    shown(&app.input.screens)
}

/// Every frame (`App::frame`): a redraw while a scan runs (`host.redraw_after`).
pub fn tick(_st: &State, _host: &dyn Host) {}

/// A row of the screen's popup menu (`MenuCmd::Tool(id)`, from `crate::input::menu`).
pub fn menu_cmd(_app: &mut App, _id: u32) {}
