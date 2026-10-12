//! The view of crash reports from the last run (`FrontendCmd::CrashReports`): the
//! explanation and one row per report (summary, when), or the viewed report's text; then
//! the buttons.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::crash_reports::{summary, when};
use soyuz_appcore::tr;

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::crash_reports::{State, text_lines};
use crate::widgets::ListRow;

/// What the dialog shows.
pub fn vm(_core: &AppCore, s: &State) -> DialogVm {
    let mut d = DialogVm { width: 90, ..DialogVm::from_form(&s.title, &s.form) };
    let Some(m) = &s.model else { return d };
    if m.viewing.is_some() {
        d.list = text_lines(s).into_iter().map(ListRow::new).collect();
        d.list_state = s.text.clone();
        d.hints = tr!("tui-requests-crash-viewing-hints").into();
    } else {
        d.body.push(tr!("dialogs-crash-body").into());
        d.list = m.listed().iter().map(|r| ListRow { text: safe(&summary(r)), right: when(r), role: None }).collect();
        d.list_state = s.list.clone();
        d.hints = tr!("tui-requests-crash-hints").into();
    }
    d
}
