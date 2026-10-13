//! The tab history dialog, "Where was I?" (`TuiDialog::TabHistory`, Phase T7, agent A):
//! every place the tab visited as a tree (`models::workspaces::history_rows`, rebuilt by
//! [`tick`] every frame, as it records panes that never moved), the current place
//! highlighted and in view when it opens (the GUI's `ui/workspaces/history.rs`).
//!
//! Keys: Up / Down / PageUp / PageDown / Home / End; Enter goes to the row's place
//! (`WsCmd::Visit`, which revisits the node) and closes; Esc closes. Mouse: a click puts
//! the cursor on a row, a second click goes there; the wheel scrolls. It closes by itself
//! when the tab is closed.

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::workspaces::{WsCmd, history_rows};
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::state::workspace::TabId;
use soyuz_appcore::state::workspace_visits::VisitRow;
use soyuz_appcore::tr;

use super::confirm::{ListClick, list_mouse};
use super::prompt::{LIST_PAGE, click, list_key};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Form, FormOutcome, SelectList};

/// What the dialog keeps while it is open.
#[derive(Debug)]
pub struct State {
    pub tab: TabId,
    /// The pane whose place is current (and that goes there).
    pub pane: PaneId,
    /// The graph's rows as of the last [`tick`].
    pub rows: Vec<VisitRow>,
    pub list: SelectList,
    /// Button: close.
    pub form: Form,
    /// The cursor was put on the current row (once, when the rows first arrived).
    pub placed: bool,
    /// The tab is gone.
    pub gone: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            tab: TabId(0),
            pane: PaneId(0),
            rows: Vec::new(),
            list: SelectList::new(),
            form: form(),
            placed: false,
            gone: false,
        }
    }
}

fn form() -> Form {
    Form::new(Vec::new(), vec![Button::new(tr!("common-close"))])
}

/// Open the dialog for `FrontendCmd::TabHistory` (tab `tab`, starting at pane `pane`).
pub fn open(app: &mut App, tab: TabId, pane: PaneId) {
    let mut s = State { tab, pane, ..State::default() };
    tick(&mut app.core, &mut s);
    app.push_dialog(TuiDialog::TabHistory(s));
}

/// Go to the cursor row's place, unless it is the current one.
fn go(s: &State) -> Outcome {
    match s.rows.get(s.list.cursor) {
        Some(r) if !r.current => {
            Outcome::Close(Some(Cmd::Workspace(WsCmd::Visit { pane: s.pane, location: r.location.clone() })))
        }
        _ => Outcome::Close(None),
    }
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    keys(s, i)
}

fn keys(s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        if list_key(&mut s.list, ev, s.rows.len()) {
            continue;
        }
        match s.form.apply(ev) {
            FormOutcome::Pressed(_) => return go(s),
            FormOutcome::Cancel => return Outcome::Close(None),
            _ => {}
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    match list_mouse(&mut s.list, s.rows.len(), m) {
        Some(ListClick::Row { again: true, .. }) => return go(s),
        Some(_) => return Outcome::Keep,
        None => {}
    }
    match click(&mut s.form, m) {
        FormOutcome::Pressed(_) => Outcome::Close(None),
        _ => Outcome::Keep,
    }
}

/// The dialog went away by itself: the tab was closed.
pub fn stale(core: &AppCore, s: &State) -> bool {
    s.gone || !core.ws.tabs.iter().any(|t| t.id == s.tab)
}

/// Every frame while it is open (`input::dialogs::tick`, from `view::prepare`): the rows
/// rebuilt from the tab's graph; the first time, the cursor goes to the current row.
pub fn tick(core: &mut AppCore, s: &mut State) {
    let Some(rows) = history_rows(&mut core.ws, s.tab, s.pane) else {
        s.gone = true;
        return;
    };
    s.rows = rows;
    if !s.placed && !s.rows.is_empty() {
        s.placed = true;
        if let Some(i) = s.rows.iter().position(|r| r.current) {
            s.list.select(i, s.rows.len());
            s.list.show_cursor(LIST_PAGE);
        }
    }
    s.list.clamp(s.rows.len(), LIST_PAGE);
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::keys::input::InputEvent;
    use soyuz_core::Location;

    use super::*;

    fn row(path: &str, depth: usize, current: bool) -> VisitRow {
        VisitRow {
            node: 0,
            depth,
            location: Location::Dir(path.into()),
            first: 0,
            last: 0,
            count: 1,
            current,
            branches: false,
        }
    }

    fn key(s: &str) -> DialogInput {
        DialogInput { key: None, events: vec![InputEvent::Chord(s.parse().unwrap())] }
    }

    #[test]
    fn enter_goes_to_another_place_and_esc_closes() {
        let mut s = State { rows: vec![row("/a", 0, false), row("/a/b", 1, true)], ..State::default() };
        s.list.select(1, 2);
        // The current place: nothing to do but close.
        assert!(matches!(go(&s), Outcome::Close(None)));
        assert!(matches!(keys(&mut s, &key("Up")), Outcome::Keep));
        match keys(&mut s, &key("Enter")) {
            Outcome::Close(Some(Cmd::Workspace(WsCmd::Visit { location, .. }))) => {
                assert_eq!(location, Location::Dir("/a".into()))
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(keys(&mut s, &key("Esc")), Outcome::Close(None)));
    }
}
