//! Bulk attributes and times (`FrontendCmd::MetaDialog(MetaDialog::Attributes)`, the
//! core's `Action::SetAttributes`) — Phase T3.
//!
//! The editing state is the core's `soyuz_appcore::models::attrs_edit` (shared with the
//! GUI's dialog): one field per attribute this platform can change (read-only, hidden,
//! system, archive, … on Windows; locked / hidden on macOS; immutable on Linux) — a
//! checkbox when every selected item agrees, else a "mixed / on / off" choice that keeps
//! each item's value until changed —, on unix the permissions as an octal mode (`644`;
//! empty when the items differ = keep), the created / modified / accessed times as text
//! (`YYYY-MM-DD HH:MM:SS` or `now`; empty = keep; Linux cannot set the creation time), and
//! "also inside the selected folders". Apply submits undoable `SetMeta` jobs
//! (`MetaCmd::SetMeta`); with the recursive option the folders are walked in the
//! background first ([`tick`] submits when the walk ends).

use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;
use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Wake;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::attrs_edit::{
    AttrEdit, ModeEdit, Tri, editable_attrs, mode_string, parse_octal, parse_time, plan_jobs, time_text,
};
use soyuz_appcore::models::properties::{MetaCmd, MetaJob, targets};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_core::{Control, EntryInfo, EntryKind, OpControl};
use soyuz_fs::{TimesSet, WalkItem, WalkOptions};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Field, FieldKind, Form, FormOutcome};

/// What a form field edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Row `i` of the [`AttrEdit`].
    Attr(usize),
    /// The octal mode.
    Mode,
    /// Created (0), modified (1), accessed (2).
    Time(usize),
    /// Also inside the selected folders.
    Recursive,
}

/// Cancel handle and result of the recursive walk.
type Collecting = (Arc<OpControl>, Arc<Mutex<Option<Vec<EntryInfo>>>>);

const APPLY: usize = 0;

/// What the dialog keeps while it is open.
#[derive(Default)]
pub struct State {
    pub title: String,
    pub form: Form,
    pub entries: Vec<EntryInfo>,
    pub attrs: Option<AttrEdit>,
    pub mode: Option<ModeEdit>,
    /// What each form field edits.
    pub slots: Vec<Slot>,
    /// Per time field, the text it started with.
    pub times0: [String; 3],
    /// The walk of the recursive Apply.
    pub collecting: Option<Collecting>,
    /// Applied (the dialog closes on its next frame).
    pub done: bool,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("attributes::State")
            .field("title", &self.title)
            .field("entries", &self.entries.len())
            .field("slots", &self.slots)
            .field("collecting", &self.collecting.is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some((c, _)) = &self.collecting {
            c.cancel();
        }
    }
}

/// Open the dialog (from `crate::frontend`) on the focused pane's selection (the folder
/// itself when nothing is selected).
pub fn open(app: &mut App) {
    match targets(&app.core) {
        Ok((_, entries)) => app.push_dialog(TuiDialog::Attributes(State::new(entries))),
        Err(e) => {
            app.core.ui.toasts.push(Toast::info(e));
        }
    }
}

/// The value all `vals` share, if they do.
fn same<T: PartialEq + Copy>(vals: impl Iterator<Item = T>) -> Option<T> {
    let mut it = vals;
    let first = it.next()?;
    it.all(|v| v == first).then_some(first)
}

/// Linux file systems cannot change the creation time.
const CREATED_SETTABLE: bool = !cfg!(target_os = "linux");

