//! The "Manage workspaces" dialog (`TuiDialog::ManageWorkspaces`, Phase T7, agent A): the
//! saved workspaces (`models::workspaces::store_snapshot`, re-read every frame, so the
//! list follows the commands) with open, edit (name, hotkey), move up and delete, like
//! the GUI's `ManageWorkspacesDialog` (opened by `Action::ManageWorkspaces`, "Manage
//! workspaces…" in the palette). "New window" is not offered: the terminal UI has
//! one screen.
//!
//! Keys: Up / Down / PageUp / PageDown / Home / End move the cursor; Enter opens the
//! workspace (and closes the dialog, as the whole workspace changes under it); F2 edits;
//! Alt+Up moves it up; Del deletes it at once, with no undo (the GUI's rule, owner
//! decision 5); the buttons' letters work too (legacy terminals). In edit mode the name
//! and hotkey fields show under the list: Enter (Done) checks them (a blank or taken name
//! and a hotkey that is not a key chord are refused with a message) and queues
//! `WsCmd::Rename` + `WsCmd::SetHotkey`; Esc leaves edit mode, a second Esc closes.
//! Mouse: a click puts the cursor on a row, a second click opens it; the wheel scrolls.

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::workspaces::{WsCmd, store_snapshot};
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::confirm::{ListClick, list_mouse};
use super::prompt::{LIST_PAGE, buttons, click, is_key, list_key};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Field, Form, FormOutcome, SelectList};

/// Buttons of the list, in order.
pub const OPEN: usize = 0;
pub const EDIT: usize = 1;
pub const UP: usize = 2;
pub const DELETE: usize = 3;
pub const CLOSE: usize = 4;
/// Buttons in edit mode.
pub const DONE: usize = 0;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// The cursor in the list of workspaces.
    pub list: SelectList,
    /// The list's buttons, or in edit mode the name and hotkey fields with Done / Cancel.
    pub form: Form,
    /// The row being edited.
    pub editing: Option<usize>,
}

/// Open the dialog for `FrontendCmd::ManageWorkspaces`.
pub fn open(app: &mut App) {
    app.push_dialog(TuiDialog::ManageWorkspaces(state()));
}

/// A new dialog's state (the list's buttons, the cursor on the first row).
pub fn state() -> State {
    let mut s = State { form: list_form(), ..State::default() };
    sync(&mut s, store_snapshot().len());
    s
}

fn list_form() -> Form {
    let labels = [
        tr!("workspaces-manage-open"),
        tr!("workspaces-manage-edit"),
        tr!("workspaces-manage-up"),
        tr!("common-delete"),
        tr!("common-close"),
    ];
    let mut f = Form::new(Vec::new(), buttons(&labels)).default_button(OPEN);
    f.focus_button(OPEN);
    f
}

fn edit_form(name: &str, hotkey: &str) -> Form {
    Form::new(
        vec![Field::text(tr!("tui-workspaces-name"), name), Field::text(tr!("tui-workspaces-hotkey"), hotkey)],
        buttons(&[tr!("workspaces-manage-done"), tr!("common-cancel")]),
    )
    .default_button(DONE)
}

/// Keep the cursor in a list of `n` rows and the buttons that need a row (or a row above)
/// enabled only then.
fn sync(s: &mut State, n: usize) {
    s.list.clamp(n, LIST_PAGE);
    if s.editing.is_some() {
        return;
    }
    let row = n > 0;
    for (i, b) in s.form.buttons.iter_mut().enumerate() {
        b.enabled = match i {
            OPEN | EDIT | DELETE => row,
            UP => row && s.list.cursor > 0,
            _ => true,
        };
    }
    if s.form.focused_button().is_none_or(|b| !s.form.buttons[b].enabled) {
        s.form.focus_button(if row { OPEN } else { CLOSE });
    }
}

fn start_edit(s: &mut State) {
    let list = store_snapshot();
    let Some((name, hotkey, _)) = list.get(s.list.cursor) else { return };
    s.editing = Some(s.list.cursor);
    s.form = edit_form(name, hotkey);
}

fn end_edit(s: &mut State) {
    s.editing = None;
    s.form = list_form();
    sync(s, store_snapshot().len());
}

/// Done in edit mode: the fields checked, then the commands for the row.
fn finish_edit(core: &mut AppCore, s: &mut State) {
    let Some(i) = s.editing else { return };
    let value = |k: usize| s.form.fields.get(k).and_then(|f| f.value()).unwrap_or_default().trim().to_string();
    let (name, hotkey) = (value(0), value(1));
    let list = store_snapshot();
    let error = if name.is_empty() {
        Some(tr!("tui-workspaces-name-empty").to_string())
    } else if list.iter().enumerate().any(|(k, (n, _, _))| k != i && *n == name) {
        Some(tr!("workspaces-name-exists").to_string())
    } else if !hotkey.is_empty() && soyuz_core::config::parse_sequence(&hotkey).is_err() {
        Some(tr!("workspaces-bad-chord", chord = &hotkey))
    } else {
        None
    };
    if let Some(e) = error {
        s.form.error = Some(e);
        return;
    }
    core.cmds.push(Cmd::Workspace(WsCmd::Rename(i, name)));
    core.cmds.push(Cmd::Workspace(WsCmd::SetHotkey(i, hotkey)));
    end_edit(s);
}

