//! Choose the columns of a details view (`FrontendCmd::ChooseColumns`): a list of the
//! columns with a tick each (`soyuz_appcore::models::requests::columns`); Space or a
//! click shows / hides the cursor row's column right away (Name stays), Enter or Done
//! closes.

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::columns;
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::Column;
use soyuz_core::config::KeyName;

use super::confirm::{ListClick, click, list_mouse};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Form, FormOutcome, SelectList};
use soyuz_appcore::keys::input::InputEvent;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    /// Buttons: Done.
    pub form: Form,
    pub pane: Option<PaneId>,
    /// The columns offered, in list order.
    pub cols: Vec<Column>,
    pub list: SelectList,
}

/// Open the dialog (from `crate::frontend`).
pub fn open(app: &mut App, pane: PaneId) {
    let form = Form::new(Vec::new(), vec![Button::new(tr!("dialogs-done"))]);
    let s = State {
        title: tr!("dialogs-columns-title").into(),
        form,
        pane: Some(pane),
        cols: columns::choosable(),
        list: SelectList::new(),
    };
    app.push_dialog(TuiDialog::Columns(s));
}

/// Show or hide the column on row `i`; the view is redrawn (`Cmd::ViewChanged`).
pub fn toggle(core: &mut AppCore, s: &State, i: usize) {
    let Some(&col) = s.cols.get(i) else { return };
    let id = s.pane.unwrap_or_else(|| core.ws.pane().id);
    let Some(pane) = core.ws.find_pane_mut(id) else { return };
    let on = !columns::is_visible(pane, col);
    if columns::set_visible(pane, col, on) {
        core.cmds.push(Cmd::ViewChanged(id));
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        if s.list.apply(ev, s.cols.len(), height) {
            continue;
        }
        if let InputEvent::Chord(c) = ev
            && c.key == KeyName::Space
            && c.mods.is_empty()
        {
            toggle(core, s, s.list.cursor);
            continue;
        }
        match s.form.apply(ev) {
            FormOutcome::Pressed(_) | FormOutcome::Cancel => return Outcome::Close(None),
            _ => {}
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    if let Some(ListClick::Row { i, .. }) = list_mouse(&mut s.list, s.cols.len(), e) {
        toggle(core, s, i);
        return Outcome::Keep;
    }
    match click(&mut s.form, e) {
        Some(FormOutcome::Pressed(_)) => Outcome::Close(None),
        _ => Outcome::Keep,
    }
}

/// The dialog went away by itself: its pane closed.
pub fn stale(core: &AppCore, s: &State) -> bool {
    s.pane.is_some_and(|p| core.ws.find_pane(p).is_none())
}
