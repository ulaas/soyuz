//! Select by pattern (`FrontendCmd::SelectByPattern`): a pattern field (glob or plain
//! text, starting as `*.`), "Add to the current selection", Select and Cancel. Select
//! applies `soyuz_appcore::models::requests::pattern::apply` to the pane and toasts the
//! count, like the GUI.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::pattern;
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Field, Form, FormOutcome};

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    /// Fields: the pattern, "add to the selection"; buttons: Select, Cancel.
    pub form: Form,
    /// The pane to select in (`None`: the focused pane).
    pub pane: Option<PaneId>,
}

/// The form of a new dialog.
pub fn form() -> Form {
    Form::new(
        vec![
            Field::text(tr!("tui-requests-pattern-field"), pattern::DEFAULT_PATTERN),
            Field::check(tr!("dialogs-pattern-add"), false),
        ],
        vec![Button::new(tr!("dialogs-pattern-select")), Button::new(tr!("common-cancel"))],
    )
}

/// Open the dialog (from `crate::frontend`).
pub fn open(app: &mut App, pane: PaneId) {
    app.push_dialog(TuiDialog::Pattern(State {
        title: tr!("dialogs-pattern-title").into(),
        form: form(),
        pane: Some(pane),
    }));
}

fn outcome(core: &mut AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => {
            let text = s.form.fields.first().and_then(|f| f.value()).unwrap_or_default().to_string();
            let add = s.form.fields.get(1).and_then(|f| f.on()).unwrap_or(false);
            let pane = s.pane.unwrap_or_else(|| core.ws.pane().id);
            Some(Outcome::Close(pattern::apply(&mut core.ws, pane, &text, add)))
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
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
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    match super::confirm::click(&mut s.form, e) {
        Some(o) => outcome(core, s, o).unwrap_or(Outcome::Keep),
        None => Outcome::Keep,
    }
}

/// The dialog went away by itself: its pane closed.
pub fn stale(core: &AppCore, s: &State) -> bool {
    s.pane.is_some_and(|p| core.ws.find_pane(p).is_none())
}
