//! Interrupted jobs from the journal (`FrontendCmd::RecoveredJobs`): the operations that
//! were running when Soyuz stopped, how far each got, and Resume / Roll back / Discard /
//! Decide later (`soyuz_appcore::models::requests::recovery`, carried out by
//! `services::crash::pump`). Leftover temp files are looked for on a worker thread; the
//! checkbox moves them to the trash with the choice. Esc decides later: the journal stays
//! for the next start.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::recovery::RecoveryModel;
use soyuz_appcore::tr;
use soyuz_jobs::journal::Recovered;

use super::confirm::click;
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Field, Form, FormOutcome, SelectList};

/// Buttons, in order.
pub const RESUME: usize = 0;
pub const ROLL_BACK: usize = 1;
pub const DISCARD: usize = 2;
pub const LATER: usize = 3;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    /// Fields: clean up the leftovers; buttons: Resume, Roll back, Discard, Decide later.
    pub form: Form,
    /// The jobs and the leftover scan (`None` only in a default state).
    pub model: Option<RecoveryModel>,
    /// The cursor in the list of jobs.
    pub list: SelectList,
}

/// Open the dialog (from `crate::frontend`).
pub fn open(app: &mut App, recovered: Vec<Recovered>) {
    let model = RecoveryModel::new(recovered, app.core.wake.clone());
    let mut form = Form::new(
        vec![Field::check(tr!("dialogs-recovery-scanning"), model.clean)],
        vec![
            Button::new(tr!("dialogs-recovery-resume")).accel('r'),
            Button::new(tr!("dialogs-recovery-rollback")).accel('b'),
            Button::new(tr!("dialogs-recovery-discard")).accel('d'),
            Button::new(tr!("dialogs-recovery-later")).accel('l'),
        ],
    );
    form.buttons[ROLL_BACK].enabled = model.can_roll_back();
    form.focus_button(RESUME);
    app.push_dialog(TuiDialog::Recovery(State {
        title: model.title.clone(),
        form,
        model: Some(model),
        list: SelectList::new(),
    }));
}

fn outcome(s: &mut State, o: FormOutcome) -> Option<Outcome> {
    let m = s.model.as_mut()?;
    // The checkbox is the model's `clean`.
    if let Some(on) = s.form.fields.first().and_then(|f| f.on()) {
        m.clean = on;
    }
    match o {
        FormOutcome::Pressed(RESUME) => {
            m.resume();
            Some(Outcome::Close(None))
        }
        FormOutcome::Pressed(ROLL_BACK) => m.roll_back().then_some(Outcome::Close(None)),
        FormOutcome::Pressed(DISCARD) => {
            m.discard();
            Some(Outcome::Close(None))
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let len = s.model.as_ref().map_or(0, |m| m.jobs.len());
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        if s.list.apply(ev, len, height) {
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = outcome(s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    let len = s.model.as_ref().map_or(0, |m| m.jobs.len());
    if super::confirm::list_mouse(&mut s.list, len, e).is_some() {
        return Outcome::Keep;
    }
    match click(&mut s.form, e) {
        Some(o) => outcome(s, o).unwrap_or(Outcome::Keep),
        None => Outcome::Keep,
    }
}

/// The dialog went away by itself (its pane or files are gone, …).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
