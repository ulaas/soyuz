//! Batch rename on the `rename` planner (`FrontendCmd::MetaDialog(MetaDialog::BatchRename)`:
//! F2 on a multiple selection, Ctrl+Shift+R) — Phase T3.
//!
//! The state, the preview and its validation are the core's
//! `soyuz_appcore::models::batch_rename::BatchRenameModel` (the GUI draws the same model).
//! The terminal dialog edits a fixed set of rules as form fields, in this order, applied
//! to the part chosen in "Edit" (name, name + extension, extension):
//!
//! | Field | Rule |
//! |---|---|
//! | Find / Replace, Regex, Match case | `Rule::Replace` (when Find is not empty) |
//! | Change case | `Rule::Case` (unless "Keep") |
//! | Numbering, Separator, Start, Digits | `Rule::Number` at the start or end, with the counter |
//!
//! Every change recomputes the live old → new preview (the list; PageUp / PageDown or
//! the wheel scroll it): unchanged rows are muted, duplicates and invalid names red,
//! names that already exist yellow. The rename button is enabled when something changes
//! and nothing conflicts; it submits one undoable `Rename` job (`MetaCmd::BatchRename`).
//! Tokens (`{n}`, `{name}`, `{date:%Y}`, `{exif:%Y%m%d}`, …) work in Find / Replace like in
//! the GUI; EXIF dates are read in the background ([`tick`] picks them up).

use crossterm::event::{MouseButton, MouseEventKind};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Wake;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::batch_rename::BatchRenameModel;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::rename::{CaseMode, Counter, Position, Rule, RuleEntry, Spec, Status, Target};
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::layout::HitTarget;
use crate::view::dialogs::MAX_LIST_ROWS;
use crate::widgets::{Button, Field, Form, FormOutcome, ListMove, SelectList};

/// Field indices.
pub const TARGET: usize = 0;
pub const FIND: usize = 1;
pub const REPLACE: usize = 2;
pub const REGEX: usize = 3;
pub const MATCH_CASE: usize = 4;
pub const CASE: usize = 5;
pub const NUMBER: usize = 6;
pub const SEPARATOR: usize = 7;
pub const START: usize = 8;
pub const DIGITS: usize = 9;

/// The rename button; the other one cancels.
const GO: usize = 0;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    pub form: Form,
    /// The core's batch rename state (`None` only in a default `State`).
    pub model: Option<Box<BatchRenameModel>>,
    /// Cursor and scroll of the preview.
    pub list: SelectList,
}

/// Open the dialog (from `crate::frontend`) on the focused pane's selection (every
/// visible item when nothing is selected).
pub fn open(app: &mut App) {
    match BatchRenameModel::open(&app.core) {
        Ok(m) => {
            let s = State::new(&app.core.wake, m);
            app.push_dialog(TuiDialog::BatchRename(s));
        }
        Err(e) => {
            app.core.ui.toasts.push(Toast::info(e));
        }
    }
}

impl State {
    /// The dialog for `m`, its preview computed.
    pub fn new(wake: &Wake, m: BatchRenameModel) -> State {
        let targets: Vec<String> = Target::ALL.iter().map(|t| t.label().to_string()).collect();
        let cases: Vec<String> = std::iter::once(tr!("tui-ops-keep"))
            .chain(CaseMode::ALL.iter().map(|c| c.label()))
            .map(String::from)
            .collect();
        let at = vec![
            tr!("tui-ops-off").into(),
            tr!("dialogs-rename-pos-start").into(),
            tr!("dialogs-rename-pos-end").into(),
        ];
        let c = Counter::default();
        let form = Form::new(
            vec![
                Field::radio(tr!("dialogs-rename-edit"), targets, 0),
                Field::text(tr!("dialogs-rename-find"), ""),
                Field::text(tr!("dialogs-rename-replace"), ""),
                Field::check(tr!("dialogs-rename-regex"), false),
                Field::check(tr!("dialogs-rename-match-case"), false),
                Field::radio(tr!("dialogs-rename-rule-case"), cases, 0),
                Field::radio(tr!("dialogs-rename-rule-number"), at, 0),
                Field::text(tr!("dialogs-rename-separator"), " "),
                Field::text(tr!("dialogs-rename-counter-start"), c.start.to_string()),
                Field::text(tr!("dialogs-rename-counter-digits"), c.pad.to_string()),
            ],
            vec![Button::new(tr!("common-rename")).accel('r'), Button::new(tr!("common-cancel"))],
        )
        .default_button(GO);
        let mut s = State { title: m.title.clone(), form, model: Some(Box::new(m)), list: SelectList::new() };
        s.update(wake);
        s
    }

