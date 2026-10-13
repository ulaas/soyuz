//! The smart folder editor (`FrontendCmd::Smart(SmartPrompt::Edit)`, `TuiDialog::Smart`):
//! the name, the query and "include hidden" fields, Save and Cancel, like the GUI's
//! `ui::smart::SmartDialog`. The view shows how the query will be answered
//! (`models::smart::coverage_text`) and the query cheat sheet (`query_help`); the
//! validation message is `models::smart::validate` (no name, no query, a query that does
//! not parse, a name another smart folder has). Save sends `SmartCmd::Save`, which stores
//! `smart_folders.toml` and opens the folder.
//!
//! The delete confirmation (`SmartPrompt::ConfirmDelete`) is the shared confirm dialog
//! (`confirm::open`), whose Delete sends `SmartCmd::ConfirmDelete`.

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::smart::{SmartCmd, SmartFolder, SmartPrompt, validate};
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Field, Form, FormOutcome};

/// Field indices of the form.
pub const NAME: usize = 0;
pub const QUERY: usize = 1;
pub const HIDDEN: usize = 2;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// The smart folder edited (`None`: a new one).
    pub index: Option<usize>,
    /// Fields: name, query, include hidden; buttons: Save, Cancel.
    pub form: Form,
    /// The user changed something (the validation message shows from then on, like the
    /// GUI's, which stays quiet while the name is empty).
    pub touched: bool,
}

/// Open the dialog for the core's request (`crate::frontend`).
pub fn open(app: &mut App, prompt: SmartPrompt) {
    match prompt {
        SmartPrompt::Edit { index, folder } => app.push_dialog(TuiDialog::Smart(state(index, &folder))),
        SmartPrompt::ConfirmDelete { index, name } => {
            let message =
                format!("{} {}", tr!("views-smart-delete-confirm", name = &name), tr!("views-smart-delete-body"));
            super::confirm::open(
                app,
                tr!("views-smart-delete-title").to_string(),
                message,
                tr!("common-delete").to_string(),
                Cmd::Smart(SmartCmd::ConfirmDelete(index)),
            );
        }
    }
}

/// A new dialog's state for `folder` (at `index`, or new).
pub fn state(index: Option<usize>, folder: &SmartFolder) -> State {
    let form = Form::new(
        vec![
            Field::text(tr!("views-smart-name"), folder.name.as_str()),
            Field::text(tr!("views-smart-query"), folder.query.as_str()),
            Field::check(tr!("views-smart-include-hidden"), folder.include_hidden),
        ],
        vec![Button::new(tr!("common-save")), Button::new(tr!("common-cancel"))],
    )
    .default_button(0);
    State { index, form, touched: false }
}

/// The smart folder as typed.
pub fn folder(s: &State) -> SmartFolder {
    let text = |i: usize| s.form.fields.get(i).and_then(|f| f.value()).unwrap_or_default().to_string();
    SmartFolder {
        name: text(NAME),
        query: text(QUERY),
        include_hidden: s.form.fields.get(HIDDEN).and_then(|f| f.on()).unwrap_or(false),
    }
}

/// Why the folder cannot be saved, once the user typed something.
pub fn problem(s: &State) -> Option<String> {
    let f = folder(s);
    let quiet = !s.touched && f.name.trim().is_empty() && f.query.trim().is_empty();
    if quiet { None } else { validate(&f, s.index) }
}

fn outcome(s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => {
            let f = folder(s);
            match validate(&f, s.index) {
                Some(e) => {
                    s.touched = true;
                    s.form.error = Some(e);
                    Some(Outcome::Keep)
                }
                None => Some(Outcome::Close(Some(Cmd::Smart(SmartCmd::Save { index: s.index, folder: f })))),
            }
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(_) => {
            s.touched = true;
            s.form.error = problem(s);
            None
        }
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
pub fn on_mouse(_core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    match super::confirm::click(&mut s.form, m).and_then(|o| outcome(s, o)) {
        Some(out) => out,
        None => Outcome::Keep,
    }
}

/// The dialog went away by itself: never (the store only changes through it).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::keys::input::InputEvent;

    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    #[test]
    fn save_validates_then_sends_the_folder() {
        let mut s = state(None, &SmartFolder::default());
        assert_eq!(problem(&s), None, "quiet until something is typed");
        // Enter in the empty name field: Save refuses with the reason.
        assert!(matches!(outcome(&mut s, FormOutcome::Pressed(0)), Some(Outcome::Keep)));
        assert_eq!(s.form.error.as_deref(), Some(tr!("views-smart-need-name")));
        s.form.apply(&InputEvent::Text("Pics".into()));
        let o = s.form.apply(&chord("Tab"));
        assert_eq!(o, FormOutcome::Consumed);
        let o = s.form.apply(&InputEvent::Text("kind:image".into()));
        assert!(outcome(&mut s, o).is_none());
        assert_eq!(s.form.error, None, "valid now");
        let o = s.form.apply(&chord("Enter"));
        match outcome(&mut s, o) {
            Some(Outcome::Close(Some(Cmd::Smart(SmartCmd::Save { index: None, folder })))) => {
                assert_eq!((folder.name.as_str(), folder.query.as_str()), ("Pics", "kind:image"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_bad_query_is_reported_and_cancel_closes() {
        let f = SmartFolder { name: "Big".into(), query: "size:>1GB".into(), include_hidden: true };
        let mut s = state(Some(3), &f);
        assert_eq!(folder(&s), f);
        s.form.focus = QUERY;
        let o = s.form.apply(&InputEvent::Text(" (".into()));
        outcome(&mut s, o);
        assert!(s.form.error.is_some(), "unbalanced parenthesis");
        let o = s.form.apply(&chord("Escape"));
        assert!(matches!(outcome(&mut s, o), Some(Outcome::Close(None))));
    }
}
