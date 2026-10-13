//! The dialog for naming a recipe (`TuiDialog::RecipeName`): "Save as recipe" on a
//! finished assistant task (the GUI's inline name field). Save writes the task's executed
//! steps as a TOML recipe in the background (`soyuz_appcore::ai::state::save_recipe`; the
//! task end then says where it went); an empty name is refused. The dialog closes by
//! itself when the task's end is gone (new chat).

use soyuz_appcore::ai::TaskId;
use soyuz_appcore::ai::state::{self, ChatItem};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Field, Form, FormOutcome};

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// The task whose steps are saved.
    pub task: u64,
    /// Field: the name; buttons: save, cancel.
    pub form: Form,
}

/// Open the dialog for the end of `task`.
pub fn open(app: &mut App, task: u64) {
    app.push_dialog(TuiDialog::RecipeName(state(task)));
}

/// A new dialog's state.
pub fn state(task: u64) -> State {
    let form = Form::new(
        vec![Field::text(tr!("ai-recipe-name-hint"), "")],
        vec![Button::new(tr!("common-save")), Button::new(tr!("common-cancel"))],
    )
    .default_button(0);
    State { task, form }
}

fn submit(core: &mut AppCore, s: &mut State) -> Outcome {
    let name = s.form.fields.first().and_then(|f| f.value()).unwrap_or_default().trim().to_string();
    if name.is_empty() {
        s.form.error = Some(tr!("tui-ai-recipe-name-empty").into());
        return Outcome::Keep;
    }
    state::save_recipe(core, TaskId(s.task), name);
    for c in &mut core.ai.chat {
        if let ChatItem::TaskEnd(t) = c
            && t.task.0 == s.task
        {
            t.recipe_name = None;
            t.saved = Some(tr!("ai-saving").into());
        }
    }
    Outcome::Close(None)
}

fn outcome(core: &mut AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => Some(submit(core, s)),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(_) => {
            s.form.error = None;
            None
        }
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        let o = s.form.apply(ev);
        if let Some(out) = outcome(core, s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    match super::confirm::click(&mut s.form, m) {
        Some(o) => outcome(core, s, o).unwrap_or(Outcome::Keep),
        None => Outcome::Keep,
    }
}

/// The dialog went away by itself: the task's end is gone (new chat).
pub fn stale(core: &AppCore, s: &State) -> bool {
    !core.ai.chat.iter().any(|c| matches!(c, ChatItem::TaskEnd(t) if t.task.0 == s.task))
}
