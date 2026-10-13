//! The dialog for the two-way sync conflicts of the compare screen
//! (`TuiDialog::CompareConflict`), the GUI's conflict step one conflict at a time: the
//! item, why it conflicts and both sides, then Keep left / Keep right / Keep both / Skip
//! and Keep newer (every conflict: the side modified last wins). With "Apply to all"
//! checked a choice answers every conflict at once. Each answer goes to the shared
//! `CompareModel` (`resolve` / `resolve_all`) and the next open conflict shows; the dialog
//! closes after the last one. Esc leaves the rest undecided (they are skipped).
//!
//! Keys: the buttons' letters (L, R, B, S, N), Left / Right / Tab between the buttons,
//! Enter / Space press, Space on the check box toggles it (Alt+A anywhere).

use std::rc::Rc;

use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::compare::{RESOLUTIONS, resolution_allowed};
use soyuz_appcore::models::compare_state::Resolution;
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::input::compare::Shared;
use crate::widgets::{Button, Field, Form, FormOutcome};

/// Button index of "Keep newer" (after the four resolutions).
pub const KEEP_NEWER: usize = 4;

/// What the dialog keeps while it is open.
#[derive(Default)]
pub struct State {
    /// The compare screen's model (`None` only in a default state).
    pub model: Option<Shared>,
    /// The conflict rows to answer (indexes into the model's rows), in order.
    pub rows: Vec<usize>,
    /// The one shown.
    pub pos: usize,
    /// Field 0: "Apply to all"; buttons: [`RESOLUTIONS`], then Keep newer.
    pub form: Form,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State").field("rows", &self.rows).field("pos", &self.pos).finish_non_exhaustive()
    }
}

impl State {
    /// The model row shown.
    pub fn row(&self) -> Option<usize> {
        self.rows.get(self.pos).copied()
    }

    /// "Apply to all" is checked.
    pub fn all(&self) -> bool {
        self.form.fields.first().and_then(Field::on).unwrap_or(false)
    }

    /// Enable "Keep both" for the shown row (it needs the item on both sides).
    fn refresh(&mut self) {
        let ok = match (&self.model, self.row()) {
            (Some(m), Some(i)) => {
                self.all() || m.borrow().rows().get(i).is_some_and(|r| resolution_allowed(r, Resolution::KeepBoth))
            }
            _ => false,
        };
        let kb = RESOLUTIONS.iter().position(|r| *r == Resolution::KeepBoth).unwrap_or(2);
        if let Some(b) = self.form.buttons.get_mut(kb) {
            b.enabled = ok;
        }
        let focus_ok = self.form.focused_button().is_none_or(|b| self.form.buttons.get(b).is_some_and(|b| b.enabled));
        if !focus_ok {
            self.form.focus_button(0);
        }
    }
}

fn accel(r: Resolution) -> char {
    match r {
        Resolution::Left => 'l',
        Resolution::Right => 'r',
        Resolution::KeepBoth => 'b',
        Resolution::Skip => 's',
    }
}

/// The dialog's state for the open conflicts of `model` (every conflict when all are
/// decided); `None` when it has none.
pub fn state(model: Shared) -> Option<State> {
    let (rows, open) = {
        let m = model.borrow();
        let all = m.conflict_rows();
        let open: Vec<usize> = all.iter().copied().filter(|&i| m.rows()[i].open_conflict()).collect();
        (all, open)
    };
    let rows = if open.is_empty() { rows } else { open };
    if rows.is_empty() {
        return None;
    }
    let mut buttons: Vec<Button> = RESOLUTIONS.iter().map(|r| Button::new(r.label()).accel(accel(*r))).collect();
    buttons.push(Button::new(tr!("tools-compare-keep-newer")).accel('n'));
    let form = Form::new(vec![Field::check(tr!("tools-compare-apply-all"), false)], buttons);
    let mut s = State { model: Some(model), rows, pos: 0, form };
    s.form.focus_button(0);
    s.refresh();
    Some(s)
}

/// Open the dialog on `model`'s conflicts (the compare screen's C / Conflicts).
pub fn open(app: &mut App, model: Shared) {
    if let Some(s) = state(model) {
        app.push_dialog(TuiDialog::CompareConflict(s));
    }
}

/// Answer with button `b`; returns whether the dialog is done.
fn answer(s: &mut State, b: usize) -> bool {
    let Some(model) = s.model.clone() else { return true };
    let all = s.all();
    if b == KEEP_NEWER {
        model.borrow_mut().resolve_all(None);
        return true;
    }
    let Some(res) = RESOLUTIONS.get(b).copied() else { return false };
    if all {
        model.borrow_mut().resolve_all(Some(res));
        return true;
    }
    if let Some(i) = s.row() {
        model.borrow_mut().resolve(i, res);
    }
    s.pos += 1;
    s.refresh();
    s.pos >= s.rows.len()
}

fn on_form(s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(b) => answer(s, b).then_some(Outcome::Close(None)),
        FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(_) => {
            s.refresh();
            None
        }
        FormOutcome::Consumed | FormOutcome::Ignored => None,
    }
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        // Alt+A toggles "Apply to all" wherever the focus is.
        if let InputEvent::Chord(c) = ev
            && c.key == KeyName::A
            && c.mods.contains(Mods::ALT)
            && let Some(f) = s.form.fields.first_mut()
            && let Some(on) = f.on()
        {
            *f = Field::check(f.label.clone(), !on);
            s.refresh();
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = on_form(s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    let o = super::confirm::click(&mut s.form, m);
    o.and_then(|o| on_form(s, o)).unwrap_or(Outcome::Keep)
}

/// The dialog went away by itself: the compare screen closed or compared again.
pub fn stale(_core: &AppCore, s: &State) -> bool {
    let Some(m) = &s.model else { return true };
    if Rc::strong_count(m) < 2 {
        return true;
    }
    let m = m.borrow();
    s.row().is_none_or(|i| m.rows().get(i).is_none_or(|r| r.conflict.is_none()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use soyuz_appcore::models::compare::CompareModel;

    use super::*;

    #[test]
    fn a_model_without_conflicts_opens_nothing() {
        let m = CompareModel::new_in(
            std::env::temp_dir(),
            std::path::PathBuf::from("a"),
            None,
            Vec::new(),
            soyuz_appcore::host::no_wake(),
        );
        assert!(state(Rc::new(RefCell::new(m))).is_none());
    }
}