impl State {
    pub fn new(entries: Vec<EntryInfo>) -> State {
        let title = if entries.len() == 1 {
            tr!("dialogs-attrs-title-one", name = crate::dialog::safe(&entries[0].name))
        } else {
            tr!("dialogs-attrs-title-many", count = entries.len())
        };
        let attrs = AttrEdit::new(&entries);
        let mode = ModeEdit::new(&entries);
        let mut fields = Vec::new();
        let mut slots = Vec::new();
        let labels = editable_attrs();
        for (i, ((_, now, _), (_, label, _))) in attrs.rows.iter().zip(&labels).enumerate() {
            fields.push(match now {
                Tri::Mixed => Field::radio(
                    *label,
                    vec![tr!("dialogs-mode-mixed").into(), tr!("tui-ops-on").into(), tr!("tui-ops-off").into()],
                    0,
                ),
                t => Field::check(*label, *t == Tri::On),
            });
            slots.push(Slot::Attr(i));
        }
        if let Some(m) = &mode {
            fields.push(Field::text(tr!("tui-ops-mode"), m.octal.clone()));
            slots.push(Slot::Mode);
        }
        let mut times0: [String; 3] = Default::default();
        let labels = [tr!("dialogs-props-created"), tr!("dialogs-props-modified"), tr!("dialogs-props-accessed")];
        for (i, label) in labels.into_iter().enumerate() {
            if i == 0 && !CREATED_SETTABLE {
                continue;
            }
            let get = |e: &EntryInfo| match i {
                0 => e.created,
                1 => e.modified,
                _ => e.accessed,
            };
            times0[i] = same(entries.iter().map(get)).flatten().map(time_text).unwrap_or_default();
            fields.push(Field::text(label, times0[i].clone()));
            slots.push(Slot::Time(i));
        }
        if entries.iter().any(|e| e.kind == EntryKind::Dir) {
            fields.push(Field::check(tr!("dialogs-attrs-recursive"), false));
            slots.push(Slot::Recursive);
        }
        let mut form =
            Form::new(fields, vec![Button::new(tr!("common-apply")).accel('a'), Button::new(tr!("common-cancel"))])
                .default_button(APPLY);
        form.buttons[APPLY].enabled = false;
        State { title, form, entries, attrs: Some(attrs), mode, slots, times0, collecting: None, done: false }
    }

    fn field(&self, slot: Slot) -> Option<&Field> {
        self.slots.iter().position(|s| *s == slot).and_then(|i| self.form.fields.get(i))
    }

    /// Copy the fields into the edits; `Err` is the message of a field that cannot be used.
    pub fn sync(&mut self) -> Result<(), String> {
        for (i, slot) in self.slots.iter().enumerate() {
            let f = &self.form.fields[i];
            match (*slot, &f.kind) {
                (Slot::Attr(r), FieldKind::Check(on)) => {
                    if let Some(row) = self.attrs.as_mut().and_then(|a| a.rows.get_mut(r)) {
                        row.1 = if *on { Tri::On } else { Tri::Off };
                    }
                }
                (Slot::Attr(r), FieldKind::Radio { selected, .. }) => {
                    if let Some(row) = self.attrs.as_mut().and_then(|a| a.rows.get_mut(r)) {
                        row.1 = [Tri::Mixed, Tri::On, Tri::Off][(*selected).min(2)];
                    }
                }
                (Slot::Mode, FieldKind::Text { text, .. }) => {
                    if let Some(m) = self.mode.as_mut() {
                        m.octal = text.clone();
                        if text.trim().is_empty() {
                            // Empty: every item keeps its mode.
                            *m = ModeEdit::new(&self.entries).unwrap_or_else(|| m.clone());
                        } else if !m.set_octal(text) {
                            return Err(tr!("tui-ops-bad-mode").into());
                        }
                    }
                }
                _ => {}
            }
        }
        self.times().map(|_| ())
    }

    /// The new times (`None`: no change).
    pub fn times(&self) -> Result<Option<TimesSet>, String> {
        let now = SystemTime::now();
        let mut t = TimesSet::default();
        for i in 0..3 {
            let Some(text) = self.field(Slot::Time(i)).and_then(Field::value) else { continue };
            if text.trim() == self.times0[i].trim() || text.trim().is_empty() {
                continue;
            }
            let v = parse_time(text, now).ok_or_else(|| tr!("tui-ops-bad-time", text = text))?;
            match i {
                0 => t.created = Some(v),
                1 => t.modified = Some(v),
                _ => t.accessed = Some(v),
            }
        }
        Ok((t != TimesSet::default()).then_some(t))
    }

