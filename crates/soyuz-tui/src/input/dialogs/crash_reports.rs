//! Crash reports from the last run (`FrontendCmd::CrashReports`): the reports (newest
//! first), View (the report's text replaces the list; V or Esc goes back), Delete
//! reports, Open folder (a new tab on the crashes folder) and Close (keeps them, not
//! offered again); Esc closes without deciding, like the GUI. The rules are
//! `soyuz_appcore::models::requests::crash_reports`; the choices are carried out by
//! `services::crash::pump`.
//!
//! Restart in safe mode is GUI-only: a second terminal UI cannot start in this terminal.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::requests::crash_reports::CrashReportsModel;
use soyuz_appcore::services::crash::CrashReport;
use soyuz_appcore::tr;

use super::confirm::{ListClick, click, list_mouse};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Form, FormOutcome, SelectList};

/// Buttons, in order.
pub const VIEW: usize = 0;
pub const DELETE: usize = 1;
pub const FOLDER: usize = 2;
pub const CLOSE: usize = 3;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    /// Buttons: View / Hide, Delete reports, Open folder, Close.
    pub form: Form,
    /// The reports and the one viewed (`None` only in a default state).
    pub model: Option<CrashReportsModel>,
    /// The cursor in the list of reports.
    pub list: SelectList,
    /// The scroll of the viewed report's lines.
    pub text: SelectList,
}

/// Open the dialog (from `crate::frontend`).
pub fn open(app: &mut App, reports: Vec<CrashReport>) {
    let model = CrashReportsModel::new(reports);
    let mut form = Form::new(
        Vec::new(),
        vec![
            Button::new(tr!("dialogs-crash-view")).accel('v'),
            Button::new(tr!("dialogs-crash-delete")).accel('d'),
            Button::new(tr!("dialogs-crash-open-folder")).accel('o'),
            Button::new(tr!("common-close")).accel('c'),
        ],
    )
    .default_button(VIEW);
    form.focus_button(VIEW);
    let s = State {
        title: model.title.clone(),
        form,
        model: Some(model),
        list: SelectList::new(),
        text: SelectList::new(),
    };
    app.push_dialog(TuiDialog::CrashReports(s));
}

/// The lines of the viewed report (none when no report is viewed).
pub fn text_lines(s: &State) -> Vec<String> {
    let Some((_, text)) = s.model.as_ref().and_then(|m| m.viewing.as_ref()) else { return Vec::new() };
    text.lines().map(|l| crate::dialog::safe(&l.replace('\t', "    "))).collect()
}

fn viewing(s: &State) -> bool {
    s.model.as_ref().is_some_and(|m| m.viewing.is_some())
}

fn toggle_view(s: &mut State) {
    let i = s.list.cursor;
    if let Some(m) = s.model.as_mut() {
        m.toggle_view(i);
    }
    s.text = SelectList::new();
    s.form.buttons[VIEW].label = if viewing(s) { tr!("dialogs-hide") } else { tr!("dialogs-crash-view") }.into();
}

fn outcome(s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(VIEW) => {
            toggle_view(s);
            None
        }
        FormOutcome::Pressed(DELETE) => {
            s.model.as_ref()?.delete();
            Some(Outcome::Close(None))
        }
        FormOutcome::Pressed(FOLDER) => {
            s.model.as_ref()?.open_folder();
            Some(Outcome::Close(None))
        }
        FormOutcome::Pressed(_) => {
            s.model.as_ref()?.keep();
            Some(Outcome::Close(None))
        }
        // Esc leaves the viewed report first.
        FormOutcome::Cancel if viewing(s) => {
            toggle_view(s);
            None
        }
        FormOutcome::Cancel => Some(Outcome::Close(None)),
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        let used = if viewing(s) {
            let n = text_lines(s).len();
            s.text.apply(ev, n, height)
        } else {
            let n = s.model.as_ref().map_or(0, |m| m.listed().len());
            s.list.apply(ev, n, height)
        };
        if used {
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
    if viewing(s) {
        let n = text_lines(s).len();
        if list_mouse(&mut s.text, n, e).is_some() {
            return Outcome::Keep;
        }
    } else {
        let n = s.model.as_ref().map_or(0, |m| m.listed().len());
        match list_mouse(&mut s.list, n, e) {
            Some(ListClick::Row { again: true, .. }) => {
                toggle_view(s);
                return Outcome::Keep;
            }
            Some(_) => return Outcome::Keep,
            None => {}
        }
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
