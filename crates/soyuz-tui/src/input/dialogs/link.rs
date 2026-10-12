//! New links: the core's "new link" prompt (`DialogModel::NewLink`) and the link dialog
//! the core requests for the selection (`FrontendCmd::MetaDialog(MetaDialog::Link)`,
//! the GUI's `dialogs::links`).
//!
//! The prompt mirrors the GUI's `ui/dialogs/new_item.rs`: the kind (symbolic link, hard
//! link, junction on Windows; Left / Right pick, a click on the focused group picks the
//! next), the target (focused first; the name follows it until the name is edited) and
//! the name, then Cancel / Create. Enter creates when `LinkModel::create` accepts the
//! form (its error shows under the fields once a target is typed), Esc cancels.
//!
//! The link dialog for the selection mirrors the GUI's `ui/dialogs/links.rs` on the core's
//! `models::link_dialog::LinkForm`: kind, target, "relative path" (symbolic links), name;
//! under them the kind's hint, the target's state (checking, missing, file, folder: a
//! debounced probe off the UI thread, [`tick`]) and a target picker listing a folder
//! (the target's, following typed folders). With the target focused, Up / Down move the
//! picker's cursor and Tab / Enter take the row (a folder opens in the picker; ".."
//! goes up); a click takes a row too. Enter creates (a `Create` job, undoable) once the
//! form is valid, Esc cancels.

use std::path::PathBuf;
use std::time::Instant;

use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Wake;
use soyuz_appcore::models::dialogs::{DialogModel, LinkChoice, LinkModel};
use soyuz_appcore::models::link_dialog::{LinkForm, PROBE_DEBOUNCE, Probe, TargetState, list_folder, probe_target};
use soyuz_appcore::models::properties::{Bg, LinkKind};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_core::EntryInfo;
use soyuz_core::config::KeyName;

use super::prompt::{buttons, click, is_key, left_click, set_text, wheel};
use super::{DialogInput, DialogMouse, ModelUi, Outcome, TuiDialog};
use crate::app::App;
use crate::dialog::safe;
use crate::layout::HitTarget;
use crate::widgets::{Field, Form, FormOutcome, SelectList};

// ---- the core prompt (`DialogModel::NewLink`) -----------------------------------------

/// Field indexes of the prompt's form.
pub const KIND: usize = 0;
pub const TARGET: usize = 1;
pub const NAME: usize = 2;

/// The kinds offered on this platform, in the radio group's order.
pub fn kinds() -> Vec<LinkChoice> {
    LinkChoice::offered().collect()
}

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(m: &DialogModel) -> ModelUi {
    let DialogModel::NewLink(l) = m else { return ModelUi::default() };
    let kinds = kinds();
    let labels = kinds.iter().map(|k| k.label().to_string()).collect();
    let selected = kinds.iter().position(|k| *k == l.kind).unwrap_or(0);
    let mut form = Form::new(
        vec![
            Field::radio(tr!("tui-dialogs-link-kind"), labels, selected),
            Field::text(tr!("dialogs-link-target"), l.target.clone()),
            Field::text(tr!("dialogs-link-name"), l.name.clone()),
        ],
        buttons(&[tr!("common-cancel"), tr!("dialogs-link-create")]),
    )
    .default_button(1);
    form.focus = TARGET;
    form.error = error(l);
    ModelUi { form, ..ModelUi::default() }
}

/// Why the link cannot be created, once a target is typed (like the GUI).
fn error(l: &LinkModel) -> Option<String> {
    if l.target.trim().is_empty() { None } else { l.error() }
}

fn on_form(m: &mut DialogModel, ui: &mut ModelUi, out: FormOutcome) -> Option<Outcome> {
    let DialogModel::NewLink(l) = m else { return None };
    match out {
        FormOutcome::Changed(KIND) => {
            if let Some(k) = ui.form.fields[KIND].selected().and_then(|i| kinds().get(i).copied()) {
                l.kind = k;
            }
        }
        FormOutcome::Changed(TARGET) => {
            l.target = ui.form.fields[TARGET].value().unwrap_or_default().to_string();
            l.target_edited();
            if ui.form.fields[NAME].value() != Some(l.name.as_str()) {
                let name = l.name.clone();
                set_text(&mut ui.form, NAME, &name);
            }
        }
        FormOutcome::Changed(_) => {
            l.name = ui.form.fields[NAME].value().unwrap_or_default().to_string();
            l.name_edited();
        }
        FormOutcome::Pressed(1) => {
            return match l.create() {
                Some(c) => Some(Outcome::Close(Some(c))),
                None => {
                    // Say why (also before a target was typed).
                    ui.form.error = l.error();
                    None
                }
            };
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => return Some(Outcome::Close(None)),
        FormOutcome::Consumed | FormOutcome::Ignored => return None,
    }
    ui.form.error = error(l);
    None
}

/// A key press or paste on the prompt.
pub fn on_model_input(m: &mut DialogModel, ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        let out = ui.form.apply(ev);
        if let Some(o) = on_form(m, ui, out) {
            return o;
        }
    }
    Outcome::Keep
}

