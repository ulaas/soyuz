//! Open with (`FrontendCmd::OpenWith`): the applications registered for the file's type
//! (looked up on a worker thread from `soyuz_fs::launch`, the frame loop is woken when
//! they are known), the programs picked earlier, or another program: Browse lists programs
//! folder by folder ([`Browser`]), or type its full path. Open queues `Cmd::OpenWith`
//! (`soyuz_appcore::models::requests::open_with::choose` validates first, the message
//! shows under the field); Cancel / Esc closes. Up / Down move through the list from
//! anywhere; a click picks a row, a second click on it opens.
//!
//! Like the GUI, it starts on the extension's default or last-used application
//! (`[open]`, `model::preselect`, applied by [`tick`] once the applications are known),
//! and a file with an extension gets an "Always use this app for .ext files" checkbox
//! (Tab to it, Space ticks): the choice becomes the extension's default.

use std::path::PathBuf;

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::file_browser::{BrowseFilter, FileBrowser};
use soyuz_appcore::models::requests::open_with::{self as model, AppsLookup};
use soyuz_appcore::tr;
use soyuz_core::config::KeyName;
use soyuz_fs::AppInfo;

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
    /// What the list shows, once the applications are known ([`tick`]).
    pub listed: Vec<AppInfo>,
    pub list: SelectList,
    /// The starting choice was applied ([`tick`]).
    pub preselected: bool,
    /// The cursor row is a choice (preselected, or moved to): a list without one must not
    /// make its first row the default on Enter.
    pub chosen: bool,
    /// The program browser, in place of the dialog's own view.
    pub browser: Option<Browser>,
}

/// Browse: folders and programs, a query to narrow them, "All files".
pub struct Browser {
    pub files: FileBrowser,
    /// Fields: the query, all files; buttons: Open, Back.
    pub form: Form,
    pub list: SelectList,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Browser").field("dir", &self.files.dir()).finish_non_exhaustive()
    }
}

/// Open the dialog (from `crate::frontend`).
pub fn open(app: &mut App, path: PathBuf) {
    let apps = AppsLookup::start(&path, app.core.wake.clone());
    let s = State {
        title: model::title(&path),
        form: form(&path, "", false),
        path,
        apps,
        listed: Vec::new(),
        list: SelectList::new(),
        preselected: false,
        chosen: false,
        browser: None,
    };
    app.push_dialog(TuiDialog::OpenWith(s));
}

/// The form: the program field (`custom`), the "always" checkbox when `path` has an
/// extension, Open and Cancel.
pub fn form(path: &std::path::Path, custom: &str, always: bool) -> Form {
    let mut fields = vec![Field::text(tr!("dialogs-openwith-other"), custom)];
    if let Some(label) = model::always_label(path) {
        fields.push(Field::check(label, always));
    }
    let buttons = vec![
        Button::new(tr!("dialogs-openwith-open")),
        Button::new(tr!("dialogs-openwith-browse")).accel('b'),
        Button::new(tr!("common-cancel")),
    ];
    Form::new(fields, buttons)
}

fn browser_form(all: bool) -> Form {
    Form::new(
        vec![Field::text(tr!("dialogs-openwith-filter"), ""), Field::check(tr!("dialogs-openwith-all-files"), all)],
        vec![Button::new(tr!("dialogs-openwith-open")), Button::new(tr!("dialogs-openwith-back"))],
    )
}

/// Open the browser on the usual programs folder, or the last program's.
fn browse(core: &AppCore, s: &mut State) {
    let mut files = FileBrowser::new(BrowseFilter::Programs, core.wake.clone());
    files.navigate(model::browse_start(&core.config.open));
    s.browser = Some(Browser { files, form: browser_form(false), list: SelectList::new() });
}

/// Every frame: once the applications are known, start on the default / last-used one
/// (unless a program was typed meanwhile).
pub fn tick(core: &AppCore, s: &mut State) {
    if let Some(b) = &mut s.browser
        && b.files.poll()
    {
        b.list = SelectList::new();
    }
    if s.preselected {
        return;
    }
    let Some(apps) = s.apps.get() else { return };
    s.preselected = true;
    let (listed, pre) = model::start(&core.config.open, &s.path, &apps);
    s.listed = listed;
    let typed = custom(s);
    if !typed.trim().is_empty() {
        s.form = form(&s.path, &typed, pre.always);
        return;
    }
    if let Some(i) = pre.selected {
        s.list.select(i, s.listed.len());
        s.chosen = true;
    }
    s.form = form(&s.path, &pre.custom, pre.always);
}