    /// The rules the fields describe (`Err`: a counter field is not a whole number).
    pub fn spec(&self) -> Result<Spec, String> {
        let f = &self.form.fields;
        let text = |i: usize| f[i].value().unwrap_or_default().to_string();
        let on = |i: usize| f[i].on().unwrap_or(false);
        let pick = |i: usize| f[i].selected().unwrap_or(0);
        let mut rules = Vec::new();
        let find = text(FIND);
        if !find.is_empty() {
            rules.push(RuleEntry::new(Rule::Replace {
                find,
                with: text(REPLACE),
                regex: on(REGEX),
                case_sensitive: on(MATCH_CASE),
                first_only: false,
            }));
        }
        if let Some(mode) = pick(CASE).checked_sub(1).and_then(|i| CaseMode::ALL.get(i)) {
            rules.push(RuleEntry::new(Rule::Case { mode: *mode }));
        }
        let mut counter = Counter::default();
        if pick(NUMBER) > 0 {
            let at = if pick(NUMBER) == 1 { Position::Start } else { Position::End };
            rules.push(RuleEntry::new(Rule::Number { at, separator: text(SEPARATOR) }));
            let bad = || tr!("tui-ops-bad-counter").to_string();
            counter.start = text(START).trim().parse().map_err(|_| bad())?;
            counter.pad = text(DIGITS).trim().parse::<usize>().ok().filter(|p| *p <= 12).ok_or_else(bad)?;
        }
        let target = Target::ALL.get(pick(TARGET)).copied().unwrap_or_default();
        Ok(Spec { target, counter, rules })
    }

    /// After a change: the spec into the model, the preview, the error and the button.
    pub fn update(&mut self, wake: &Wake) {
        let spec = self.spec();
        let Some(m) = self.model.as_mut() else { return };
        let bad_field = match spec {
            Ok(spec) => {
                m.spec = spec;
                None
            }
            Err(e) => Some(e),
        };
        m.recompute(wake);
        let (ok, _) = m.counts();
        self.form.error = bad_field.clone().or_else(|| m.compile_error.clone());
        let b = &mut self.form.buttons[GO];
        b.label = tr!("dialogs-rename-go", count = ok);
        b.enabled = bad_field.is_none() && m.can_apply();
        self.list.clamp(m.items.len(), usize::from(MAX_LIST_ROWS));
    }
}

/// Per frame (`view::prepare`): EXIF dates that landed update the open dialogs' previews.
pub fn tick(core: &AppCore, st: &mut super::State) {
    for (_, d) in &mut st.tui.stack {
        if let TuiDialog::BatchRename(s) = d
            && s.model.as_ref().is_some_and(|m| m.spec.uses_exif())
        {
            s.update(&core.wake);
        }
    }
}

/// A preview scroll key (PageUp / PageDown, Ctrl+Home / Ctrl+End).
fn list_move(ev: &InputEvent) -> Option<ListMove> {
    let InputEvent::Chord(c) = ev else { return None };
    let ctrl = c.mods.contains(Mods::CTRL);
    match c.key {
        KeyName::PageUp if c.mods.is_empty() => Some(ListMove::PageUp),
        KeyName::PageDown if c.mods.is_empty() => Some(ListMove::PageDown),
        KeyName::Home if ctrl => Some(ListMove::Home),
        KeyName::End if ctrl => Some(ListMove::End),
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    handle(&core.wake, s, &i.events)
}

/// The input events of one key press or paste.
fn handle(wake: &Wake, s: &mut State, events: &[InputEvent]) -> Outcome {
    let len = s.model.as_ref().map_or(0, |m| m.items.len());
    for ev in events {
        match s.form.apply(ev) {
            FormOutcome::Cancel => return Outcome::Close(None),
            FormOutcome::Changed(_) => s.update(wake),
            FormOutcome::Pressed(b) => return press(s, b),
            FormOutcome::Ignored => {
                if let Some(m) = list_move(ev) {
                    let page = usize::from(MAX_LIST_ROWS);
                    s.list.go(m, len, page.saturating_sub(1));
                    s.list.show_cursor(page);
                }
            }
            FormOutcome::Consumed => {}
        }
    }
    Outcome::Keep
}

/// Button `b` was pressed.
fn press(s: &State, b: usize) -> Outcome {
    match &s.model {
        Some(m) if b == GO => {
            if m.can_apply() && s.form.buttons[GO].enabled {
                Outcome::Close(Some(m.apply_cmd()))
            } else {
                Outcome::Keep
            }
        }
        _ => Outcome::Close(None),
    }
}

/// A form's mouse: a click on a field focuses (a checkbox toggles) it, on a button
/// presses it; a click on a list row moves `list`'s cursor; the wheel scrolls `list`.
pub fn form_mouse(form: &mut Form, list: Option<(&mut SelectList, usize)>, m: &DialogMouse) -> FormOutcome {
    match m.event.kind {
        MouseEventKind::Down(MouseButton::Left) => match m.target {
            Some(HitTarget::DialogField(i)) => form.click_field(i),
            Some(HitTarget::DialogButton(i)) => form.click_button(i),
            Some(HitTarget::DialogRow(i)) => {
                if let Some((l, len)) = list {
                    l.select(i, len);
                }
                FormOutcome::Consumed
            }
            _ => FormOutcome::Ignored,
        },
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            if let Some((l, len)) = list {
                let rows = if m.event.kind == MouseEventKind::ScrollUp { -3 } else { 3 };
                l.wheel(rows, len, usize::from(MAX_LIST_ROWS));
            }
            FormOutcome::Consumed
        }
        _ => FormOutcome::Ignored,
    }
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    let len = s.model.as_ref().map_or(0, |m| m.items.len());
    match form_mouse(&mut s.form, Some((&mut s.list, len)), e) {
        FormOutcome::Changed(_) => s.update(&core.wake),
        FormOutcome::Pressed(b) => return press(s, b),
        _ => {}
    }
    Outcome::Keep
}

