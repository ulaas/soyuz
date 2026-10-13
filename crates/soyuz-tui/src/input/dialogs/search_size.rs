//! The dialog for the search bar's size filter (`TuiDialog::SearchSize`): the size pill
//! (the GUI's size popup). Minimum and maximum (`10M`, `16k`, …; empty: no limit) and the
//! GUI's presets (tiny, large, huge, any) as a radio row that fills both fields. Apply
//! checks the sizes, stores them in the form and re-runs the search on screen.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::search as model;
use soyuz_appcore::services::search as svc;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Field, FieldKind, Form, FormOutcome, LineEdit};

/// Field indices.
const MIN: usize = 0;
const MAX: usize = 1;
const PRESET: usize = 2;

/// The presets after "Custom" (radio option `i + 1`): (min, max).
const PRESETS: [(&str, &str); 4] = [("", "16k"), ("100M", ""), ("1G", ""), ("", "")];

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// Fields: min, max, preset; buttons: apply, cancel.
    pub form: Form,
}

/// Open the dialog on the bar's size filter.
pub fn open(app: &mut App) {
    let f = &app.core.ui.search.form;
    let s = state(&f.min_size, &f.max_size);
    app.push_dialog(TuiDialog::SearchSize(s));
}

/// A new dialog's state for `min` and `max`.
pub fn state(min: &str, max: &str) -> State {
    let (min, max) = (min.trim(), max.trim());
    let preset = PRESETS.iter().position(|(a, b)| *a == min && *b == max).map_or(0, |i| i + 1);
    let options = vec![
        tr!("tui-search-size-custom").to_string(),
        tr!("tui-search-size-preset-tiny").to_string(),
        tr!("tui-search-size-preset-large").to_string(),
        tr!("tui-search-size-preset-huge").to_string(),
        tr!("views-search-size-any").to_string(),
    ];
    let form = Form::new(
        vec![
            Field::text(tr!("views-search-size-min"), min),
            Field::text(tr!("views-search-size-max"), max),
            Field::radio(tr!("tui-search-size-preset"), options, preset),
        ],
        vec![Button::new(tr!("common-apply")), Button::new(tr!("common-cancel"))],
    )
    .default_button(0);
    State { form }
}

fn text(s: &State, i: usize) -> String {
    s.form.fields.get(i).and_then(|f| f.value()).unwrap_or_default().trim().to_string()
}

fn set_text(s: &mut State, i: usize, v: &str) {
    if let Some(FieldKind::Text { text, edit, .. }) = s.form.fields.get_mut(i).map(|f| &mut f.kind) {
        *text = v.to_string();
        *edit = LineEdit::at_end(v);
    }
}

/// The checked sizes, or the message for the first bad one.
fn validate(s: &State) -> Result<(String, String), String> {
    let (min, max) = (text(s, MIN), text(s, MAX));
    for v in [&min, &max] {
        if !v.is_empty() && svc::parse_size(v).is_none() {
            return Err(tr!("tui-search-size-invalid", value = v.as_str()));
        }
    }
    Ok((min, max))
}

fn changed(s: &mut State, i: usize) {
    s.form.error = None;
    match i {
        PRESET => {
            let p = s.form.fields[PRESET].selected().unwrap_or(0);
            if let Some((min, max)) = p.checked_sub(1).and_then(|k| PRESETS.get(k)) {
                set_text(s, MIN, min);
                set_text(s, MAX, max);
            }
        }
        _ => {
            if let FieldKind::Radio { selected, .. } = &mut s.form.fields[PRESET].kind {
                *selected = 0;
            }
        }
    }
}

fn outcome(core: &mut AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => match validate(s) {
            Ok((min, max)) => {
                let form = &mut core.ui.search.form;
                let differs = form.min_size.trim() != min || form.max_size.trim() != max;
                form.min_size = min;
                form.max_size = max;
                model::submit(core, false, false, differs);
                Some(Outcome::Close(None))
            }
            Err(e) => {
                s.form.error = Some(e);
                Some(Outcome::Keep)
            }
        },
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(i) => {
            changed(s, i);
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

/// The dialog went away by itself: the bar was closed.
pub fn stale(core: &AppCore, _s: &State) -> bool {
    !core.ui.search.open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_fill_the_fields_and_typing_makes_it_custom() {
        let mut s = state("", "16k");
        assert_eq!(s.form.fields[PRESET].selected(), Some(1));
        if let FieldKind::Radio { selected, .. } = &mut s.form.fields[PRESET].kind {
            *selected = 3;
        }
        changed(&mut s, PRESET);
        assert_eq!((text(&s, MIN), text(&s, MAX)), ("1G".to_string(), String::new()));
        changed(&mut s, MIN);
        assert_eq!(s.form.fields[PRESET].selected(), Some(0));
        assert_eq!(state("5M", "9M").form.fields[PRESET].selected(), Some(0));
    }

    #[test]
    fn bad_sizes_are_refused() {
        assert!(validate(&state("10M", "")).is_ok());
        assert!(validate(&state("lots", "")).is_err());
        assert!(validate(&state("", "1Q")).is_err());
    }
}