    fn recursive(&self) -> bool {
        self.field(Slot::Recursive).and_then(Field::on).unwrap_or(false)
    }

    /// The jobs for `entries` with the current edits.
    pub fn jobs_for(&self, entries: &[EntryInfo]) -> Vec<MetaJob> {
        let times = self.times().ok().flatten();
        let attrs = self.attrs.as_ref().and_then(AttrEdit::changes);
        plan_jobs(entries, times, attrs, self.mode.as_ref())
    }

    fn has_changes(&self) -> bool {
        self.times().ok().flatten().is_some()
            || self.attrs.as_ref().is_some_and(|a| a.changes().is_some())
            || self.mode.as_ref().is_some_and(ModeEdit::changed)
    }

    /// After an edit: the error and whether Apply is possible.
    pub fn update(&mut self) {
        let r = self.sync();
        self.form.error = r.clone().err();
        self.form.buttons[APPLY].enabled = r.is_ok() && self.has_changes() && self.collecting.is_none();
    }

    /// The current mode of the items, for the body.
    pub fn mode_line(&self) -> Option<String> {
        let m = self.mode.as_ref()?.uniform()?;
        Some(format!("{} ({:o})", mode_string(m), m & 0o7777))
    }

    /// Apply: the command, or the background walk of the recursive option.
    fn apply(&mut self, wake: &Wake) -> Outcome {
        self.update();
        if !self.form.buttons[APPLY].enabled {
            return Outcome::Keep;
        }
        if self.recursive() {
            self.start_collect(wake);
            self.form.buttons[APPLY].enabled = false;
            return Outcome::Keep;
        }
        let jobs = self.jobs_for(&self.entries);
        self.done = true;
        Outcome::Close(Some(Cmd::Meta(MetaCmd::SetMeta(jobs))))
    }

    fn start_collect(&mut self, wake: &Wake) {
        let ctl = OpControl::new();
        let slot = Arc::new(Mutex::new(None));
        let roots = self.entries.clone();
        let (c2, s2, wake) = (ctl.clone(), slot.clone(), wake.clone());
        let _ = std::thread::Builder::new().name("soyuz-attr-walk".into()).spawn(move || {
            let mut all: Vec<EntryInfo> = Vec::new();
            for root in roots {
                if root.kind != EntryKind::Dir {
                    all.push(root);
                    continue;
                }
                let _ = soyuz_fs::walk(&root.path, &WalkOptions::default(), &c2, &mut |item| {
                    if let WalkItem::Entry { entry, .. } = item {
                        all.push(entry);
                    }
                    if c2.is_cancelled() { Control::Stop } else { Control::Continue }
                });
            }
            if !c2.is_cancelled() {
                *s2.lock() = Some(all);
                wake();
            }
        });
        self.collecting = Some((ctl, slot));
    }
}

/// Per frame (`view::prepare`): a finished recursive walk submits its jobs and closes
/// the dialog.
pub fn tick(core: &mut AppCore, st: &mut super::State) {
    for (_, d) in &mut st.tui.stack {
        if let TuiDialog::Attributes(s) = d
            && let Some(all) = s.collecting.as_ref().and_then(|(_, slot)| slot.lock().take())
        {
            let jobs = s.jobs_for(&all);
            core.cmds.push(Cmd::Meta(MetaCmd::SetMeta(jobs)));
            s.collecting = None;
            s.done = true;
        }
    }
}