/// A mouse event on the prompt.
pub fn on_model_mouse(m: &mut DialogModel, ui: &mut ModelUi, e: &DialogMouse) -> Outcome {
    let out = click(&mut ui.form, e);
    on_form(m, ui, out).unwrap_or(Outcome::Keep)
}

/// The prompt went away by itself.
pub fn model_stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}

// ---- the terminal UI dialog (`MetaDialog::Link`) ---------------------------------------

/// Field indexes of the link dialog's form.
pub const SEL_KIND: usize = 0;
pub const SEL_TARGET: usize = 1;
pub const SEL_RELATIVE: usize = 2;
pub const SEL_NAME: usize = 3;
/// The Create button (the other one cancels).
const CREATE: usize = 1;

/// Background listing of the picker folder.
type Listing = Bg<Result<Vec<EntryInfo>, String>>;

/// A row of the target picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickRow {
    pub label: String,
    pub path: PathBuf,
    pub dir: bool,
}

/// What the dialog keeps while it is open.
#[derive(Default)]
pub struct State {
    pub title: String,
    pub form: Form,
    /// The core's form (`None` only in a default `State`).
    pub link: Option<LinkForm>,
    /// The probe of the target path `.0` (debounced, background).
    pub probe: Option<(PathBuf, Bg<Probe>)>,
    /// When the target was last typed (debounce start).
    pub edited_at: Option<Instant>,
    /// Switch the picker to the probe's folder once it lands (typed edits).
    pub follow_browse: bool,
    /// The picker's folder and its listing.
    pub browse: PathBuf,
    pub listing: Option<(PathBuf, Listing)>,
    /// The picker's cursor; `armed` once Up / Down moved it (Tab / Enter then take it).
    pub list: SelectList,
    pub armed: bool,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("link::State")
            .field("title", &self.title)
            .field("link", &self.link)
            .field("browse", &self.browse)
            .field("list", &self.list)
            .finish_non_exhaustive()
    }
}

/// Open the link dialog of kind `kind` for the focused pane's selection (an info toast
/// when the pane is not a folder on disk).
pub fn open(app: &mut App, kind: LinkKind) {
    let choice = match kind {
        LinkKind::Symlink => LinkChoice::Symlink,
        LinkKind::Hardlink => LinkChoice::Hardlink,
        LinkKind::Junction => LinkChoice::Junction,
    };
    match LinkForm::open(&app.core, choice) {
        Ok(f) => {
            let s = State::new(f);
            app.push_dialog(TuiDialog::Link(s));
        }
        Err(e) => {
            app.core.ui.toasts.push(Toast::info(e));
        }
    }
}

impl State {
    /// The dialog for `f`: the target focused (the picker shows the target's folder).
    pub fn new(f: LinkForm) -> State {
        let kinds = kinds();
        let labels = kinds.iter().map(|k| k.label().to_string()).collect();
        let selected = kinds.iter().position(|k| *k == f.kind).unwrap_or(0);
        let mut form = Form::new(
            vec![
                Field::radio(tr!("tui-dialogs-link-kind"), labels, selected),
                Field::text(tr!("dialogs-link-target"), f.target.clone()),
                Field::check(tr!("dialogs-link-relative"), f.relative),
                Field::text(tr!("dialogs-link-name"), f.name.clone()),
            ],
            buttons(&[tr!("common-cancel"), tr!("dialogs-link-create")]),
        )
        .default_button(CREATE);
        form.focus = SEL_TARGET;
        let mut s = State {
            title: tr!("dialogs-link-title").into(),
            form,
            browse: f.start_folder(),
            link: Some(f),
            ..State::default()
        };
        s.refresh();
        s
    }