/// The dialog went away by itself (its pane or files are gone, …).
pub fn stale(core: &AppCore, s: &State) -> bool {
    s.model.as_ref().is_none_or(|m| core.ws.find_pane(m.pane).is_none())
}

/// The short label of a problem in the preview.
pub fn status_label(st: &Status) -> &'static str {
    match st {
        Status::Unchanged | Status::Ok => "",
        Status::Invalid(_) => tr!("tui-ops-status-invalid"),
        Status::Duplicate => tr!("tui-ops-status-duplicate"),
        Status::Exists => tr!("tui-ops-status-exists"),
        Status::Error(_) => tr!("tui-ops-status-error"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use soyuz_appcore::rename::Item;
    use soyuz_appcore::state::pane::PaneId;

    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn state(names: &[&str]) -> (Wake, State) {
        let wake = soyuz_appcore::host::no_wake();
        let dir = PathBuf::from("/tmp/soyuz-br");
        let items: Vec<Item> = names.iter().map(|n| Item::new(dir.join(n), false, None)).collect();
        let known: HashSet<PathBuf> = items.iter().map(|i| i.path.clone()).collect();
        let m = BatchRenameModel::new(PaneId(1), items, known, Some(dir), true);
        let s = State::new(&wake, m);
        (wake, s)
    }

    fn input(events: Vec<InputEvent>) -> Vec<InputEvent> {
        events
    }

    #[test]
    fn numbering_fields_drive_the_preview() {
        let (wake, mut s) = state(&["a.txt", "b.txt"]);
        assert!(!s.form.buttons[GO].enabled, "nothing changes yet");
        s.form.focus = NUMBER;
        let _ = handle(&wake, &mut s, &input(vec![chord("Right"), chord("Right")]));
        let rows = &s.model.as_ref().unwrap().rows;
        assert_eq!(rows[0].new_name, "a 01.txt");
        assert_eq!(rows[1].new_name, "b 02.txt");
        assert!(s.form.buttons[GO].enabled);
        // A bad counter blocks the rename with a message.
        s.form.focus = START;
        let _ = handle(&wake, &mut s, &input(vec![InputEvent::Text("x".into())]));
        assert!(s.form.error.is_some());
        assert!(!s.form.buttons[GO].enabled);
        assert!(matches!(handle(&wake, &mut s, &input(vec![chord("Enter")])), Outcome::Keep));
        let _ = handle(&wake, &mut s, &input(vec![chord("Backspace")]));
        assert!(s.form.error.is_none());
        match handle(&wake, &mut s, &input(vec![chord("Enter")])) {
            Outcome::Close(Some(soyuz_appcore::cmd::Cmd::Meta(_))) => {}
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn duplicates_are_flagged() {
        let (wake, mut s) = state(&["a1.txt", "a2.txt", "b.txt"]);
        s.form.focus = FIND;
        let _ = handle(&wake, &mut s, &input(vec![InputEvent::Text("[0-9]".into())]));
        s.form.fields[REGEX].kind = crate::widgets::FieldKind::Check(true);
        s.update(&wake);
        let rows = &s.model.as_ref().unwrap().rows;
        assert_eq!(rows[0].status, Status::Duplicate);
        assert_eq!(rows[2].status, Status::Unchanged);
        assert_eq!(status_label(&rows[0].status), tr!("tui-ops-status-duplicate"));
        assert!(!s.form.buttons[GO].enabled);
        // PageDown scrolls the preview.
        let _ = handle(&wake, &mut s, &input(vec![chord("PageDown")]));
        assert_eq!(s.list.cursor, 2);
        assert!(matches!(handle(&wake, &mut s, &input(vec![chord("Esc")])), Outcome::Close(None)));
    }
}
