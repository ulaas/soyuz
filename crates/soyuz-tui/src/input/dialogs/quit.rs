//! Quit with running jobs (`DialogModel::Quit`), like the GUI's `ui/dialogs/quit.rs`:
//! "Keep open", "Quit when finished" (the safe default, focused: Enter) and "Cancel jobs
//! and quit". Esc keeps Soyuz open. Letters press the buttons only without Ctrl / Alt,
//! so a second Ctrl+Q while the question is open never cancels the running jobs.

use soyuz_appcore::models::dialogs::{DialogModel, QuitChoice};
use soyuz_appcore::tr;

use super::prompt::{buttons, click};
use super::{DialogInput, DialogMouse, ModelUi, Outcome};
use crate::widgets::{Form, FormOutcome};

/// The buttons' choices, left to right.
const CHOICES: [QuitChoice; 3] = [QuitChoice::KeepOpen, QuitChoice::QuitWhenFinished, QuitChoice::CancelAndQuit];

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(_m: &DialogModel) -> ModelUi {
    let labels = [tr!("dialogs-quit-keep-open"), tr!("dialogs-quit-when-finished"), tr!("dialogs-quit-cancel-jobs")];
    let mut form = Form::new(Vec::new(), buttons(&labels)).default_button(1);
    form.focus_button(1);
    ModelUi { form, ..ModelUi::default() }
}

fn on_form(m: &mut DialogModel, out: FormOutcome) -> Option<Outcome> {
    let DialogModel::Quit(q) = m else { return None };
    let choice = match out {
        FormOutcome::Pressed(b) => *CHOICES.get(b)?,
        FormOutcome::Cancel => QuitChoice::KeepOpen,
        _ => return None,
    };
    Some(Outcome::Close(q.choose(choice)))
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
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use soyuz_appcore::cmd::Cmd;
    use soyuz_appcore::keys::input::InputEvent;
    use soyuz_appcore::models::dialogs::QuitModel;

    use super::*;
    use crate::layout::HitTarget;

    fn chord(s: &str) -> DialogInput {
        DialogInput {
            key: Some(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)),
            events: vec![InputEvent::Chord(s.parse().unwrap())],
        }
    }

    fn model() -> DialogModel {
        QuitModel::new(2, Arc::new(AtomicBool::new(true))).into()
    }

    #[test]
    fn choices() {
        let mut m = model();
        let mut u = ui(&m);
        assert!(matches!(on_input(&mut m, &mut u, &chord("Ctrl+Q")), Outcome::Keep), "Ctrl+Q again does nothing");
        assert!(matches!(
            on_input(&mut m, &mut u, &chord("Enter")),
            Outcome::Close(Some(Cmd::Quit { when_idle: true }))
        ));
        assert!(matches!(on_input(&mut m, &mut u, &chord("Esc")), Outcome::Close(None)));
        assert!(matches!(on_input(&mut m, &mut u, &chord("K")), Outcome::Close(None)));
        assert!(matches!(on_input(&mut m, &mut u, &chord("C")), Outcome::Close(Some(Cmd::Quit { when_idle: false }))));
        let click = DialogMouse {
            event: MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            target: Some(HitTarget::DialogButton(2)),
        };
        assert!(matches!(on_mouse(&mut m, &mut u, &click), Outcome::Close(Some(Cmd::Quit { when_idle: false }))));
    }
}
