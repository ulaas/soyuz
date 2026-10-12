//! Keys while the address field is open (`core.ui.address.editing`, opened by
//! `Action::FocusAddressBar` (Ctrl+L), `GoTo` or `EditPath`).
//!
//! Key order, as in the GUI: the core's input runs first with `KeyContext::Omnibar` and
//! `text_focused` (bindings that are not text editing, e.g. Ctrl+T, still work), then a
//! [`LineEdit`] on `ui.address.text` takes the unconsumed events. Its outcomes drive the
//! shared editing logic of `soyuz_appcore::state::crumbs` (the GUI's too):
//!
//! | Key | Does |
//! |---|---|
//! | Tab | accepts the highlighted completion, else the first (`crumbs::tab`) |
//! | Down / Up, Shift+Tab | moves the highlight through the completions (wrapping) |
//! | Enter | accepts the highlighted completion, then navigates (`crumbs::enter`) |
//! | Esc | leaves the field without navigating (`crumbs::cancel`) |
//!
//! On entry the cursor goes to the end; a folder path gets a trailing separator, so typing
//! a name completes inside the folder at once (the GUI selects the whole text instead,
//! which a terminal field without selection cannot).

use std::cell::Cell;

use crossterm::event::Event;
use soyuz_appcore::state::crumbs;
use soyuz_core::Location;

use super::{TuiFocus, core_input, key_context};
use crate::app::App;
use crate::widgets::line_edit::{EditOutcome, LineEdit};

/// What the address field keeps between frames.
#[derive(Debug, Default)]
pub struct State {
    /// Cursor and scroll of the field.
    pub edit: LineEdit,
}

thread_local! {
    /// The field's cursor (byte offset in `ui.address.text`) after the last input, for
    /// the view (`crate::view::crumbs`), which only sees the core. `None`: not editing.
    static CURSOR: Cell<Option<usize>> = const { Cell::new(None) };
}

/// The address field's cursor as of the last input (`None` when it was not edited yet).
pub fn cursor() -> Option<usize> {
    CURSOR.get()
}

/// The field was just opened (`request_focus`): cursor at the end, a separator after a
/// folder path.
fn enter_field(app: &mut App) {
    let a = &mut app.core.ui.address;
    a.request_focus = false;
    if matches!(Location::parse(&a.text), Some(Location::Dir(_))) && !a.text.ends_with(['/', '\\']) {
        a.text.push(if a.text.contains('\\') { '\\' } else { std::path::MAIN_SEPARATOR });
    }
    app.input.address.edit = LineEdit::at_end(&a.text);
}

pub fn route(app: &mut App, events: &[Event]) {
    if app.core.ui.address.request_focus {
        enter_field(app);
    }
    let (kctx, text) = key_context(TuiFocus::Address);
    let mut frame = core_input(app, events, kctx, text);
    if !app.core.ui.address.editing {
        CURSOR.set(None);
        return;
    }
    // A binding re-opened the field (Ctrl+L again).
    if app.core.ui.address.request_focus {
        enter_field(app);
    }
    loop {
        let before = app.core.ui.address.text.clone();
        let out = app.input.address.edit.feed(&mut app.core.ui.address.text, &mut frame);
        // The completion of the text as typed so far; this also requests the typed
        // folder's listing while the field is open (every frame).
        let comp = crumbs::completion(&mut app.core.services, &app.core.ui.address.text);
        let a = &mut app.core.ui.address;
        // Editing the text drops the highlight (like the GUI's `changed()`).
        if a.text != before {
            a.completion = None;
        }
        match out {
            EditOutcome::Consumed | EditOutcome::Ignored => break,
            EditOutcome::Submit => {
                let pane = app.core.ws.pane().id;
                crumbs::enter(&mut app.core.ui.address, &mut app.core.services, pane, &mut app.core.cmds, &comp);
                CURSOR.set(None);
                return;
            }
            EditOutcome::Cancel => {
                crumbs::cancel(a);
                a.text = app.core.ws.pane().location.display();
                CURSOR.set(None);
                return;
            }
            EditOutcome::Tab => {
                if crumbs::tab(a, &comp.names) {
                    app.input.address.edit = LineEdit::at_end(&a.text);
                }
            }
            EditOutcome::Down => crumbs::cycle(a, comp.names.len(), true),
            EditOutcome::Up | EditOutcome::ShiftTab => crumbs::cycle(a, comp.names.len(), false),
            EditOutcome::PageUp | EditOutcome::PageDown => {}
        }
    }
    CURSOR.set(Some(app.input.address.edit.cursor()));
}