    /// The probe's answer for the typed target (`Checking` while it runs).
    pub fn target_state(&self) -> (TargetState, bool) {
        let Some(f) = &self.link else { return (TargetState::Empty, true) };
        if f.target.trim().is_empty() {
            return (TargetState::Empty, true);
        }
        let tp = f.target_path();
        match self.probe.as_ref().filter(|(p, _)| *p == tp).and_then(|(_, bg)| bg.with(|r| r.cloned())) {
            Some(p) => (p.state, p.same_volume),
            None => (TargetState::Checking, true),
        }
    }

    /// The form follows the model: Relative only for symbolic links, the error (once a
    /// target is typed and checked), Create only when nothing is wrong.
    pub fn refresh(&mut self) {
        let (state, same_volume) = self.target_state();
        let Some(f) = &self.link else { return };
        self.form.fields[SEL_RELATIVE].enabled = f.relative_applies();
        let err = f.error(&state, same_volume);
        self.form.buttons[CREATE].enabled = err.is_none();
        self.form.error = err.filter(|_| !f.target.trim().is_empty() && state != TargetState::Checking);
    }

    /// The picker's rows: the parent folder, then the folder's items (folders first).
    pub fn rows(&self) -> Vec<PickRow> {
        let mut out = Vec::new();
        if let Some(p) = self.browse.parent() {
            out.push(PickRow { label: "..".into(), path: p.to_path_buf(), dir: true });
        }
        if let Some(Some(Ok(entries))) =
            self.listing.as_ref().filter(|(d, _)| *d == self.browse).map(|(_, bg)| bg.with(|l| l.cloned()))
        {
            let sep = std::path::MAIN_SEPARATOR;
            out.extend(entries.into_iter().map(|e| {
                let dir = e.is_dir_like();
                let label = if dir { format!("{}{sep}", safe(&e.name)) } else { safe(&e.name) };
                PickRow { label, path: e.path, dir }
            }));
        }
        out
    }

    /// Take picker row `i`: it becomes the target (the name follows); a folder also opens
    /// in the picker.
    pub fn pick(&mut self, i: usize) {
        let Some(row) = self.rows().get(i).cloned() else { return };
        let Some(f) = self.link.as_mut() else { return };
        f.set_target(&row.path);
        let (target, name) = (f.target.clone(), f.name.clone());
        set_text(&mut self.form, SEL_TARGET, &target);
        set_text(&mut self.form, SEL_NAME, &name);
        if row.dir {
            self.browse = row.path;
            self.list = SelectList::default();
        }
        self.armed = false;
        self.follow_browse = false;
        self.edited_at = None;
        self.refresh();
    }
}

/// Per frame (`view::prepare`): the debounced target probe, the picker following a typed
/// folder, the picker's listing (both off the UI thread), the form's state.
pub fn tick(core: &AppCore, st: &mut super::State) {
    for (_, d) in &mut st.tui.stack {
        if let TuiDialog::Link(s) = d {
            tick_one(&core.wake, s);
        }
    }
}

fn tick_one(wake: &Wake, s: &mut State) {
    let Some(f) = &s.link else { return };
    if !f.target.trim().is_empty() {
        let tp = f.target_path();
        let fresh = s.probe.as_ref().is_some_and(|(p, _)| *p == tp);
        if !fresh && s.edited_at.is_none_or(|t| t.elapsed() >= PROBE_DEBOUNCE) {
            let (t2, d2) = (tp.clone(), f.dir.clone());
            s.probe = Some((tp, Bg::spawn_wake(wake.clone(), move || probe_target(&t2, &d2))));
        }
    }
    if s.follow_browse
        && let Some((_, bg)) = &s.probe
        && let Some(p) = bg.with(|r| r.cloned())
    {
        s.follow_browse = false;
        if let Some(b) = p.browse
            && b != s.browse
        {
            s.browse = b;
            s.list = SelectList::default();
        }
    }
    if s.listing.as_ref().is_none_or(|(d, _)| *d != s.browse) {
        let d2 = s.browse.clone();
        s.listing = Some((s.browse.clone(), Bg::spawn_wake(wake.clone(), move || list_folder(&d2))));
    }
    s.refresh();
}

