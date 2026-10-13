//! The dialog for creating, editing, renaming and deleting a sync profile with its
//! schedule (`TuiDialog::CompareProfile`), opened from the compare screen's profiles page
//! (`crate::input::compare`).
//!
//! - New / Edit: a form with the name, the left and right folders (with folder completion
//!   like the address bar: Up / Down pick a name, Tab takes it while a partial name is
//!   typed, a click on a completion takes it), the compare method, the sync mode, the
//!   schedule kind and its fields (minutes between runs for "Every…", the time of day for
//!   "Daily"; the field of another kind is disabled). Save validates and stores through
//!   `models::compare_profiles` (`create_profile` / `update_profile`, which also renames);
//!   a refused edit shows its reason under the fields and focuses the field to fix.
//! - Rename: the new name only (`rename_profile`; its reports move along).
//! - Delete: a confirmation (`delete_profile`); refused while the profile runs.
//!
//! Dialog callbacks only get the core, so the name to select afterwards goes to the
//! compare screen through a shared slot ([`crate::input::compare::Select`]).
//!
//! Keys: the form's (Tab / Shift+Tab, Left / Right pick an option, Enter saves, Esc
//! cancels), Alt+S saves from anywhere; in the delete confirmation D (or Enter) deletes.
//! Mouse: a click focuses a field, a click on a focused option row picks the next option,
//! a click on a button presses it.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::compare::{Method, Preset};
use soyuz_appcore::models::compare_profiles::{
    self, DAILY_HOURS, DAILY_MINUTES, INTERVAL_MINUTES, ProfileError, Schedule, ScheduleKind, SyncProfile,
};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::state::crumbs;
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::input::compare::Select;
use crate::layout::HitTarget;
use crate::widgets::form::FieldKind;
use crate::widgets::{Button, Field, Form, FormOutcome, LineEdit};

/// Field indexes of the New / Edit form.
pub mod field {
    pub const NAME: usize = 0;
    pub const LEFT: usize = 1;
    pub const RIGHT: usize = 2;
    pub const METHOD: usize = 3;
    pub const PRESET: usize = 4;
    pub const SCHEDULE: usize = 5;
    pub const MINUTES: usize = 6;
    pub const TIME: usize = 7;
}

/// What the dialog does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A new profile.
    New,
    /// Edit the profile of this name (every field, the name too).
    Edit(String),
    /// Rename the profile of this name.
    Rename(String),
    /// Delete the profile of this name (a confirmation).
    Delete(String),
}

/// What the dialog keeps while it is open.
#[derive(Default)]
pub struct State {
    pub mode: Option<Mode>,
    pub form: Form,
    /// The highlighted folder completion of the focused folder field.
    pub pick: usize,
    /// Where the name to select afterwards goes (the compare screen's profiles page).
    pub select: Option<Select>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State").field("mode", &self.mode).field("focus", &self.form.focus).finish_non_exhaustive()
    }
}

fn kind_index(k: ScheduleKind) -> usize {
    ScheduleKind::ALL.iter().position(|x| *x == k).unwrap_or(0)
}

/// The sync modes offered (`Preset::ALL`; a hand-edited "Custom" saves as "Update").
fn preset_index(p: Preset) -> usize {
    Preset::ALL
        .iter()
        .position(|x| *x == p)
        .or_else(|| Preset::ALL.iter().position(|x| *x == Preset::Update))
        .unwrap_or(0)
}

impl State {
    /// A new profile, prefilled with `p` (the compare setup, or blank).
    pub fn new_profile(p: SyncProfile, select: Option<Select>) -> State {
        State::form_state(Mode::New, &p, select)
    }

    /// Edit `p` (stored as `name`; `p` may carry other values, e.g. the compare setup).
    pub fn edit(name: &str, p: &SyncProfile, select: Option<Select>) -> State {
        State::form_state(Mode::Edit(name.to_string()), p, select)
    }