/// The browser's cursor row: a folder is entered; a file comes back as the choice.
fn browser_activate(s: &mut State) {
    let Some(b) = &mut s.browser else { return };
    let Some(row) = b.files.row(b.list.cursor).cloned() else { return };
    if row.is_dir {
        b.files.navigate(row.path);
        b.list = SelectList::new();
        b.form = browser_form(b.files.show_all());
        return;
    }
    s.browser = None;
    let name = std::path::Path::new(&row.name).file_stem().map(|n| n.to_string_lossy().into_owned());
    let row_i = model::adopt(&mut s.listed, model::picked_app(&row.path, name));
    s.list.select(row_i, s.listed.len());
    s.chosen = true;
    s.form = form(&s.path, "", always(s));
}

fn browser_outcome(s: &mut State, o: FormOutcome) {
    let Some(b) = &mut s.browser else { return };
    match o {
        FormOutcome::Pressed(0) => browser_activate(s),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => s.browser = None,
        FormOutcome::Changed(0) => {
            let query = b.form.fields.first().and_then(|f| f.value()).unwrap_or_default().to_string();
            b.files.set_query(&query);
            b.list = SelectList::new();
        }
        FormOutcome::Changed(_) => {
            let all = b.form.fields.get(1).and_then(|f| f.on()).unwrap_or(false);
            b.files.set_show_all(all);
            b.list = SelectList::new();
        }
        _ => {}
    }
}

fn browser_input(s: &mut State, i: &DialogInput) {
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        let Some(b) = &mut s.browser else { return };
        if let InputEvent::Chord(c) = ev {
            let len = b.files.len();
            if matches!(c.key, KeyName::ArrowUp | KeyName::ArrowDown | KeyName::PageUp | KeyName::PageDown)
                && len > 0
                && b.list.apply(ev, len, height)
            {
                continue;
            }
            if c.key == KeyName::Backspace
                && b.files.query().is_empty()
                && let Some(parent) = b.files.parent()
            {
                b.files.navigate(parent);
                b.list = SelectList::new();
                b.form = browser_form(b.files.show_all());
                continue;
            }
        }
        let o = b.form.apply(ev);
        browser_outcome(s, o);
    }
}

fn browser_mouse(s: &mut State, e: &DialogMouse) {
    let Some(b) = &mut s.browser else { return };
    match list_mouse(&mut b.list, b.files.len(), e) {
        Some(ListClick::Row { again: true, .. }) => return browser_activate(s),
        Some(_) => return,
        None => {}
    }
    if let Some(o) = click(&mut b.form, e) {
        browser_outcome(s, o);
    }
}

fn custom(s: &State) -> String {
    s.form.fields.first().and_then(|f| f.value()).unwrap_or_default().to_string()
}

fn always(s: &State) -> bool {
    s.form.fields.get(1).and_then(|f| f.on()).unwrap_or(false)
}

/// Open with the typed program or the cursor row (or say what is missing).
fn submit(s: &mut State) -> Outcome {
    let selected = (s.chosen && !s.listed.is_empty()).then_some(s.list.cursor);
    match model::choose(&custom(s), selected, Some(&s.listed)) {
        Ok(app) => Outcome::Close(Some(Cmd::Batch(model::choice_cmds(&s.path, app, always(s))))),
        Err(e) => {
            s.form.error = Some(crate::dialog::safe(&e));
            Outcome::Keep
        }
    }
}

fn outcome(core: &AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => Some(submit(s)),
        FormOutcome::Pressed(1) => {
            browse(core, s);
            Some(Outcome::Keep)
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(_) => {
            s.form.error = None;
            None
        }
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    if s.browser.is_some() {
        browser_input(s, i);
        return Outcome::Keep;
    }
    let len = s.listed.len();
    let height = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        // The list's keys (Home / End stay with the text field).
        if let InputEvent::Chord(c) = ev
            && matches!(c.key, KeyName::ArrowUp | KeyName::ArrowDown | KeyName::PageUp | KeyName::PageDown)
            && len > 0
            && s.list.apply(ev, len, height)
        {
            s.form.error = None;
            s.chosen = true;
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = outcome(core, s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    if s.browser.is_some() {
        browser_mouse(s, e);
        return Outcome::Keep;
    }
    let len = s.listed.len();
    match list_mouse(&mut s.list, len, e) {
        Some(ListClick::Row { again, .. }) => {
            let was_chosen = std::mem::replace(&mut s.chosen, true);
            return if again && was_chosen { submit(s) } else { Outcome::Keep };
        }
        Some(_) => return Outcome::Keep,
        None => {}
    }
    match click(&mut s.form, e) {
        Some(o) => outcome(core, s, o).unwrap_or(Outcome::Keep),
        None => Outcome::Keep,
    }
}

/// The dialog went away by itself (its pane or files are gone, …).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}