fn handle(wake: &Wake, s: &mut State, events: &[InputEvent]) -> Outcome {
    for ev in events {
        match s.form.apply(ev) {
            FormOutcome::Cancel => return Outcome::Close(None),
            FormOutcome::Changed(_) => s.update(),
            FormOutcome::Pressed(APPLY) => return s.apply(wake),
            FormOutcome::Pressed(_) => return Outcome::Close(None),
            FormOutcome::Consumed | FormOutcome::Ignored => {}
        }
    }
    Outcome::Keep
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    handle(&core.wake, s, &i.events)
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    match super::batch_rename::form_mouse(&mut s.form, None, e) {
        FormOutcome::Changed(_) => s.update(),
        FormOutcome::Pressed(APPLY) => return s.apply(&core.wake),
        FormOutcome::Pressed(_) => return Outcome::Close(None),
        _ => {}
    }
    Outcome::Keep
}

/// The dialog went away by itself: applied after the recursive walk.
pub fn stale(_core: &AppCore, s: &State) -> bool {
    s.done
}

/// An octal mode field's value, for tests and the view.
pub fn octal_ok(s: &str) -> bool {
    s.trim().is_empty() || parse_octal(s).is_some()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use soyuz_core::Attributes;

    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn entry(name: &str, attrs: Attributes, mode: Option<u32>) -> EntryInfo {
        let mut e = EntryInfo::minimal(PathBuf::from("/x").join(name), EntryKind::File);
        e.attrs = attrs;
        e.mode = mode;
        e
    }

    #[test]
    fn toggling_a_flag_makes_a_job() {
        let wake = soyuz_appcore::host::no_wake();
        let first = editable_attrs()[0].0;
        let mut s = State::new(vec![entry("a", Attributes::empty(), Some(0o644))]);
        assert!(!s.form.buttons[APPLY].enabled);
        assert_eq!(s.form.focused_field(), Some(0));
        assert!(matches!(handle(&wake, &mut s, &[chord("Space")]), Outcome::Keep));
        assert!(s.form.buttons[APPLY].enabled);
        match handle(&wake, &mut s, &[chord("Alt+A")]) {
            Outcome::Close(Some(Cmd::Meta(MetaCmd::SetMeta(jobs)))) => {
                assert_eq!(jobs.len(), 1);
                assert_eq!(jobs[0].attrs.map(|a| a.set), Some(first));
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn mixed_flags_bad_modes_and_times() {
        let first = editable_attrs()[0].0;
        let mut s = State::new(vec![entry("a", first, Some(0o644)), entry("b", Attributes::empty(), Some(0o600))]);
        assert!(matches!(s.form.fields[0].kind, FieldKind::Radio { .. }), "mixed");
        let mode = s.slots.iter().position(|x| *x == Slot::Mode).unwrap();
        assert_eq!(s.form.fields[mode].value(), Some(""), "modes differ");
        s.form.fields[mode].kind = FieldKind::Text { text: "9".into(), edit: Default::default(), secret: false };
        s.update();
        assert!(s.form.error.is_some());
        assert!(!octal_ok("9") && octal_ok("640") && octal_ok(""));
        s.form.fields[mode].kind = FieldKind::Text { text: "640".into(), edit: Default::default(), secret: false };
        s.update();
        assert_eq!(s.form.error, None);
        assert!(s.form.buttons[APPLY].enabled);
        let jobs = s.jobs_for(&s.entries);
        assert!(jobs.iter().all(|j| j.mode.map(|m| m & 0o7777) == Some(0o640)), "{jobs:?}");
        let modified = s.slots.iter().position(|x| *x == Slot::Time(1)).unwrap();
        s.form.fields[modified].kind =
            FieldKind::Text { text: "2024-01-02 03:04".into(), edit: Default::default(), secret: false };
        s.update();
        assert!(s.times().unwrap().is_some_and(|t| t.modified.is_some() && t.accessed.is_none()));
        s.form.fields[modified].kind = FieldKind::Text { text: "soon".into(), edit: Default::default(), secret: false };
        s.update();
        assert!(s.form.error.is_some() && !s.form.buttons[APPLY].enabled);
    }
}
