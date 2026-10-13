//! Keys and mouse of the full-screen tools (Phase T7, agent C): the disk usage treemap
//! ([`treemap`]) and the duplicate finder ([`dupes`]), overlays like the compare screen.
//! They draw the core's models (`models::treemap::TreemapModel`,
//! `models::dupes::DupesModel`), the same ones the GUI's tool windows draw.
//!
//! One screen at a time (owner decision 3): opening one closes the other and the compare
//! screen (its scan stops), and opening the compare screen closes it
//! (`crate::input::compare` calls [`close`]). "Reveal" / "Open in pane" only hide it
//! ([`State::hidden`]): the model keeps its results, gets no keys and is not drawn;
//! opening the same tool on the same folder(s) shows it again.
//!
//! While shown it owns the keyboard (`TuiFocus::Tool`, like the compare screen) and every
//! mouse event (`crate::input::mouse` hands its targets here: `Screen*`, `Treemap*`,
//! `Dupes*`, `ConfirmBar`, and everything while [`captures_mouse`]). Popup menu rows run
//! through `MenuCmd::Tool(id)` → [`menu_cmd`]. Like the GUI's tool windows they are modal:
//! keys they do not use stay away from the keymap, except Quit.
//!
//! Delete, Refresh and the context menu follow the user's chords for `Action::Delete`,
//! `Action::Refresh` and `Action::ContextMenu` (`[keys]`), with fixed fallbacks that every
//! terminal can send: Del or X, F5 or Ctrl+R, Shift+F10 (and the Menu key where the
//! terminal reports it). Shifted letters are matched by the typed character's case
//! (terminals differ in whether they report the Shift bit).

pub mod dupes;
pub mod treemap;

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind, MouseEvent};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Host;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_core::config::{Action, KeyChord, KeyName, Mods};

use super::{InputState, TuiFocus, key_context};
use crate::app::App;
use crate::layout::HitTarget;

/// While a scan, a trash job or a flash runs the screen redraws this often (the GUI's
/// rate).
pub const BUSY_REDRAW: Duration = Duration::from_millis(120);

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
    Treemap(Box<treemap::State>),
    Dupes(Box<dupes::State>),
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

/// Show the hidden screen again (the compare screen closes: one at a time).
fn reveal(app: &mut App) {
    crate::input::compare::close(app);
    app.input.screens.hidden = false;
}

/// Open the disk usage treemap on `root` (`ToolCmd::Treemap`, from `crate::tools::drain`).
/// The treemap of the same folder, hidden by Reveal / Open in pane, shows again with its
/// results (like the GUI's window).
pub fn open_treemap(app: &mut App, root: PathBuf) {
    if let Some(Screen::Treemap(t)) = &app.input.screens.screen
        && t.model.root_path() == root.as_path()
    {
        reveal(app);
        return;
    }
    let st = treemap::State::new(root, app.host.wake());
    install(app, Screen::Treemap(Box::new(st)));
}

/// Open the duplicate finder on `roots` (`ToolCmd::Duplicates`). The finder on the same
/// folders, hidden by Reveal, shows again with its results.
pub fn open_dupes(app: &mut App, roots: Vec<PathBuf>) {
    if let Some(Screen::Dupes(d)) = &app.input.screens.screen
        && d.model.roots == roots
        && app.input.screens.hidden
    {
        reveal(app);
        return;
    }
    let st = dupes::State::new(roots, app.host.wake());
    install(app, Screen::Dupes(Box::new(st)));
}

/// Close the screen; dropping its state stops a running scan.
pub fn close(app: &mut App) {
    app.input.screens = State::default();
}

/// Step aside for the pane (Reveal, Open in pane): the screen keeps its model.
pub fn hide(app: &mut App) {
    if app.input.screens.screen.is_some() {
        app.input.screens.hidden = true;
    }
}

/// This frame's events while it has the focus: its own keys first (in order), the rest
/// stay away from the keymap except Quit.
pub fn route(app: &mut App, events: &[Event]) {
    // The Menu key (only with the kitty keyboard protocol) has no chord.
    let menu_key =
        events.iter().any(|e| matches!(e, Event::Key(k) if k.code == KeyCode::Menu && k.kind != KeyEventKind::Release));
    if menu_key {
        open_menu(app);
    }
    let mut frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    for i in 0..frame.events.len() {
        if !shown(&app.input.screens) || app.focus() != TuiFocus::Tool {
            break;
        }
        if frame.consumed[i] {
            continue;
        }
        let ev = frame.events[i].clone();
        // A letter's case comes from the text it types when there is some.
        let upper = match (&ev, frame.events.get(i + 1)) {
            (InputEvent::Chord(_), Some(InputEvent::Text(t))) => t.chars().next().map(char::is_uppercase),
            _ => None,
        };
        let used = match &app.input.screens.screen {
            Some(Screen::Treemap(_)) => treemap::key(app, &ev),
            Some(Screen::Dupes(_)) => dupes::key(app, &ev, upper),
            None => true,
        };
        if used {
            frame.consumed[i] = true;
        }
    }
    if shown(&app.input.screens) && app.focus() == TuiFocus::Tool {
        let (kctx, text) = key_context(TuiFocus::Tool);
        for i in 0..frame.events.len() {
            let quit = chord(&frame.events[i])
                .is_some_and(|c| app.core.keymap.lookup(kctx, c).is_some_and(|a| *a == Action::Quit));
            if !quit {
                frame.consumed[i] = true;
            }
        }
        app.core.input(&app.host, &mut frame, kctx, text);
    }
}

