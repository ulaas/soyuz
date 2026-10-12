//! The popup menu (`TuiFocus::Menu`): the context menu of the focused item
//! (`ContextMenu`, Shift+F10; a right click) — Phase T3 fills [`open_context`].
//!
//! While a menu is open it owns the keyboard and the mouse: keys go to
//! `widgets::Menu::apply`, a press on a `MenuItem` target to `Menu::click`, a press
//! anywhere else closes it (and does nothing else). An activated entry closes the menu
//! and runs its [`MenuCmd`].

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::tr;
use soyuz_core::config::Action;

use crate::app::App;
use crate::layout::HitTarget;
use crate::widgets::{Menu, MenuOutcome};

/// What a menu entry runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuCmd {
    /// A core action (`Cmd::Action`).
    Action(Action),
}

/// The open menu, if any.
#[derive(Debug, Default)]
pub struct State {
    pub menu: Option<Menu<MenuCmd>>,
}

/// Whether a menu is open.
pub fn is_open(st: &State) -> bool {
    st.menu.is_some()
}

/// `ContextMenu` (Shift+F10): the menu of the focused item (T3).
pub fn open_context(app: &mut App) {
    app.unavailable(tr!("tui-feature-menu"));
}

/// Open `menu` (replacing an open one).
pub fn open(app: &mut App, menu: Menu<MenuCmd>) {
    app.input.menu.menu = Some(menu);
}

fn run(app: &mut App, c: MenuCmd) {
    app.input.menu.menu = None;
    match c {
        MenuCmd::Action(a) => app.core.cmds.push(Cmd::Action(a)),
    }
}

/// Keys while a menu is open.
pub fn route(app: &mut App, events: &[Event]) {
    let screen = app.regions().area;
    let frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    for ev in &frame.events {
        let Some(menu) = app.input.menu.menu.as_mut() else { return };
        match menu.apply(ev, screen) {
            MenuOutcome::Activated(c) => run(app, c),
            MenuOutcome::Close => app.input.menu.menu = None,
            MenuOutcome::Consumed | MenuOutcome::Ignored => {}
        }
    }
}

/// A mouse event while a menu is open (`crate::input::mouse` hands it over).
pub fn mouse(app: &mut App, m: &MouseEvent) {
    let MouseEventKind::Down(button) = m.kind else { return };
    let screen = app.regions().area;
    let target = crate::input::mouse::hit(&app.hits, m.column, m.row).map(|(_, t)| t);
    let Some(menu) = app.input.menu.menu.as_mut() else { return };
    match target {
        Some(HitTarget::MenuItem { depth, index }) if button == MouseButton::Left => {
            match menu.click(usize::from(depth), index, screen) {
                MenuOutcome::Activated(c) => run(app, c),
                MenuOutcome::Close => app.input.menu.menu = None,
                MenuOutcome::Consumed | MenuOutcome::Ignored => {}
            }
        }
        Some(HitTarget::MenuItem { .. }) => {}
        _ => app.input.menu.menu = None,
    }
}