fn move_up(core: &mut AppCore, s: &mut State) {
    let i = s.list.cursor;
    if i > 0 && i < store_snapshot().len() {
        core.cmds.push(Cmd::Workspace(WsCmd::MoveUp(i)));
        s.list.cursor = i - 1;
        s.list.show_cursor(LIST_PAGE);
    }
}

fn delete(core: &mut AppCore, s: &mut State) {
    if s.list.cursor < store_snapshot().len() {
        core.cmds.push(Cmd::Workspace(WsCmd::Delete(s.list.cursor)));
    }
}

fn opened(s: &State) -> Option<Outcome> {
    (s.list.cursor < store_snapshot().len()).then_some(Outcome::Close(Some(Cmd::Workspace(WsCmd::Open(s.list.cursor)))))
}

fn outcome(core: &mut AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    if s.editing.is_some() {
        match o {
            FormOutcome::Pressed(DONE) => finish_edit(core, s),
            FormOutcome::Pressed(_) | FormOutcome::Cancel => end_edit(s),
            FormOutcome::Changed(_) => s.form.error = None,
            _ => {}
        }
        return None;
    }
    match o {
        FormOutcome::Pressed(OPEN) => opened(s),
        FormOutcome::Pressed(EDIT) => {
            start_edit(s);
            None
        }
        FormOutcome::Pressed(UP) => {
            move_up(core, s);
            None
        }
        FormOutcome::Pressed(DELETE) => {
            delete(core, s);
            None
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        _ => None,
    }
}

fn chord_is(ev: &InputEvent, key: KeyName, mods: Mods) -> bool {
    matches!(ev, InputEvent::Chord(c) if c.key == key && c.mods == mods)
}

/// One event in list mode: whether it was one of the list's own keys (Enter presses the
/// focused button, Open unless another was picked).
fn list_event(core: &mut AppCore, s: &mut State, ev: &InputEvent, n: usize) -> bool {
    if list_key(&mut s.list, ev, n) {
        return true;
    }
    if is_key(ev, KeyName::F2) {
        start_edit(s);
    } else if chord_is(ev, KeyName::ArrowUp, Mods::ALT) {
        move_up(core, s);
    } else if is_key(ev, KeyName::Delete) {
        delete(core, s);
    } else {
        return false;
    }
    true
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        let n = store_snapshot().len();
        sync(s, n);
        if s.editing.is_none() && list_event(core, s, ev, n) {
            continue;
        }
        let o = s.form.apply(ev);
        let out = outcome(core, s, o);
        if let Some(out) = out {
            return out;
        }
    }
    sync(s, store_snapshot().len());
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    let n = store_snapshot().len();
    sync(s, n);
    if s.editing.is_none() {
        match list_mouse(&mut s.list, n, m) {
            Some(ListClick::Row { again: true, .. }) => return opened(s).unwrap_or(Outcome::Keep),
            Some(_) => {
                sync(s, n);
                return Outcome::Keep;
            }
            None => {}
        }
    }
    let o = click(&mut s.form, m);
    let out = outcome(core, s, o).unwrap_or(Outcome::Keep);
    sync(s, store_snapshot().len());
    out
}

/// The dialog went away by itself: never (the list follows the store).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_follow_the_rows() {
        let mut s = State { form: list_form(), ..State::default() };
        sync(&mut s, 0);
        let on: Vec<bool> = s.form.buttons.iter().map(|b| b.enabled).collect();
        assert_eq!(on, [false, false, false, false, true], "an empty store offers Close only");
        assert_eq!(s.form.focused_button(), Some(CLOSE));
        sync(&mut s, 3);
        assert!(!s.form.buttons[UP].enabled, "the first row cannot move up");
        s.list.cursor = 2;
        sync(&mut s, 3);
        assert!(s.form.buttons[UP].enabled);
        // The cursor stays inside a list that shrank.
        sync(&mut s, 1);
        assert_eq!(s.list.cursor, 0);
    }

    #[test]
    fn edit_mode_swaps_the_form() {
        let mut s = State { form: edit_form("Work", "Ctrl+Alt+1"), editing: Some(0), ..State::default() };
        assert_eq!(s.form.fields.len(), 2);
        assert_eq!(s.form.fields[1].value(), Some("Ctrl+Alt+1"));
        end_edit(&mut s);
        assert!(s.editing.is_none());
        assert!(s.form.fields.is_empty());
        assert_eq!(s.form.buttons.len(), 5);
    }
}