fn on_dialog_form(s: &mut State, out: FormOutcome) -> Option<Outcome> {
    let f = s.link.as_mut()?;
    match out {
        FormOutcome::Changed(SEL_KIND) => {
            if let Some(k) = s.form.fields[SEL_KIND].selected().and_then(|i| kinds().get(i).copied()) {
                f.kind = k;
            }
        }
        FormOutcome::Changed(SEL_TARGET) => {
            f.target = s.form.fields[SEL_TARGET].value().unwrap_or_default().to_string();
            f.target_typed();
            let name = f.name.clone();
            if s.form.fields[SEL_NAME].value() != Some(name.as_str()) {
                set_text(&mut s.form, SEL_NAME, &name);
            }
            s.edited_at = Some(Instant::now());
            s.follow_browse = true;
            s.armed = false;
        }
        FormOutcome::Changed(SEL_RELATIVE) => {
            f.relative = s.form.fields[SEL_RELATIVE].on().unwrap_or(false);
        }
        FormOutcome::Changed(_) => {
            f.name = s.form.fields[SEL_NAME].value().unwrap_or_default().to_string();
            f.name_edited();
        }
        FormOutcome::Pressed(CREATE) => {
            let (state, same_volume) = s.target_state();
            if let Some(c) = s.link.as_ref().and_then(|f| f.create(&state, same_volume)) {
                return Some(Outcome::Close(Some(c)));
            }
            s.refresh();
            return None;
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => return Some(Outcome::Close(None)),
        FormOutcome::Consumed | FormOutcome::Ignored => return None,
    }
    s.refresh();
    None
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    handle_input(s, i)
}

fn handle_input(s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        if s.form.focused_field() == Some(SEL_TARGET) {
            let n = s.rows().len();
            if n > 0 && (is_key(ev, KeyName::ArrowDown) || is_key(ev, KeyName::ArrowUp)) {
                // Up / Down move the picker (wrapping); the first press starts at the top.
                let c = if !s.armed {
                    0
                } else if is_key(ev, KeyName::ArrowDown) {
                    (s.list.cursor + 1) % n
                } else {
                    (s.list.cursor + n - 1) % n
                };
                s.list.select(c, n);
                s.armed = true;
                continue;
            }
            if s.armed && (is_key(ev, KeyName::Tab) || is_key(ev, KeyName::Enter)) {
                let c = s.list.cursor;
                s.pick(c);
                continue;
            }
            if s.armed && is_key(ev, KeyName::Escape) {
                s.armed = false;
                continue;
            }
        }
        let out = s.form.apply(ev);
        if let Some(o) = on_dialog_form(s, out) {
            return o;
        }
    }
    Outcome::Keep
}

/// A mouse event: a click on a picker row takes it, the wheel scrolls the picker.
pub fn on_mouse(_core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    if let (true, Some(HitTarget::DialogRow(r))) = (left_click(e), e.target) {
        s.list.select(r, s.rows().len());
        s.pick(r);
        s.form.focus = SEL_TARGET;
        return Outcome::Keep;
    }
    let n = s.rows().len();
    if wheel(&mut s.list, e, n) {
        return Outcome::Keep;
    }
    let out = click(&mut s.form, e);
    on_dialog_form(s, out).unwrap_or(Outcome::Keep)
}

