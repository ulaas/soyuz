//! "Save changes?" for an editor document or before quitting
//! (`FrontendCmd::EditorUnsaved`, the GUI's `ui::editors::UnsavedDialog`) — Phase T4.
//!
//! Three buttons: Save (Save all when quitting; the default, Enter), Don't save and
//! Cancel. Esc, closing the dialog any other way, or the document going away answer
//! Cancel ([`State`]'s `Drop`), so the core never waits for an answer (`editors::pump`
//! applies it next frame: closing saves first or discards; quitting saves every document
//! and then quits, or quits right away, or stays).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::editors::UnsavedPrompt;
use soyuz_appcore::editors::io::Answer;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::layout::HitTarget;
use crate::widgets::{Button, Form, FormOutcome};

/// The buttons, in order.
pub const SAVE: usize = 0;
pub const DISCARD: usize = 1;
pub const CANCEL: usize = 2;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub body: String,
    pub form: Form,
    prompt: Option<UnsavedPrompt>,
    answered: bool,
}

impl State {
    /// The dialog for `p`.
    pub fn new(p: UnsavedPrompt) -> State {
        let save = if p.target.is_some() { tr!("common-save") } else { tr!("editors-unsaved-save-all") };
        let form = Form::new(
            Vec::new(),
            vec![Button::new(save), Button::new(tr!("editors-unsaved-dont-save")), Button::new(tr!("common-cancel"))],
        )
        .default_button(SAVE);
        State {
            title: crate::dialog::safe(&p.title),
            body: crate::dialog::safe(&p.body),
            form,
            prompt: Some(p),
            answered: false,
        }
    }

    /// Answer once.
    fn answer(&mut self, a: Answer) {
        if !self.answered {
            self.answered = true;
            if let Some(p) = &self.prompt {
                p.answer(a);
            }
        }
    }

    /// The document it asks about (`None`: quitting).
    pub fn target(&self) -> Option<u64> {
        self.prompt.as_ref().and_then(|p| p.target)
    }
}

impl Drop for State {
    /// Dismissed without an answer: Cancel (the prompt's contract).
    fn drop(&mut self) {
        self.answer(Answer::Cancel);
    }
}

/// Ask `p`.
pub fn open(app: &mut App, p: UnsavedPrompt) {
    app.push_dialog(TuiDialog::Unsaved(State::new(p)));
}

/// What pressing button `b` does.
fn press(s: &mut State, b: usize) -> Outcome {
    s.answer(match b {
        SAVE => Answer::Save,
        DISCARD => Answer::Discard,
        _ => Answer::Cancel,
    });
    Outcome::Close(None)
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        match s.form.apply(ev) {
            FormOutcome::Pressed(b) => return press(s, b),
            FormOutcome::Cancel => return press(s, CANCEL),
            _ => {}
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = e.event.kind else {
        return Outcome::Keep;
    };
    match e.target {
        Some(HitTarget::DialogButton(b)) => match s.form.click_button(b) {
            FormOutcome::Pressed(b) => press(s, b),
            _ => Outcome::Keep,
        },
        _ => Outcome::Keep,
    }
}

/// The dialog went away by itself: the document it asks about was closed.
pub fn stale(core: &AppCore, s: &State) -> bool {
    s.target().is_some_and(|id| !core.editors.docs.iter().any(|d| d.id == id))
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::editors::io::Msg;

    use super::*;

    #[test]
    fn answers_once_and_cancels_when_dismissed() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut s = State::new(UnsavedPrompt::close("a.txt", 7, tx.clone()));
        assert_eq!(s.target(), Some(7));
        assert_eq!(s.form.buttons.len(), 3);
        assert_eq!(s.form.focused_button(), Some(SAVE), "Enter saves");
        assert!(matches!(press(&mut s, DISCARD), Outcome::Close(None)));
        drop(s);
        assert!(matches!(rx.try_recv(), Ok(Msg::Close { doc: 7, answer: Answer::Discard })));
        assert!(rx.try_recv().is_err(), "answered once");
        // Dismissed without an answer: Cancel.
        drop(State::new(UnsavedPrompt::quit(&["a".into(), "b".into()], tx)));
        assert!(matches!(rx.try_recv(), Ok(Msg::Quit(Answer::Cancel))));
    }
}
