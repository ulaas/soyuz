//! Script approval cards (`DialogModel::ScriptApproval`, `[scripting] approve_writes`),
//! like the GUI's `ui/automation/approval.rs`: the changes a script action planned as a
//! list (Up / Down / PageUp / PageDown / Home / End and the wheel scroll it), then Cancel
//! and Run (the default, focused). Run submits them as normal jobs
//! (`ScriptApprovalModel::run`); Cancel and Esc drop them.

use soyuz_appcore::models::dialogs::DialogModel;
use soyuz_appcore::tr;

use super::prompt::{buttons, click, left_click, list_key, wheel};
use super::{DialogInput, DialogMouse, ModelUi, Outcome};
use crate::layout::HitTarget;
use crate::widgets::{Form, FormOutcome};

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(_m: &DialogModel) -> ModelUi {
    let mut form =
        Form::new(Vec::new(), buttons(&[tr!("common-cancel"), tr!("automation-approval-run")])).default_button(1);
    form.focus_button(1);
    ModelUi { form, ..ModelUi::default() }
}

fn on_form(m: &mut DialogModel, out: FormOutcome) -> Option<Outcome> {
    let DialogModel::ScriptApproval(s) = m else { return None };
    match out {
        FormOutcome::Pressed(1) => Some(Outcome::Close(s.run())),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => {
            s.cancel();
            Some(Outcome::Close(None))
        }
        _ => None,
    }
}

fn len(m: &DialogModel) -> usize {
    match m {
        DialogModel::ScriptApproval(s) => s.jobs.len(),
        _ => 0,
    }
}

/// A key press or paste.
pub fn on_input(m: &mut DialogModel, ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    let n = len(m);
    for ev in &i.events {
        if list_key(&mut ui.list, ev, n) {
            continue;
        }
        let out = ui.form.apply(ev);
        if let Some(o) = on_form(m, out) {
            return o;
        }
    }
    Outcome::Keep
}

/// A mouse event: buttons; a click on a change selects it, the wheel scrolls.
pub fn on_mouse(m: &mut DialogModel, ui: &mut ModelUi, e: &DialogMouse) -> Outcome {
    let n = len(m);
    if let (true, Some(HitTarget::DialogRow(r))) = (left_click(e), e.target) {
        ui.list.select(r, n);
        return Outcome::Keep;
    }
    if wheel(&mut ui.list, e, n) {
        return Outcome::Keep;
    }
    let out = click(&mut ui.form, e);
    on_form(m, out).unwrap_or(Outcome::Keep)
}

/// The prompt went away by itself.
pub fn stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use soyuz_appcore::automation::AutoCmd;
    use soyuz_appcore::cmd::Cmd;
    use soyuz_appcore::keys::input::InputEvent;
    use soyuz_appcore::models::dialogs::ScriptApprovalModel;
    use soyuz_jobs::{DeleteMode, JobKind};

    use super::*;

    fn chord(s: &str) -> DialogInput {
        DialogInput {
            key: Some(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)),
            events: vec![InputEvent::Chord(s.parse().unwrap())],
        }
    }

    /// A script that wants to trash `n` files.
    pub(crate) fn model(n: usize) -> DialogModel {
        let jobs = (0..n)
            .map(|i| {
                let path = PathBuf::from(format!("f{i}.txt"));
                let kind = JobKind::Delete { paths: vec![path], mode: DeleteMode::Trash };
                soyuz_script::PlannedJob { kind, summary: format!("Trash f{i}.txt") }
            })
            .collect();
        ScriptApprovalModel::new("Tidy".into(), jobs).into()
    }

    #[test]
    fn run_scroll_and_cancel() {
        let mut m = model(12);
        let mut u = ui(&m);
        on_input(&mut m, &mut u, &chord("End"));
        assert_eq!(u.list.cursor, 11);
        match on_input(&mut m, &mut u, &chord("Enter")) {
            Outcome::Close(Some(Cmd::Automation(AutoCmd::SubmitPlanned { title, jobs }))) => {
                assert_eq!(title, "Tidy");
                assert_eq!(jobs.len(), 12);
            }
            other => panic!("{other:?}"),
        }
        let mut m = model(1);
        let mut u = ui(&m);
        assert!(matches!(on_input(&mut m, &mut u, &chord("Esc")), Outcome::Close(None)));
        assert_eq!(len(&m), 0, "the planned jobs are dropped");
    }
}