    /// Rename the profile `name`.
    pub fn rename(name: &str, select: Option<Select>) -> State {
        let form = Form::new(
            vec![Field::text(tr!("tui-cmpprof-new-name"), name)],
            vec![Button::new(tr!("common-rename")), Button::new(tr!("common-cancel"))],
        );
        State { mode: Some(Mode::Rename(name.to_string())), form, pick: 0, select }
    }

    /// Ask before deleting the profile `name`.
    pub fn delete(name: &str) -> State {
        let mut form = Form::new(
            Vec::new(),
            vec![Button::new(tr!("common-delete")).accel('d'), Button::new(tr!("common-cancel"))],
        );
        form.focus_button(0);
        State { mode: Some(Mode::Delete(name.to_string())), form, pick: 0, select: None }
    }

    fn form_state(mode: Mode, p: &SyncProfile, select: Option<Select>) -> State {
        let minutes = match p.schedule {
            Schedule::Interval { minutes } => minutes,
            _ => 60,
        };
        let (hour, minute) = match p.schedule {
            Schedule::Daily { hour, minute } => (hour, minute),
            _ => (18, 0),
        };
        let fields = vec![
            Field::text(tr!("tools-profiles-name-hint"), p.name.clone()),
            Field::text(tr!("tools-compare-left-hint"), p.left.to_string_lossy()),
            Field::text(tr!("tools-compare-right-hint"), p.right.to_string_lossy()),
            Field::radio(
                tr!("tools-compare-by"),
                Method::ALL.iter().map(|m| m.label().to_string()).collect(),
                Method::ALL.iter().position(|m| *m == p.method).unwrap_or(0),
            ),
            Field::radio(
                tr!("tools-compare-sync-mode"),
                Preset::ALL.iter().map(|m| m.label().to_string()).collect(),
                preset_index(p.preset),
            ),
            Field::radio(
                tr!("tools-sched-label"),
                ScheduleKind::ALL.iter().map(|k| k.label().to_string()).collect(),
                kind_index(p.schedule.kind()),
            ),
            Field::text(tr!("tui-cmpprof-every"), minutes.to_string()),
            Field::text(tr!("tui-cmpprof-at"), format!("{hour:02}:{minute:02}")),
        ];
        let form = Form::new(fields, vec![Button::new(tr!("common-save")), Button::new(tr!("common-cancel"))]);
        let mut s = State { mode: Some(mode), form, pick: 0, select };
        s.refresh_schedule();
        s
    }

    /// The schedule kind picked.
    pub fn kind(&self) -> ScheduleKind {
        let i = self.form.fields.get(field::SCHEDULE).and_then(Field::selected).unwrap_or(0);
        ScheduleKind::ALL.get(i).copied().unwrap_or(ScheduleKind::Manual)
    }

    /// Only the picked kind's own field is enabled.
    fn refresh_schedule(&mut self) {
        let k = self.kind();
        if let Some(f) = self.form.fields.get_mut(field::MINUTES) {
            f.enabled = k == ScheduleKind::Interval;
        }
        if let Some(f) = self.form.fields.get_mut(field::TIME) {
            f.enabled = k == ScheduleKind::Daily;
        }
    }

    fn text(&self, i: usize) -> &str {
        self.form.fields.get(i).and_then(Field::value).unwrap_or_default()
    }

    /// The focused field is a folder field (left or right) of the New / Edit form.
    pub fn folder_focus(&self) -> Option<usize> {
        matches!(self.mode, Some(Mode::New | Mode::Edit(_)))
            .then(|| self.form.focused_field())
            .flatten()
            .filter(|i| matches!(*i, field::LEFT | field::RIGHT))
    }

