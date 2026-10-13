//! The "Save workspace as…" dialog (`TuiDialog::SaveWorkspace`, Phase T7, agent A): a name
//! field prefilled with the core's suggestion; Save queues `WsCmd::Save` (the GUI's
//! `ui/workspaces/dialogs.rs` `SaveWorkspaceDialog`). An existing name is overwritten (the
//! toast says "updated"); a blank name is refused with a message.

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::workspaces::WsCmd;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Field, Form, FormOutcome};

/// Buttons, in order.
pub const SAVE: usize = 0;
pub const CANCEL: usize = 1;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// Field: the name; buttons: save, cancel.
    pub form: Form,
}

/// Open the dialog for `FrontendCmd::SaveWorkspace` with the suggested `name`.
pub fn open(app: &mut App, name: String) {
    app.push_dialog(TuiDialog::SaveWorkspace(state(&name)));
}

/// A new dialog's state with `name` in the field.
pub fn state(name: &str) -> State {
    let form = Form::new(
        vec![Field::text(tr!("tui-workspaces-name"), name)],
        vec![Button::new(tr!("common-save")), Button::new(tr!("common-cancel"))],
    )
    .default_button(SAVE);
    State { form }
}

/// The name typed (trimmed).
pub fn name(s: &State) -> String {
    s.form.fields.first().and_then(|f| f.value()).unwrap_or_default().trim().to_string()
}

fn outcome(s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(SAVE) => {
            let name = name(s);
            if name.is_empty() {
                s.form.error = Some(tr!("tui-workspaces-name-empty").into());
                return Some(Outcome::Keep);
            }
            Some(Outcome::Close(Some(Cmd::Workspace(WsCmd::Save(name)))))
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(_) => {
            s.form.error = None;
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
    let o = super::prompt::click(&mut s.form, m);
    outcome(s, o).unwrap_or(Outcome::Keep)
}

/// The dialog went away by itself: never.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::keys::input::InputEvent;

    use super::*;

    fn press(s: &mut State, chord: &str) -> Option<Outcome> {
        let o = s.form.apply(&InputEvent::Chord(chord.parse().unwrap()));
        outcome(s, o)
    }

    #[test]
    fn enter_saves_the_trimmed_name_and_blank_names_are_refused() {
        let mut s = state("  Photos 2 ");
        match press(&mut s, "Enter") {
            Some(Outcome::Close(Some(Cmd::Workspace(WsCmd::Save(n))))) => assert_eq!(n, "Photos 2"),
            other => panic!("{other:?}"),
        }
        let mut s = state("   ");
        assert!(matches!(press(&mut s, "Enter"), Some(Outcome::Keep)));
        assert!(s.form.error.is_some());
        // Typing clears the message.
        let o = s.form.apply(&InputEvent::Text("x".into()));
        assert!(outcome(&mut s, o).is_none());
        assert!(s.form.error.is_none());
        assert!(matches!(press(&mut s, "Esc"), Some(Outcome::Close(None))));
    }
}
