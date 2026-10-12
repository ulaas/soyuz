//! Keys while the filter field is open (step T2.8), in the GUI toolbar filter's order
//! (`soyuz-app/src/ui/toolbar.rs`): the core first (`KeyContext::Omnibar`,
//! `text_focused`, so bindings like Ctrl+T still work), then a `LineEdit` on the focused
//! pane's filter with the unconsumed events. Every change goes through
//! `Pane::set_filter`.
//!
//! The field opens when the core sets `ui.filter_focus`: the Filter action, and
//! type-ahead in filter mode (`[behaviour] type_ahead = "filter"`), which has already put
//! the typed text into the filter. Either way the cursor goes to the end.
//!
//! - Esc clears the filter and gives the keyboard back to the list.
//! - Enter gives the keyboard back to the list, keeping the filter; when the cursor is not
//!   on a matching item, it moves to the first one (like the GUI).
//! - The field closes by itself when the focused pane changes.

use crossterm::event::Event;
use soyuz_appcore::core::{AppCore, Focus};
use soyuz_appcore::state::pane::PaneId;

use super::{TuiFocus, core_input, key_context};
use crate::app::App;
use crate::view::status::{FilterVm, StatusVm, field_width};
use crate::widgets::line_edit::{EditOutcome, LineEdit};

/// What the filter field keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// The filter field owns the keyboard (armed by the core's `ui.filter_focus`).
    pub editing: bool,
    /// Cursor and scroll of the field.
    pub edit: LineEdit,
    /// The pane whose filter is being edited.
    pub pane: Option<PaneId>,
}

/// Open the field on the focused pane's filter with the cursor at the end (also for a
/// click on the field, T2.9).
pub fn open(app: &mut App) {
    let pane = app.core.ws.pane();
    let st = &mut app.input.filter;
    st.editing = true;
    st.edit = LineEdit::at_end(&pane.filter);
    st.pane = Some(pane.id);
}

/// Close the field (the filter stays) and give the keyboard to the list.
fn close(app: &mut App) {
    app.input.filter.editing = false;
    app.input.filter.pane = None;
    app.core.ui.focus = Focus::List;
}

/// Before routing: open the field when the core asked for it (`ui.filter_focus`); close
/// it when the focused pane is no longer the one it edits.
pub fn arm(app: &mut App) {
    if app.core.ui.filter_focus {
        app.core.ui.filter_focus = false;
        open(app);
    } else if app.input.filter.editing && app.input.filter.pane != Some(app.core.ws.pane().id) {
        app.input.filter.editing = false;
        app.input.filter.pane = None;
    }
}

pub fn route(app: &mut App, events: &[Event]) {
    let (kctx, text) = key_context(TuiFocus::Filter);
    let mut frame = core_input(app, events, kctx, text);
    // The core's input may have moved the focus to another pane or tab.
    if !app.input.filter.editing || app.input.filter.pane != Some(app.core.ws.pane().id) {
        return;
    }
    let mut text = app.core.ws.pane().filter.clone();
    let outcome = app.input.filter.edit.feed(&mut text, &mut frame);
    app.core.ws.pane_mut().set_filter(&text);
    match outcome {
        EditOutcome::Cancel => {
            app.core.ws.pane_mut().set_filter("");
            close(app);
        }
        EditOutcome::Submit => {
            close(app);
            enter_first_match(&mut app.core);
        }
        _ => {}
    }
}

/// Enter in the filter: the cursor to the first match unless it is on one already.
fn enter_first_match(core: &mut AppCore) {
    // The visible rows for the filter just typed (the same frame may have changed it).
    core.prepare_visible_panes();
    let pane = core.ws.pane_mut();
    if pane.cursor_pos().is_none() && !pane.visible.is_empty() {
        pane.move_cursor(0, false, false);
    }
}

/// The open field's text as drawn, with the cursor column, for a status line `width`
/// cells wide (`view::status::field_width`). Scrolls `edit` to keep the cursor in view.
fn field_view(edit: &mut LineEdit, core: &AppCore, width: u16) -> (String, u16) {
    let safe = crate::dialog::safe(&core.ws.pane().filter);
    // `safe` only changes control characters, which the editor never inserts (and `view`
    // clamps the cursor should one come from elsewhere).
    edit.view(&safe, field_width(width))
}

/// Before the model is built (`view::prepare`): scroll the open field to its cursor.
pub fn prepare(st: &mut State, core: &AppCore, width: u16) {
    if st.editing {
        field_view(&mut st.edit, core, width);
    }
}

/// While the model is built (`ViewModel::from_core`, after [`prepare`]): the open field's
/// visible text and cursor column and the filter's key hint.
pub fn decorate(st: &State, core: &AppCore, status: &mut StatusVm, width: u16) {
    if !st.editing {
        return;
    }
    // `prepare` scrolled the field already: this view does not move it again.
    let (text, cursor) = field_view(&mut st.edit.clone(), core, width);
    let label = soyuz_appcore::tr!("app-toolbar-filter").to_string();
    status.filter = Some(FilterVm { label, text, cursor: Some(cursor) });
    status.hint = soyuz_appcore::tr!("tui-hint-filter").to_string();
}