    /// The profile the form describes (folders and name as typed; `validate` trims them).
    pub fn profile(&self) -> Result<SyncProfile, (usize, String)> {
        let pick = |i: usize| self.form.fields.get(i).and_then(Field::selected).unwrap_or(0);
        let schedule = match self.kind() {
            ScheduleKind::Interval => Schedule::Interval { minutes: parse_minutes(self.text(field::MINUTES))? },
            ScheduleKind::Daily => {
                let (hour, minute) = parse_time(self.text(field::TIME))?;
                Schedule::Daily { hour, minute }
            }
            k => k.default_schedule(),
        };
        Ok(SyncProfile {
            name: self.text(field::NAME).to_string(),
            left: self.text(field::LEFT).trim().into(),
            right: self.text(field::RIGHT).trim().into(),
            method: Method::ALL.get(pick(field::METHOD)).copied().unwrap_or_default(),
            preset: Preset::ALL.get(pick(field::PRESET)).copied().unwrap_or_default(),
            schedule,
        })
    }
}

/// Minutes between runs ([`INTERVAL_MINUTES`]).
pub fn parse_minutes(s: &str) -> Result<u32, (usize, String)> {
    let err = || {
        (field::MINUTES, tr!("tui-cmpprof-err-minutes", min = *INTERVAL_MINUTES.start(), max = *INTERVAL_MINUTES.end()))
    };
    let n: u32 = s.trim().parse().map_err(|_| err())?;
    if INTERVAL_MINUTES.contains(&n) { Ok(n) } else { Err(err()) }
}

/// A time of day: `H:MM`, `HH:MM`, `HH.MM` or a bare hour.
pub fn parse_time(s: &str) -> Result<(u8, u8), (usize, String)> {
    let err = || (field::TIME, tr!("tui-cmpprof-err-time").to_string());
    let s = s.trim();
    let (h, m) = match s.split_once([':', '.']) {
        Some((h, m)) if m.len() == 2 => (h, m),
        Some(_) => return Err(err()),
        None => (s, "0"),
    };
    let h: u8 = h.trim().parse().map_err(|_| err())?;
    let m: u8 = m.parse().map_err(|_| err())?;
    if DAILY_HOURS.contains(&h) && DAILY_MINUTES.contains(&m) { Ok((h, m)) } else { Err(err()) }
}

/// The message of a refused edit and the field to fix.
pub fn error_text(e: &ProfileError) -> (usize, String) {
    match e {
        ProfileError::EmptyName => (field::NAME, tr!("tui-cmpprof-err-empty-name").to_string()),
        ProfileError::Duplicate(n) => (field::NAME, tr!("tui-cmpprof-err-duplicate", name = n.clone())),
        ProfileError::NotFound(n) => (field::NAME, tr!("tui-cmpprof-err-not-found", name = n.clone())),
        ProfileError::MissingPath => (field::LEFT, tr!("tui-cmpprof-err-missing-path").to_string()),
        ProfileError::SamePaths => (field::RIGHT, tr!("tui-cmpprof-err-same-paths").to_string()),
        ProfileError::Running(n) => (field::NAME, tr!("tui-cmpprof-err-running", name = n.clone())),
    }
}

/// Open the dialog.
pub fn open(app: &mut App, s: State) {
    app.push_dialog(TuiDialog::CompareProfile(s));
}

/// Set text field `i` to `text`, the cursor at its end.
fn set_text(form: &mut Form, i: usize, text: String) {
    if let Some(f) = form.fields.get_mut(i)
        && let FieldKind::Text { text: t, edit, .. } = &mut f.kind
    {
        *edit = LineEdit::at_end(&text);
        *t = text;
    }
}

/// The folder completions of the focused folder field (requests the listing).
fn completions(core: &mut AppCore, s: &State) -> crumbs::Completion {
    match s.folder_focus() {
        Some(i) => crumbs::completion(&mut core.services, s.text(i)),
        None => crumbs::Completion::default(),
    }
}

/// Take completion `k` into the focused folder field.
fn accept(core: &mut AppCore, s: &mut State, k: usize) {
    let Some(i) = s.folder_focus() else { return };
    let c = completions(core, s);
    let Some(name) = c.names.get(k) else { return };
    let mut text = s.text(i).to_string();
    crumbs::accept(&mut text, name);
    set_text(&mut s.form, i, text);
    s.pick = 0;
}

