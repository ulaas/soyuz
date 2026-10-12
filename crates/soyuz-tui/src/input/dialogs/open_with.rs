//! Open with (`FrontendCmd::OpenWith`): the applications registered for the file's type
//! (looked up on a worker thread from `soyuz_fs::launch`, the frame loop is woken when
//! they are known), or another program typed as a full path. Open queues `Cmd::OpenWith`
//! (`soyuz_appcore::models::requests::open_with::choose` validates first, the message
//! shows under the field); Cancel / Esc closes. Up / Down move through the list from
//! anywhere; a click picks a row, a second click on it opens.
//!
//! The GUI has no "always use this application" choice, so neither does this dialog.

use std::path::PathBuf;

use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::requests::open_with::{self as model, AppsLookup};
use soyuz_appcore::tr;
use soyuz_core::config::KeyName;

use super::confirm::{ListClick, click, list_mouse};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Field, Form, FormOutcome, SelectList};

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    /// Fields: another program; buttons: Open, Cancel.
    pub form: Form,
    pub path: PathBuf,
    pub apps: AppsLookup,
    pub list: SelectList,
}

/// Open the dialog (from `crate::frontend`).
pub fn open(app: &mut App, path: PathBuf) {
    let apps = AppsLookup::start(&path, app.core.wake.clone());
    let s = State { title: model::title(&path), form: form(), path, apps, list: SelectList::new() };
    app.push_dialog(TuiDialog::OpenWith(s));
}

/// The form of a new dialog.
pub fn form() -> Form {
    Form::new(
        vec![Field::text(tr!("dialogs-openwith-other"), "")],
        vec![Button::new(tr!("dialogs-openwith-open")), Button::new(tr!("common-cancel"))],
    )
}

fn custom(s: &State) -> String {
    s.form.fields.first().and_then(|f| f.value()).unwrap_or_default().to_string()
}

/// Open with the typed program or the cursor row (or say what is missing).
fn submit(s: &mut State) -> Outcome {
    let apps = s.apps.get();
    let selected = apps.as_ref().filter(|l| !l.is_empty()).map(|_| s.list.cursor);
    match model::choose(&custom(s), selected, apps.as_deref()) {
        Ok(app) => Outcome::Close(Some(model::open_cmd(&s.path, app))),
        Err(e) => {
            s.form.error = Some(crate::dialog::safe(&e));
            Outcome::Keep
        }
    }
}

fn outcome(s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => Some(submit(s)),
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
    let len = s.apps.get().map_or(0, |l| l.len());
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        // The list's keys (Home / End stay with the text field).
        if let InputEvent::Chord(c) = ev
            && matches!(c.key, KeyName::ArrowUp | KeyName::ArrowDown | KeyName::PageUp | KeyName::PageDown)
            && len > 0
            && s.list.apply(ev, len, height)
        {
            s.form.error = None;
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
    let len = s.apps.get().map_or(0, |l| l.len());
    match list_mouse(&mut s.list, len, e) {
        Some(ListClick::Row { again: true, .. }) => return submit(s),
        Some(_) => return Outcome::Keep,
        None => {}
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
