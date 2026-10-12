//! The view of interrupted jobs from the journal (`FrontendCmd::RecoveredJobs`): the
//! explanation, one row per operation (label, how far it got), the cursor operation's
//! details, the leftover checkbox (its label says what the scan found) and the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::recovery::{done_text, job_line};
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::recovery::State;
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let mut d = DialogVm {
        hints: tr!("tui-requests-recovery-hints").into(),
        alert: true,
        ..DialogVm::from_form(&s.title, &s.form)
    };
    let Some(m) = &s.model else { return d };
    d.body.push(tr!("dialogs-recovery-body").into());
    d.list = m.jobs.iter().map(|j| ListRow { text: safe(&j.label), right: done_text(j), role: None }).collect();
    d.list_state = s.list.clone();
    if let Some(j) = m.jobs.get(s.list.cursor) {
        d.body.push(safe(&job_line(j)));
    }
    if let Some(f) = d.fields.first_mut() {
        match m.leftovers() {
            None => f.label = tr!("dialogs-recovery-scanning").into(),
            Some(v) if v.is_empty() => {
                f.label = tr!("dialogs-recovery-no-leftovers").into();
                f.enabled = false;
            }
            Some(v) => f.label = tr!("dialogs-recovery-clean", count = v.len()),
        }
    }
    d
}
