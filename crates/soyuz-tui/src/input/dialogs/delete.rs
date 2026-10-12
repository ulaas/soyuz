//! The delete / purge / empty-trash confirmation (`DialogModel::DeleteConfirm`), like the
//! GUI's `ui/dialogs/delete_confirm.rs`: Cancel and the confirm button of the model
//! ("Move to trash", "Delete", "Wipe", "Empty trash"), which has the focus. Enter
//! confirms, Esc cancels, letters press the buttons.

use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;

use super::prompt::{buttons, click};
use super::{DialogInput, DialogMouse, ModelUi, Outcome};
use crate::widgets::{Form, FormOutcome};

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(m: &DialogModel) -> ModelUi {
    let DialogModel::DeleteConfirm(d) = m else { return ModelUi::default() };
    let (_, confirm, _) = d.texts();
    let mut form = Form::new(Vec::new(), buttons(&[tr!("common-cancel"), confirm])).default_button(1);
    form.focus_button(1);
    ModelUi { form, ..ModelUi::default() }
}

fn on_form(m: &mut DialogModel, out: FormOutcome) -> Option<Outcome> {
    let DialogModel::DeleteConfirm(d) = m else { return None };
    match out {
        FormOutcome::Pressed(1) => Some(Outcome::Close(d.confirm())),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => {
            d.cancel();
            Some(Outcome::Close(None))
        }
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(m: &mut DialogModel, ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        let out = ui.form.apply(ev);
        if let Some(o) = on_form(m, out) {
            return o;
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(m: &mut DialogModel, ui: &mut ModelUi, e: &DialogMouse) -> Outcome {
    let out = click(&mut ui.form, e);
    on_form(m, out).unwrap_or(Outcome::Keep)
}

/// The prompt went away by itself.
pub fn stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use soyuz_appcore::cmd::Cmd;
    use soyuz_appcore::keys::input::InputEvent;
    use soyuz_appcore::models::dialogs::DeleteConfirmModel;
    use soyuz_appcore::ops::fileops::FileOpCmd;
    use soyuz_jobs::DeleteMode;

    use super::*;

    fn chord(s: &str) -> DialogInput {
        DialogInput {
            key: Some(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)),
            events: vec![InputEvent::Chord(s.parse().unwrap())],
        }
    }

    fn model(mode: DeleteMode) -> DialogModel {
        DeleteConfirmModel::new(vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")], mode).into()
    }

    #[test]
    fn enter_confirms_esc_and_cancel_do_not() {
        let mut m = model(DeleteMode::Permanent);
        let mut u = ui(&m);
        assert_eq!(u.form.focused_button(), Some(1));
        match on_input(&mut m, &mut u, &chord("Enter")) {
            Outcome::Close(Some(Cmd::FileOp(FileOpCmd::Delete { paths, mode: DeleteMode::Permanent }))) => {
                assert_eq!(paths.len(), 2)
            }
            other => panic!("{other:?}"),
        }
        let mut m = model(DeleteMode::Trash);
        let mut u = ui(&m);
        assert!(matches!(on_input(&mut m, &mut u, &chord("Esc")), Outcome::Close(None)));
        let DialogModel::DeleteConfirm(d) = &mut m else { panic!() };
        assert!(d.confirm().is_none(), "cancelled");
        let mut m = model(DeleteMode::Trash);
        let mut u = ui(&m);
        assert!(matches!(on_input(&mut m, &mut u, &chord("Left")), Outcome::Keep));
        assert!(matches!(on_input(&mut m, &mut u, &chord("Space")), Outcome::Close(None)), "Cancel pressed");
        let mut m: DialogModel = DeleteConfirmModel::empty_trash(4).into();
        let mut u = ui(&m);
        // "Empty trash": E.
        assert!(matches!(
            on_input(&mut m, &mut u, &chord("E")),
            Outcome::Close(Some(Cmd::FileOp(FileOpCmd::EmptyTrash)))
        ));
    }
}