/// Store what the dialog says; `Ok` closes it.
fn submit(core: &mut AppCore, s: &mut State) -> Result<(), (usize, String)> {
    let Some(mode) = s.mode.clone() else { return Ok(()) };
    let (toast, name) = match &mode {
        Mode::New | Mode::Edit(_) => {
            let p = s.profile()?;
            let name = p.name.trim().to_string();
            match &mode {
                Mode::Edit(old) => {
                    compare_profiles::update_profile(old, p).map_err(|e| error_text(&e))?;
                    (tr!("tui-cmpprof-toast-updated", name = name.clone()), name)
                }
                _ => {
                    compare_profiles::create_profile(p).map_err(|e| error_text(&e))?;
                    (tr!("tui-cmpprof-toast-created", name = name.clone()), name)
                }
            }
        }
        Mode::Rename(old) => {
            let new = s.text(0).trim().to_string();
            if new != *old {
                compare_profiles::rename_profile(old, &new).map_err(|e| (0, error_text(&e).1))?;
            }
            (tr!("tui-cmpprof-toast-renamed", name = new.clone()), new)
        }
        Mode::Delete(name) => {
            compare_profiles::delete_profile(name).map_err(|e| (0, error_text(&e).1))?;
            (tr!("tui-cmpprof-toast-deleted", name = name.clone()), String::new())
        }
    };
    core.ui.toasts.push(Toast::info(toast));
    if let Some(sel) = &s.select
        && !name.is_empty()
    {
        *sel.borrow_mut() = Some(name);
    }
    Ok(())
}

fn press(core: &mut AppCore, s: &mut State, b: usize) -> Outcome {
    if b != 0 {
        return Outcome::Close(None);
    }
    match submit(core, s) {
        Ok(()) => Outcome::Close(None),
        Err((i, msg)) => {
            s.form.error = Some(msg);
            if s.form.fields.get(i).is_some_and(|f| f.enabled) {
                s.form.focus = i;
            }
            Outcome::Keep
        }
    }
}

fn on_form(core: &mut AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(b) => Some(press(core, s, b)),
        FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(i) => {
            s.form.error = None;
            if i == field::SCHEDULE && matches!(s.mode, Some(Mode::New | Mode::Edit(_))) {
                s.refresh_schedule();
            }
            if matches!(i, field::LEFT | field::RIGHT) {
                s.pick = 0;
            }
            None
        }
        FormOutcome::Consumed | FormOutcome::Ignored => None,
    }
}

/// Up / Down / Tab over the completions of the focused folder field; returns whether the
/// key was used.
fn completion_key(core: &mut AppCore, s: &mut State, ev: &InputEvent) -> bool {
    let InputEvent::Chord(c) = ev else { return false };
    if c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT) || s.folder_focus().is_none() {
        return false;
    }
    let comp = completions(core, s);
    let n = comp.names.len();
    if n == 0 {
        return false;
    }
    match c.key {
        KeyName::ArrowDown => s.pick = (s.pick + 1) % n,
        KeyName::ArrowUp => s.pick = (s.pick + n - 1) % n,
        KeyName::Tab if !comp.prefix.is_empty() => {
            let k = s.pick.min(n - 1);
            accept(core, s, k);
        }
        _ => return false,
    }
    true
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        // Alt+S saves from any field (the plain letter types in a text field).
        if let InputEvent::Chord(c) = ev
            && c.key == KeyName::S
            && c.mods.contains(Mods::ALT)
            && !c.mods.intersects(Mods::CTRL | Mods::CMD)
            && matches!(s.mode, Some(Mode::New | Mode::Edit(_)))
        {
            return press(core, s, 0);
        }
        if completion_key(core, s, ev) {
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = on_form(core, s, o) {
            return out;
        }
    }
    // Ask for the listing of the folder typed so far (the view shows it when it is in).
    let _ = completions(core, s);
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    use crossterm::event::{MouseButton, MouseEventKind};
    if let MouseEventKind::Down(MouseButton::Left) = m.event.kind {
        match m.target {
            // A completion of the focused folder field.
            Some(HitTarget::DialogRow(k)) => {
                accept(core, s, k);
                return Outcome::Keep;
            }
            // A click on a focused option row picks the next option.
            Some(HitTarget::DialogField(f)) if s.form.focus == f => {
                if let Some(FieldKind::Radio { options, selected }) = s.form.fields.get_mut(f).map(|x| &mut x.kind) {
                    *selected = (*selected + 1) % options.len().max(1);
                    return on_form(core, s, FormOutcome::Changed(f)).unwrap_or(Outcome::Keep);
                }
            }
            _ => {}
        }
    }
    let o = super::form_mouse(&mut s.form, None, m);
    let out = on_form(core, s, o).unwrap_or(Outcome::Keep);
    if matches!(out, Outcome::Keep) {
        let _ = completions(core, s);
    }
    out
}

