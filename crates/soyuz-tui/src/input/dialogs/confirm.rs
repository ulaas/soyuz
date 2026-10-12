//! A confirmation before a command (`FrontendCmd::Confirm`): the message, then the
//! confirm button (focused, Enter) and Cancel (Esc). The command runs only on confirm
//! (`soyuz_appcore::models::requests::confirm`).
//!
//! Also the mouse helpers the other request dialogs share ([`click`], [`list_mouse`]).

use crossterm::event::{MouseButton, MouseEventKind};
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::confirm::ConfirmModel;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::layout::HitTarget;
use crate::widgets::{Button, Form, FormOutcome, SelectList};

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub form: Form,
    /// The message and the guarded command (`None` only in a default state).
    pub model: Option<ConfirmModel>,
}

/// Open the dialog (from `crate::frontend`).
pub fn open(app: &mut App, title: String, message: String, confirm: String, on_confirm: Cmd) {
    let form = Form::new(Vec::new(), vec![Button::new(confirm.as_str()), Button::new(tr!("common-cancel"))]);
    let model = ConfirmModel::new(title, message, confirm, on_confirm);
    app.push_dialog(TuiDialog::Confirm(State { title: model.title.clone(), form, model: Some(model) }));
}

fn outcome(s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => Some(Outcome::Close(s.model.as_mut().and_then(ConfirmModel::confirm))),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        let o = s.form.apply(ev);
        if let Some(out) = outcome(s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    match click(&mut s.form, e).and_then(|o| outcome(s, o)) {
        Some(out) => out,
        None => Outcome::Keep,
    }
}

/// The dialog went away by itself (its pane or files are gone, …).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

/// A left click on one of the form's fields or buttons (`None`: not one).
pub(crate) fn click(form: &mut Form, e: &DialogMouse) -> Option<FormOutcome> {
    if !matches!(e.event.kind, MouseEventKind::Down(MouseButton::Left)) {
        return None;
    }
    match e.target? {
        HitTarget::DialogButton(i) => Some(form.click_button(i)),
        HitTarget::DialogField(i) => Some(form.click_field(i)),
        _ => None,
    }
}

/// What a mouse event did to a dialog's list of `len` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListClick {
    /// A left click on row `i` (now the cursor row); `again`: it already was.
    Row { i: usize, again: bool },
    /// The wheel scrolled it.
    Scrolled,
}

/// Wheel and row clicks on a dialog's list (rows shown at most: `MAX_LIST_ROWS`).
pub(crate) fn list_mouse(list: &mut SelectList, len: usize, e: &DialogMouse) -> Option<ListClick> {
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    match e.event.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let rows = if e.event.kind == MouseEventKind::ScrollDown {
                crate::input::mouse::WHEEL_ROWS
            } else {
                -crate::input::mouse::WHEEL_ROWS
            };
            list.wheel(rows, len, height);
            Some(ListClick::Scrolled)
        }
        MouseEventKind::Down(MouseButton::Left) => match e.target? {
            HitTarget::DialogRow(i) if i < len => {
                let again = list.cursor == i;
                list.select(i, len);
                Some(ListClick::Row { i, again })
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

    use super::*;

    fn state() -> State {
        let form = Form::new(Vec::new(), vec![Button::new("Do it"), Button::new("Cancel")]);
        State { title: "T".into(), form, model: Some(ConfirmModel::new("T", "M", "Do it", Cmd::CopyText("x".into()))) }
    }

    fn key(code: KeyCode) -> DialogInput {
        let ev = crossterm::event::Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        super::super::dialog_input(&ev, true).unwrap()
    }

    #[test]
    fn enter_confirms_and_esc_cancels() {
        let mut core_less = state();
        let o = outcome(&mut core_less, FormOutcome::Pressed(0));
        assert!(matches!(o, Some(Outcome::Close(Some(Cmd::CopyText(_))))));
        let mut s = state();
        assert!(matches!(outcome(&mut s, FormOutcome::Cancel), Some(Outcome::Close(None))));
        // Enter on the focused confirm button.
        let mut s = state();
        let mut pressed = None;
        for ev in &key(KeyCode::Enter).events {
            let o = s.form.apply(ev);
            pressed = pressed.or_else(|| outcome(&mut s, o));
        }
        assert!(matches!(pressed, Some(Outcome::Close(Some(_)))), "{pressed:?}");
    }

    #[test]
    fn list_clicks_and_wheel() {
        let mut l = SelectList::new();
        let ev = |kind, target| DialogMouse {
            event: MouseEvent { kind, column: 0, row: 0, modifiers: KeyModifiers::NONE },
            target,
        };
        let down = MouseEventKind::Down(MouseButton::Left);
        assert_eq!(
            list_mouse(&mut l, 5, &ev(down, Some(HitTarget::DialogRow(2)))),
            Some(ListClick::Row { i: 2, again: false })
        );
        assert_eq!(
            list_mouse(&mut l, 5, &ev(down, Some(HitTarget::DialogRow(2)))),
            Some(ListClick::Row { i: 2, again: true })
        );
        assert_eq!(list_mouse(&mut l, 5, &ev(down, Some(HitTarget::DialogRow(9)))), None);
        assert_eq!(list_mouse(&mut l, 30, &ev(MouseEventKind::ScrollDown, None)), Some(ListClick::Scrolled));
        assert_eq!(l.top, 3);
    }
}