/// The shown screen's context menu (Menu key, Shift+F10, the keymap's chord).
fn open_menu(app: &mut App) {
    match &app.input.screens.screen {
        Some(Screen::Treemap(_)) => treemap::open_menu(app, None),
        Some(Screen::Dupes(_)) => dupes::open_menu(app, None),
        None => {}
    }
}

/// A mouse event over one of its click targets (or anywhere while it shows); returns
/// whether it was taken.
pub fn on_mouse(app: &mut App, target: HitTarget, ev: &MouseEvent) -> bool {
    if !shown(&app.input.screens) {
        return false;
    }
    match &app.input.screens.screen {
        Some(Screen::Treemap(_)) => treemap::on_mouse(app, target, ev),
        Some(Screen::Dupes(_)) => dupes::on_mouse(app, target, ev),
        None => {}
    }
    true
}

/// Every frame (`App::frame`): a redraw while the shown screen is busy (a scan, a trash
/// job, the "keep one" flash).
pub fn tick(st: &State, host: &dyn Host) {
    if !shown(st) {
        return;
    }
    let busy = match &st.screen {
        Some(Screen::Treemap(t)) => t.model.is_busy(),
        Some(Screen::Dupes(d)) => d.model.is_running() || d.model.busy() || d.model.flashing(),
        None => false,
    };
    if busy {
        host.redraw_after(BUSY_REDRAW);
    }
}

/// A row of the screen's popup menu (`MenuCmd::Tool(id)`, from `crate::input::menu`).
pub fn menu_cmd(app: &mut App, id: u32) {
    match &app.input.screens.screen {
        Some(Screen::Treemap(_)) => treemap::menu_cmd(app, id),
        Some(Screen::Dupes(_)) => dupes::menu_cmd(app, id),
        None => {}
    }
}

// ---------------------------------------------------------------------------------------
// Keys both screens read.

pub(crate) fn chord(ev: &InputEvent) -> Option<KeyChord> {
    match ev {
        InputEvent::Chord(c) => Some(*c),
        _ => None,
    }
}

pub(crate) fn plain(c: &KeyChord) -> bool {
    !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT)
}

pub(crate) fn shifted(c: &KeyChord) -> bool {
    c.mods.contains(Mods::SHIFT) && !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD)
}

/// `c` is one of the user's chords for `action`.
fn bound(core: &AppCore, action: &Action, c: &KeyChord) -> bool {
    core.keymap.chords_for(action).contains(c)
}

/// Delete: the keymap's chords, Del, or X (laptops without Del, legacy terminals).
pub(crate) fn is_delete(core: &AppCore, c: &KeyChord) -> bool {
    bound(core, &Action::Delete, c) || (plain(c) && matches!(c.key, KeyName::Delete | KeyName::X))
}

/// Refresh: the keymap's chords, F5 or Ctrl+R.
pub(crate) fn is_refresh(core: &AppCore, c: &KeyChord) -> bool {
    let ctrl = c.mods == Mods::CTRL;
    bound(core, &Action::Refresh, c) || (plain(c) && c.key == KeyName::F5) || (ctrl && c.key == KeyName::R)
}

/// The context menu: the keymap's chords or Shift+F10.
pub(crate) fn is_menu(core: &AppCore, c: &KeyChord) -> bool {
    bound(core, &Action::ContextMenu, c) || (shifted(c) && c.key == KeyName::F10)
}

/// The Delete hint: the user's first chord for Delete ("Del" when unbound).
pub(crate) fn delete_label(core: &AppCore) -> String {
    soyuz_appcore::keys::chord_label(&core.keymap, &Action::Delete).unwrap_or_else(|| "Del".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_screens_have_no_focus() {
        let mut st = State::default();
        assert!(!shown(&st));
        st.screen = Some(Screen::Dupes(Box::new(dupes::State::new(Vec::new(), soyuz_appcore::host::no_wake()))));
        assert!(shown(&st));
        st.hidden = true;
        assert!(!shown(&st));
    }

    #[test]
    fn chord_kinds() {
        let c = |mods, key| KeyChord::new(mods, key);
        assert!(plain(&c(Mods::empty(), KeyName::A)));
        assert!(shifted(&c(Mods::SHIFT, KeyName::N)));
        assert!(!shifted(&c(Mods::SHIFT | Mods::CTRL, KeyName::N)));
    }
}
