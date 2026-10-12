//! "Save changes?" for an editor document or before quitting
//! (`FrontendCmd::EditorUnsaved`) — Phase T4 (with the editor).
//!
//! Stub until the editor view exists: there is no way to change a document in the
//! terminal UI yet, so [`open`] answers by itself. Closing one document keeps it open
//! (`Cancel`, with the "not available" toast). Quitting saves every document
//! (`editors::pump`, called every frame, then closes the app once they are saved); when
//! saving failed and the user quits again, a warning says that the next quit discards the
//! changes, and the third quit does (`State::unsaved_quits` in `super`). A quit therefore
//! never hangs on a dirty document.
//!
//! Nothing is pushed until the dialog exists (see the shape of a kind file in `super`).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::editors::UnsavedPrompt;
use soyuz_appcore::editors::io::Answer;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome};
use crate::app::App;
use crate::widgets::Form;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub form: Form,
}

/// Answer (or, with the editor view, ask) `p`.
pub fn open(app: &mut App, p: UnsavedPrompt) {
    if p.target.is_some() {
        p.answer(Answer::Cancel);
        app.unavailable(tr!("tui-feature-editor"));
        return;
    }
    let tries = &mut app.input.dialogs.unsaved_quits;
    *tries = tries.saturating_add(1);
    match *tries {
        1 => p.answer(Answer::Save),
        2 => {
            p.answer(Answer::Cancel);
            app.core.ui.toasts.push(Toast::warning(tr!("tui-unsaved-quit-again")).body(p.body.clone()).sticky());
        }
        _ => p.answer(Answer::Discard),
    }
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    super::form_input(&mut s.form, i)
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, _s: &mut State, _e: &DialogMouse) -> Outcome {
    Outcome::Keep
}

/// The dialog went away by itself.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
