//! The dialog for the name of a saved search (`TuiDialog::SaveSearch`): Ctrl+S or the
//! bar's Save button (the GUI's "save this search" popup). The name starts as the form's
//! title (`models::search::begin_save`); Save stores the form under it in `searches.toml`
//! (a search of the same name is replaced) and the sidebar lists it; an empty name is
//! refused.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::search as model;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::form::FieldKind;
use crate::widgets::{Button, Field, Form, FormOutcome};

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// Field: the name; buttons: save, cancel.
    pub form: Form,
}

/// Open the dialog on the search form of the bar.
pub fn open(app: &mut App) {
    model::begin_save(&mut app.core);
    let name = app.core.ui.search.saving.clone().unwrap_or_default();
    app.push_dialog(TuiDialog::SaveSearch(state(&name)));
}

/// A new dialog's state with `name` in the field.
pub fn state(name: &str) -> State {
    let form = Form::new(
        vec![Field::text(tr!("tui-search-save-name"), name)],
        vec![Button::new(tr!("common-save")), Button::new(tr!("common-cancel"))],
    )
    .default_button(0);
    State { form }
}

fn name(s: &State) -> String {
    match s.form.fields.first().map(|f| &f.kind) {
        Some(FieldKind::Text { text, .. }) => text.trim().to_string(),
        _ => String::new(),
    }
}

fn outcome(core: &mut AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => {
            let name = name(s);
            if name.is_empty() {
                s.form.error = Some(tr!("tui-search-save-name-empty").into());
                return Some(Outcome::Keep);
            }
            core.ui.search.saving = Some(name);
            model::finish_save(core, true);
            Some(Outcome::Close(None))
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => {
            model::finish_save(core, false);
            Some(Outcome::Close(None))
        }
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

/// The dialog went away by itself: the save prompt was closed elsewhere.
pub fn stale(core: &AppCore, _s: &State) -> bool {
    core.ui.search.saving.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_starts_as_given_and_is_trimmed() {
        let mut s = state("  report  ");
        assert_eq!(name(&s), "report");
        if let FieldKind::Text { text, .. } = &mut s.form.fields[0].kind {
            text.clear();
        }
        assert_eq!(name(&s), "");
    }
}