/// The dialog went away by itself: the profile it edits, renames or deletes is gone.
pub fn stale(_core: &AppCore, s: &State) -> bool {
    match &s.mode {
        None => true,
        Some(Mode::New) => false,
        Some(Mode::Edit(n) | Mode::Rename(n) | Mode::Delete(n)) => compare_profiles::profile(n).is_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_minutes_parse_inside_their_ranges() {
        assert_eq!(parse_time("18:30"), Ok((18, 30)));
        assert_eq!(parse_time(" 9:05 "), Ok((9, 5)));
        assert_eq!(parse_time("7"), Ok((7, 0)));
        assert_eq!(parse_time("07.45"), Ok((7, 45)));
        for bad in ["24:00", "12:60", "12:5", "x", "", "-1:00"] {
            assert_eq!(parse_time(bad).map_err(|e| e.0), Err(field::TIME), "{bad}");
        }
        assert_eq!(parse_minutes("90"), Ok(90));
        for bad in ["0", "1441", "1.5", "", "abc"] {
            assert_eq!(parse_minutes(bad).map_err(|e| e.0), Err(field::MINUTES), "{bad}");
        }
    }

    #[test]
    fn the_form_round_trips_a_profile_and_follows_the_schedule_kind() {
        let p = SyncProfile {
            name: "Photos".into(),
            left: "/a".into(),
            right: "/b".into(),
            method: Method::Content,
            preset: Preset::MirrorRight,
            schedule: Schedule::Daily { hour: 7, minute: 15 },
        };
        let mut s = State::edit("Photos", &p, None);
        assert_eq!(s.profile(), Ok(p.clone()));
        assert!(s.form.fields[field::TIME].enabled && !s.form.fields[field::MINUTES].enabled);
        // Pick "Every…": the minutes field turns on, the time field off.
        if let FieldKind::Radio { selected, .. } = &mut s.form.fields[field::SCHEDULE].kind {
            *selected = kind_index(ScheduleKind::Interval);
        }
        s.refresh_schedule();
        assert!(s.form.fields[field::MINUTES].enabled && !s.form.fields[field::TIME].enabled);
        assert_eq!(s.profile().map(|p| p.schedule), Ok(Schedule::Interval { minutes: 60 }));
        set_text(&mut s.form, field::MINUTES, "0".into());
        assert_eq!(s.profile().map_err(|e| e.0), Err(field::MINUTES));
        // A hand-edited ("Custom") mode is offered as "Update".
        let custom = SyncProfile { preset: Preset::Custom, ..p };
        assert_eq!(State::new_profile(custom, None).profile().map(|p| p.preset), Ok(Preset::Update));
    }

    #[test]
    fn refused_edits_name_the_field_to_fix() {
        assert_eq!(error_text(&ProfileError::EmptyName).0, field::NAME);
        assert_eq!(error_text(&ProfileError::Duplicate("a".into())).0, field::NAME);
        assert_eq!(error_text(&ProfileError::MissingPath).0, field::LEFT);
        assert_eq!(error_text(&ProfileError::SamePaths).0, field::RIGHT);
        assert!(error_text(&ProfileError::Running("a".into())).1.contains('a'));
    }
}