/// The dialog went away by itself.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use soyuz_appcore::cmd::Cmd;
    use soyuz_appcore::keys::input::InputEvent;
    use soyuz_appcore::ops::fileops::FileOpCmd;

    use super::*;

    fn chord(s: &str) -> DialogInput {
        DialogInput {
            key: Some(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)),
            events: vec![InputEvent::Chord(s.parse().unwrap())],
        }
    }

    fn text(s: &str) -> DialogInput {
        DialogInput { key: None, events: vec![InputEvent::Text(s.into())] }
    }

    #[test]
    fn kind_target_name_and_create() {
        let dir = std::env::temp_dir();
        let mut m: DialogModel = LinkModel::new(dir, None, LinkChoice::Symlink, vec!["taken".into()]).into();
        let mut u = ui(&m);
        assert_eq!(u.form.focused_field(), Some(TARGET));
        assert!(u.form.error.is_none(), "no error before a target is typed");
        assert!(matches!(on_model_input(&mut m, &mut u, &chord("Enter")), Outcome::Keep));
        assert!(u.form.error.is_some(), "Create says why it cannot");
        on_model_input(&mut m, &mut u, &text("some/where/file.txt"));
        let name = u.form.fields[NAME].value().unwrap().to_string();
        assert!(!name.is_empty(), "the name follows the target");
        assert!(matches!(&m, DialogModel::NewLink(l) if l.name == name));
        // Shift+Tab to the kind, Right picks a hard link.
        on_model_input(&mut m, &mut u, &chord("Shift+Tab"));
        on_model_input(&mut m, &mut u, &chord("Right"));
        assert!(matches!(&m, DialogModel::NewLink(l) if l.kind == LinkChoice::Hardlink));
        // A taken name is an error; a free one creates.
        on_model_input(&mut m, &mut u, &chord("Tab"));
        on_model_input(&mut m, &mut u, &chord("Tab"));
        assert_eq!(u.form.focused_field(), Some(NAME));
        for _ in 0..name.chars().count() {
            on_model_input(&mut m, &mut u, &chord("Backspace"));
        }
        on_model_input(&mut m, &mut u, &text("TAKEN"));
        assert!(u.form.error.is_some());
        on_model_input(&mut m, &mut u, &chord("Backspace"));
        match on_model_input(&mut m, &mut u, &chord("Enter")) {
            Outcome::Close(Some(Cmd::FileOp(FileOpCmd::CreateLink {
                name,
                kind: LinkChoice::Hardlink,
                target,
                ..
            }))) => {
                assert_eq!(name, "TAKE");
                assert_eq!(target, PathBuf::from("some/where/file.txt"));
            }
            other => panic!("{other:?}"),
        }
    }

    /// Run [`tick_one`] until `done` holds (the probe and the listing are background work).
    fn settle(s: &mut State, done: impl Fn(&State) -> bool) {
        let wake = soyuz_appcore::host::no_wake();
        let until = Instant::now() + std::time::Duration::from_secs(10);
        while !done(s) && Instant::now() < until {
            tick_one(&wake, s);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(done(s), "{s:?}");
        s.refresh();
    }

    #[test]
    fn link_dialog_picker_probe_and_create() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let f = LinkForm::new(dir.path().to_path_buf(), None, LinkChoice::Hardlink, vec!["a.txt".into(), "sub".into()]);
        let mut s = State::new(f);
        assert_eq!(s.form.focused_field(), Some(SEL_TARGET));
        assert!(!s.form.buttons[CREATE].enabled, "no target yet");
        assert!(s.form.error.is_none(), "no message before a target is typed");
        settle(&mut s, |s| s.rows().len() == 3);
        let rows = s.rows();
        assert_eq!(rows[0].label, "..");
        assert!(rows[1].dir && rows[1].label.starts_with("sub"), "folders first: {rows:?}");
        assert!(!s.form.fields[SEL_RELATIVE].enabled, "relative: symbolic links only");
        // Down arms the picker on "..", Down again on sub, Up / Down wrap; Tab takes a.txt.
        let core_less = |s: &mut State, c: &str| handle_input(s, &chord(c));
        core_less(&mut s, "Down");
        assert!(s.armed && s.list.cursor == 0);
        core_less(&mut s, "Up");
        assert_eq!(s.list.cursor, 2, "wraps");
        core_less(&mut s, "Tab");
        assert!(!s.armed);
        assert!(s.link.as_ref().unwrap().target.ends_with("a.txt"));
        assert_ne!(s.form.fields[SEL_NAME].value(), Some(""), "the name follows");
        settle(&mut s, |s| s.target_state().0 == TargetState::File);
        assert!(s.form.buttons[CREATE].enabled);
        // A folder is no target for a hard link.
        core_less(&mut s, "Down");
        core_less(&mut s, "Down");
        core_less(&mut s, "Enter");
        assert_eq!(s.browse, dir.path().join("sub"), "a folder opens in the picker");
        settle(&mut s, |s| s.target_state().0 == TargetState::Dir);
        assert_eq!(s.form.error.as_deref(), Some(tr!("dialogs-link-hard-files-only")));
        assert!(!s.form.buttons[CREATE].enabled);
        // A symbolic link may point to a folder, relative to the link's folder.
        s.form.focus = SEL_KIND;
        let _ = core_less(&mut s, "Left");
        assert_eq!(s.link.as_ref().unwrap().kind, LinkChoice::Symlink);
        assert!(s.form.fields[SEL_RELATIVE].enabled);
        match core_less(&mut s, "Enter") {
            Outcome::Close(Some(Cmd::Meta(soyuz_appcore::models::properties::MetaCmd::Create { kind, .. }))) => {
                assert!(matches!(kind, soyuz_jobs::CreateKind::Symlink { .. }), "{kind:?}")
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn esc_cancels() {
        let mut m: DialogModel = LinkModel::new(std::env::temp_dir(), None, LinkChoice::Symlink, Vec::new()).into();
        let mut u = ui(&m);
        assert!(matches!(on_model_input(&mut m, &mut u, &chord("Esc")), Outcome::Close(None)));
    }
}
