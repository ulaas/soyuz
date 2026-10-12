//! Batch rename on the `rename` planner (`FrontendCmd::MetaDialog(MetaDialog::BatchRename)`:
//! F2 on a multiple selection, Ctrl+Shift+R) — Phase T3.
//!
//! The state, the preview and its validation are the core's
//! `soyuz_appcore::models::batch_rename::BatchRenameModel` (the GUI draws the same model).
//! The terminal dialog has the GUI's two modes ("Mode", the first field):
//!
//! - **Rules**: "Edit" (name, extension, both), then "Rule" picks the page of fields
//!   shown below it — one page per rule kind the core supports, applied in this fixed
//!   order when they are set (a `*` marks the pages in use):
//!
//!   | Page | Fields | Rule |
//!   |---|---|---|
//!   | Template | Pattern (`{name}_{n}`; empty: off) | `Rule::Template` |
//!   | Trim | Trim spaces, collapse repeated spaces | `Rule::Trim` |
//!   | Replace | Find, Replace, Regex, Match case, First only | `Rule::Replace` (when Find is set) |
//!   | Remove | Remove (count, 0: off), starting at, from the end | `Rule::Remove` |
//!   | Insert | Text, Position (start / end / at), index | `Rule::Insert` (when Text is set) |
//!   | Case | Change case | `Rule::Case` (unless "Keep") |
//!   | Numbering | at start / end, Separator, Start, Step, Digits, Per folder | `Rule::Number` and the counter |
//!   | Presets | the presets in the list (PageUp / PageDown or a click pick one), Load, a name and Save, Delete | `use_preset` / `save_preset` / `delete_preset` |
//!
//!   Loading a preset fills the pages from its rules (a rule kind used twice keeps the
//!   last one).
//! - **One name per line** (`Mode::Editor`): the list shows the items; "Name" edits the
//!   line of the list's cursor row (Up / Down in the field and PageUp / PageDown move the
//!   cursor; a paste of several lines fills the lines from the cursor on). "Reset names"
//!   starts again from the current names.
//!
//! Every change recomputes the live old → new preview (the list; PageUp / PageDown or
//! the wheel scroll it): unchanged rows are muted, duplicates and invalid names red,
//! names that already exist yellow. The rename button is enabled when something changes
//! and nothing conflicts; it submits one undoable `Rename` job (`MetaCmd::BatchRename`).
//! Tokens (`{n}`, `{name}`, `{date:%Y}`, `{exif:%Y%m%d}`, …) work like in the GUI; EXIF
//! dates are read in the background ([`tick`] picks them up).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Wake;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::batch_rename::{BatchRenameModel, Mode};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::rename::presets::Preset;
use soyuz_appcore::rename::{CaseMode, Counter, Position, Rule, RuleEntry, Spec, Status, Target};
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::view::dialogs::MAX_LIST_ROWS;
use crate::widgets::{Button, Field, FieldKind, Form, FormOutcome, ListMove, SelectList};

/// A field of the form (the form's fields follow the mode and the page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum F {
    Mode,
    Target,
    Page,
    Template,
    Trim,
    Collapse,
    Find,
    With,
    Regex,
    MatchCase,
    FirstOnly,
    RemCount,
    RemStart,
    RemFromEnd,
    InsText,
    InsAt,
    InsIndex,
    Case,
    NumAt,
    Separator,
    Start,
    Step,
    Digits,
    PerFolder,
    PresetName,
    /// The editor line of the list's cursor row.
    Name,
}

/// A page of rule fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Template,
    Trim,
    Replace,
    Remove,
    Insert,
    Case,
    Number,
    Presets,
}

impl Page {
    pub const ALL: [Page; 8] = [
        Page::Template,
        Page::Trim,
        Page::Replace,
        Page::Remove,
        Page::Insert,
        Page::Case,
        Page::Number,
        Page::Presets,
    ];

    fn label(self) -> &'static str {
        match self {
            Page::Template => tr!("dialogs-rename-rule-template"),
            Page::Trim => tr!("dialogs-rename-rule-trim"),
            Page::Replace => tr!("dialogs-rename-rule-replace"),
            Page::Remove => tr!("dialogs-rename-rule-remove"),
            Page::Insert => tr!("dialogs-rename-rule-insert"),
            Page::Case => tr!("dialogs-rename-rule-case"),
            Page::Number => tr!("dialogs-rename-rule-number"),
            Page::Presets => tr!("dialogs-rename-presets"),
        }
    }

    /// The fields of the page.
    fn fields(self) -> &'static [F] {
        match self {
            Page::Template => &[F::Template],
            Page::Trim => &[F::Trim, F::Collapse],
            Page::Replace => &[F::Find, F::With, F::Regex, F::MatchCase, F::FirstOnly],
            Page::Remove => &[F::RemCount, F::RemStart, F::RemFromEnd],
            Page::Insert => &[F::InsText, F::InsAt, F::InsIndex],
            Page::Case => &[F::Case],
            Page::Number => &[F::NumAt, F::Separator, F::Start, F::Step, F::Digits, F::PerFolder],
            Page::Presets => &[F::PresetName],
        }
    }
}

/// A button (the buttons follow the mode and the page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Go,
    Cancel,
    Load,
    SavePreset,
    DeletePreset,
    ResetNames,
}

/// Everything the rule pages hold, kept while other pages are shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Values {
    pub target: usize,
    pub template: String,
    pub trim: bool,
    pub collapse: bool,
    pub find: String,
    pub with: String,
    pub regex: bool,
    pub match_case: bool,
    pub first_only: bool,
    pub rem_count: String,
    pub rem_start: String,
    pub rem_from_end: bool,
    pub ins_text: String,
    /// 0: start, 1: end, 2: at [`Values::ins_index`].
    pub ins_at: usize,
    pub ins_index: String,
    /// 0: keep, else `CaseMode::ALL[case - 1]`.
    pub case: usize,
    /// 0: off, 1: start, 2: end.
    pub num_at: usize,
    pub separator: String,
    pub start: String,
    pub step: String,
    pub digits: String,
    pub per_folder: bool,
    pub preset_name: String,
}

impl Default for Values {
    fn default() -> Values {
        let c = Counter::default();
        Values {
            target: 0,
            template: String::new(),
            trim: false,
            collapse: true,
            find: String::new(),
            with: String::new(),
            regex: false,
            match_case: false,
            first_only: false,
            rem_count: "0".into(),
            rem_start: "0".into(),
            rem_from_end: false,
            ins_text: String::new(),
            ins_at: 0,
            ins_index: "0".into(),
            case: 0,
            num_at: 0,
            separator: " ".into(),
            start: c.start.to_string(),
            step: c.step.to_string(),
            digits: c.pad.to_string(),
            per_folder: false,
            preset_name: String::new(),
        }
    }
}

impl Values {
    /// The rules the values describe (`Err`: a number field is not a number).
    pub fn spec(&self) -> Result<Spec, String> {
        let mut rules = Vec::new();
        if !self.template.is_empty() {
            rules.push(Rule::Template { pattern: self.template.clone() });
        }
        if self.trim {
            rules.push(Rule::Trim { collapse: self.collapse });
        }
        if !self.find.is_empty() {
            rules.push(Rule::Replace {
                find: self.find.clone(),
                with: self.with.clone(),
                regex: self.regex,
                case_sensitive: self.match_case,
                first_only: self.first_only,
            });
        }
        let bad = || tr!("tui-ops-bad-number").to_string();
        let count: usize = self.rem_count.trim().parse().map_err(|_| bad())?;
        if count > 0 {
            let start = self.rem_start.trim().parse().map_err(|_| bad())?;
            rules.push(Rule::Remove { start, count, from_end: self.rem_from_end });
        }
        if !self.ins_text.is_empty() {
            let at = match self.ins_at {
                0 => Position::Start,
                1 => Position::End,
                _ => Position::Index(self.ins_index.trim().parse().map_err(|_| bad())?),
            };
            rules.push(Rule::Insert { text: self.ins_text.clone(), at });
        }
        if let Some(mode) = self.case.checked_sub(1).and_then(|i| CaseMode::ALL.get(i)) {
            rules.push(Rule::Case { mode: *mode });
        }
        let mut counter = Counter::default();
        if self.num_at > 0 {
            let at = if self.num_at == 1 { Position::Start } else { Position::End };
            rules.push(Rule::Number { at, separator: self.separator.clone() });
            let bad = || tr!("tui-ops-bad-counter").to_string();
            counter.start = self.start.trim().parse().map_err(|_| bad())?;
            counter.step = self.step.trim().parse().map_err(|_| bad())?;
            counter.pad = self.digits.trim().parse::<usize>().ok().filter(|p| *p <= 12).ok_or_else(bad)?;
            counter.per_folder = self.per_folder;
        }
        let target = Target::ALL.get(self.target).copied().unwrap_or_default();
        Ok(Spec { target, counter, rules: rules.into_iter().map(RuleEntry::new).collect() })
    }

    /// The values of `spec` (a preset): one slot per rule kind, a kind used twice keeps
    /// the last one; disabled rules are left out.
    pub fn from_spec(spec: &Spec, keep_name: &str) -> Values {
        let mut v = Values {
            target: Target::ALL.iter().position(|t| *t == spec.target).unwrap_or(0),
            preset_name: keep_name.to_string(),
            ..Values::default()
        };
        for r in spec.rules.iter().filter(|r| r.enabled) {
            match &r.rule {
                Rule::Template { pattern } => v.template = pattern.clone(),
                Rule::Trim { collapse } => (v.trim, v.collapse) = (true, *collapse),
                Rule::Replace { find, with, regex, case_sensitive, first_only } => {
                    (v.find, v.with) = (find.clone(), with.clone());
                    (v.regex, v.match_case, v.first_only) = (*regex, *case_sensitive, *first_only);
                }
                Rule::Remove { start, count, from_end } => {
                    (v.rem_start, v.rem_count, v.rem_from_end) = (start.to_string(), count.to_string(), *from_end);
                }
                Rule::Insert { text, at } => {
                    v.ins_text = text.clone();
                    (v.ins_at, v.ins_index) = match at {
                        Position::Start => (0, "0".into()),
                        Position::End => (1, "0".into()),
                        Position::Index(i) => (2, i.to_string()),
                    };
                }
                Rule::Case { mode } => v.case = CaseMode::ALL.iter().position(|m| m == mode).map_or(0, |i| i + 1),
                Rule::Number { at, separator } => {
                    v.num_at = if *at == Position::Start { 1 } else { 2 };
                    v.separator = separator.clone();
                    v.start = spec.counter.start.to_string();
                    v.step = spec.counter.step.to_string();
                    v.digits = spec.counter.pad.to_string();
                    v.per_folder = spec.counter.per_folder;
                }
            }
        }
        v
    }

    /// Whether `page`'s rule is in use.
    fn in_use(&self, page: Page) -> bool {
        match page {
            Page::Template => !self.template.is_empty(),
            Page::Trim => self.trim,
            Page::Replace => !self.find.is_empty(),
            Page::Remove => self.rem_count.trim().parse::<usize>().is_ok_and(|c| c > 0),
            Page::Insert => !self.ins_text.is_empty(),
            Page::Case => self.case > 0,
            Page::Number => self.num_at > 0,
            Page::Presets => false,
        }
    }
}

/// What the dialog keeps while it is open.
#[derive(Debug)]
pub struct State {
    pub title: String,
    pub form: Form,
    /// The core's batch rename state (`None` only in a default `State`).
    pub model: Option<Box<BatchRenameModel>>,
    /// Cursor and scroll of the list (the preview, or the presets on that page).
    pub list: SelectList,
    pub values: Values,
    pub page: Page,
    /// What each of the form's fields and buttons is.
    pub fmap: Vec<F>,
    pub bmap: Vec<Btn>,
}

impl Default for State {
    fn default() -> State {
        State {
            title: String::new(),
            form: Form::default(),
            model: None,
            list: SelectList::new(),
            values: Values::default(),
            page: Page::Replace,
            fmap: Vec::new(),
            bmap: Vec::new(),
        }
    }
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

fn radio(label: &str, options: Vec<String>, selected: usize) -> Field {
    Field::radio(label, options, selected)
}

impl State {
    /// The dialog for `m`, its preview computed; the focus on Find.
    pub fn new(wake: &Wake, m: BatchRenameModel) -> State {
        let mut s = State { title: m.title.clone(), model: Some(Box::new(m)), ..State::default() };
        s.rebuild(Some(F::Find));
        s.update(wake);
        s
    }

    fn mode(&self) -> Mode {
        self.model.as_ref().map_or(Mode::Rules, |m| m.mode)
    }

    /// The presets: built-in, then saved (`(label, preset, saved index)`).
    pub fn presets(&self) -> Vec<(String, Preset, Option<usize>)> {
        let Some(m) = &self.model else { return Vec::new() };
        let b =
            m.builtin.iter().map(|p| (format!("{} \u{b7} {}", tr!("dialogs-rename-builtin"), p.name), p.clone(), None));
        let u = m
            .user
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("{} \u{b7} {}", tr!("dialogs-rename-saved"), p.name), p.clone(), Some(i)));
        b.chain(u).collect()
    }

    /// Rows in the list: the presets on that page, else one per item.
    pub fn list_len(&self) -> usize {
        if self.mode() == Mode::Rules && self.page == Page::Presets {
            self.presets().len()
        } else {
            self.model.as_ref().map_or(0, |m| m.items.len())
        }
    }

    /// The field of `f`, if shown.
    pub fn idx(&self, f: F) -> Option<usize> {
        self.fmap.iter().position(|x| *x == f)
    }

    /// The button of `b`, if shown.
    pub fn bidx(&self, b: Btn) -> Option<usize> {
        self.bmap.iter().position(|x| *x == b)
    }

    /// The form for the mode and the page, from the values; the focus on `focus` (or the
    /// same field kind as before).
    pub fn rebuild(&mut self, focus: Option<F>) {
        let keep = focus.or_else(|| self.form.focused_field().and_then(|i| self.fmap.get(i).copied()));
        let v = &self.values;
        let mode = self.mode();
        let mut fmap = vec![F::Mode];
        if mode == Mode::Rules {
            fmap.extend([F::Target, F::Page]);
            fmap.extend_from_slice(self.page.fields());
        } else {
            fmap.push(F::Name);
        }
        let name_line = self.editor_line(self.list.cursor);
        let fields: Vec<Field> = fmap
            .iter()
            .map(|f| match f {
                F::Mode => radio(
                    tr!("tui-ops-rename-mode"),
                    vec![tr!("dialogs-rename-rules").into(), tr!("dialogs-rename-one-per-line").into()],
                    usize::from(mode == Mode::Editor),
                ),
                F::Target => radio(
                    tr!("dialogs-rename-edit"),
                    Target::ALL.iter().map(|t| t.label().to_string()).collect(),
                    v.target,
                ),
                F::Page => radio(
                    tr!("tui-ops-rename-rule"),
                    Page::ALL
                        .iter()
                        .map(|p| if v.in_use(*p) { format!("{}*", p.label()) } else { p.label().to_string() })
                        .collect(),
                    Page::ALL.iter().position(|p| *p == self.page).unwrap_or(0),
                ),
                F::Template => Field::text(tr!("dialogs-rename-pattern"), v.template.clone()),
                F::Trim => Field::check(tr!("dialogs-rename-rule-trim"), v.trim),
                F::Collapse => Field::check(tr!("dialogs-rename-collapse"), v.collapse),
                F::Find => Field::text(tr!("dialogs-rename-find"), v.find.clone()),
                F::With => Field::text(tr!("dialogs-rename-replace"), v.with.clone()),
                F::Regex => Field::check(tr!("dialogs-rename-regex"), v.regex),
                F::MatchCase => Field::check(tr!("dialogs-rename-match-case"), v.match_case),
                F::FirstOnly => Field::check(tr!("dialogs-rename-first-only"), v.first_only),
                F::RemCount => Field::text(tr!("dialogs-rename-remove-count"), v.rem_count.clone()),
                F::RemStart => Field::text(tr!("dialogs-rename-remove-start"), v.rem_start.clone()),
                F::RemFromEnd => Field::check(tr!("dialogs-rename-from-end"), v.rem_from_end),
                F::InsText => Field::text(tr!("dialogs-rename-text"), v.ins_text.clone()),
                F::InsAt => radio(
                    tr!("dialogs-rename-position"),
                    vec![
                        tr!("dialogs-rename-pos-start").into(),
                        tr!("dialogs-rename-pos-end").into(),
                        tr!("dialogs-rename-pos-at").into(),
                    ],
                    v.ins_at,
                ),
                F::InsIndex => Field::text(tr!("tui-ops-rename-index"), v.ins_index.clone()),
                F::Case => radio(
                    tr!("dialogs-rename-rule-case"),
                    std::iter::once(tr!("tui-ops-keep"))
                        .chain(CaseMode::ALL.iter().map(|c| c.label()))
                        .map(String::from)
                        .collect(),
                    v.case,
                ),
                F::NumAt => radio(
                    tr!("dialogs-rename-rule-number"),
                    vec![
                        tr!("tui-ops-off").into(),
                        tr!("dialogs-rename-pos-start").into(),
                        tr!("dialogs-rename-pos-end").into(),
                    ],
                    v.num_at,
                ),
                F::Separator => Field::text(tr!("dialogs-rename-separator"), v.separator.clone()),
                F::Start => Field::text(tr!("dialogs-rename-counter-start"), v.start.clone()),
                F::Step => Field::text(tr!("dialogs-rename-counter-step"), v.step.clone()),
                F::Digits => Field::text(tr!("dialogs-rename-counter-digits"), v.digits.clone()),
                F::PerFolder => Field::check(tr!("dialogs-rename-per-folder"), v.per_folder),
                F::PresetName => Field::text(tr!("dialogs-rename-preset-name"), v.preset_name.clone()),
                F::Name => Field::text(tr!("tui-ops-name"), name_line.clone()),
            })
            .collect();
        let bmap: Vec<Btn> = match (mode, self.page) {
            (Mode::Editor, _) => vec![Btn::ResetNames, Btn::Go, Btn::Cancel],
            (Mode::Rules, Page::Presets) => vec![Btn::Load, Btn::SavePreset, Btn::DeletePreset, Btn::Go, Btn::Cancel],
            (Mode::Rules, _) => vec![Btn::Go, Btn::Cancel],
        };
        let buttons = bmap
            .iter()
            .map(|b| match b {
                Btn::Go => Button::new(tr!("common-rename")).accel('r'),
                Btn::Cancel => Button::new(tr!("common-cancel")),
                Btn::Load => Button::new(tr!("tui-ops-load")).accel('l'),
                Btn::SavePreset => Button::new(tr!("common-save")),
                Btn::DeletePreset => Button::new(tr!("dialogs-rename-delete-preset")),
                Btn::ResetNames => Button::new(tr!("dialogs-rename-reset-names")),
            })
            .collect();
        let go = bmap.iter().position(|b| *b == Btn::Go).unwrap_or(0);
        self.form = Form::new(fields, buttons).default_button(go);
        self.fmap = fmap;
        self.bmap = bmap;
        if let Some(i) = keep.and_then(|f| self.idx(f)) {
            self.form.focus = i;
        }
    }

    /// Line `i` of the editor text.
    fn editor_line(&self, i: usize) -> String {
        self.model.as_ref().and_then(|m| m.editor.split('\n').nth(i).map(str::to_string)).unwrap_or_default()
    }

    /// Set editor lines from line `i` on (the text keeps one line per item at least).
    fn set_editor_lines(&mut self, i: usize, lines: &[&str]) {
        let Some(m) = self.model.as_mut() else { return };
        let mut all: Vec<String> = m.editor.split('\n').map(str::to_string).collect();
        while all.len() < m.items.len() {
            all.push(String::new());
        }
        for (k, l) in lines.iter().enumerate() {
            if let Some(slot) = all.get_mut(i + k) {
                *slot = l.trim_end_matches('\r').to_string();
            }
        }
        m.editor = all.join("\n");
    }

    /// Reload the Name field from the cursor's editor line.
    fn reload_name(&mut self) {
        if let Some(i) = self.idx(F::Name) {
            let line = self.editor_line(self.list.cursor);
            super::prompt::set_text(&mut self.form, i, &line);
        }
    }

    /// Field `i` changed: its value into [`State::values`] (or the editor).
    fn read_field(&mut self, i: usize) {
        let Some(&f) = self.fmap.get(i) else { return };
        let field = &self.form.fields[i];
        let text = field.value().unwrap_or_default().to_string();
        let on = field.on().unwrap_or(false);
        let pick = field.selected().unwrap_or(0);
        let v = &mut self.values;
        match f {
            F::Mode => {
                let want = if pick == 1 { Mode::Editor } else { Mode::Rules };
                if let Some(m) = self.model.as_mut() {
                    m.set_mode(want);
                }
                self.list = SelectList::new();
                self.rebuild(Some(F::Mode));
            }
            F::Page => {
                self.page = Page::ALL.get(pick).copied().unwrap_or(Page::Replace);
                self.list = SelectList::new();
                self.rebuild(Some(F::Page));
            }
            F::Target => v.target = pick,
            F::Template => v.template = text,
            F::Trim => v.trim = on,
            F::Collapse => v.collapse = on,
            F::Find => v.find = text,
            F::With => v.with = text,
            F::Regex => v.regex = on,
            F::MatchCase => v.match_case = on,
            F::FirstOnly => v.first_only = on,
            F::RemCount => v.rem_count = text,
            F::RemStart => v.rem_start = text,
            F::RemFromEnd => v.rem_from_end = on,
            F::InsText => v.ins_text = text,
            F::InsAt => v.ins_at = pick,
            F::InsIndex => v.ins_index = text,
            F::Case => v.case = pick,
            F::NumAt => v.num_at = pick,
            F::Separator => v.separator = text,
            F::Start => v.start = text,
            F::Step => v.step = text,
            F::Digits => v.digits = text,
            F::PerFolder => v.per_folder = on,
            F::PresetName => v.preset_name = text,
            F::Name => {
                let c = self.list.cursor;
                self.set_editor_lines(c, &[&text]);
            }
        }
    }

    /// The "Rule" field's `*` marks follow the values.
    fn refresh_marks(&mut self) {
        let Some(i) = self.idx(F::Page) else { return };
        if let FieldKind::Radio { options, .. } = &mut self.form.fields[i].kind {
            for (o, p) in options.iter_mut().zip(Page::ALL) {
                *o = if self.values.in_use(p) { format!("{}*", p.label()) } else { p.label().to_string() };
            }
        }
    }

    /// After a change: the spec into the model, the preview, the error and the buttons.
    pub fn update(&mut self, wake: &Wake) {
        let spec = self.values.spec();
        self.refresh_marks();
        let presets = self.presets().len();
        let Some(m) = self.model.as_mut() else { return };
        let bad_field = match spec {
            Ok(spec) if m.mode == Mode::Rules => {
                m.spec = spec;
                None
            }
            Ok(_) => None,
            Err(e) if m.mode == Mode::Rules => Some(e),
            Err(_) => None,
        };
        m.recompute(wake);
        let (ok, _) = m.counts();
        self.form.error = bad_field.clone().or_else(|| m.compile_error.clone());
        let can_apply = bad_field.is_none() && m.can_apply();
        let saved = (self.list.cursor >= m.builtin.len()) && self.list.cursor < presets;
        let named = !self.values.preset_name.trim().is_empty();
        for (b, button) in self.bmap.iter().zip(self.form.buttons.iter_mut()) {
            match b {
                Btn::Go => {
                    button.label = tr!("dialogs-rename-go", count = ok);
                    button.enabled = can_apply;
                }
                Btn::Load => button.enabled = presets > 0,
                Btn::SavePreset => button.enabled = named && bad_field.is_none(),
                Btn::DeletePreset => button.enabled = saved,
                Btn::Cancel | Btn::ResetNames => {}
            }
        }
        let len = self.list_len();
        self.list.clamp(len, usize::from(MAX_LIST_ROWS));
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

/// A list key (PageUp / PageDown, Ctrl+Home / Ctrl+End).
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
    for ev in events {
        let len = s.list_len();
        let on_name = s.form.focused_field().is_some_and(|i| s.fmap.get(i) == Some(&F::Name));
        if on_name {
            // One name per line: Up / Down walk the lines; a paste of lines fills them.
            let step = match ev {
                InputEvent::Chord(c) if c.mods.is_empty() && c.key == KeyName::ArrowDown => Some(true),
                InputEvent::Chord(c) if c.mods.is_empty() && c.key == KeyName::ArrowUp => Some(false),
                _ => None,
            };
            if let Some(down) = step {
                let c =
                    if down { (s.list.cursor + 1).min(len.saturating_sub(1)) } else { s.list.cursor.saturating_sub(1) };
                s.list.select(c, len);
                s.list.show_cursor(usize::from(MAX_LIST_ROWS));
                s.reload_name();
                continue;
            }
            if let InputEvent::Text(t) = ev
                && t.contains('\n')
            {
                let lines: Vec<&str> = t.trim_end_matches(['\r', '\n']).split('\n').collect();
                let c = s.list.cursor;
                s.set_editor_lines(c, &lines);
                s.reload_name();
                s.update(wake);
                continue;
            }
        }
        match s.form.apply(ev) {
            FormOutcome::Cancel => return Outcome::Close(None),
            FormOutcome::Changed(i) => {
                s.read_field(i);
                s.update(wake);
            }
            FormOutcome::Pressed(b) => {
                if let Some(o) = press(wake, s, b) {
                    return o;
                }
            }
            FormOutcome::Ignored => {
                if let Some(m) = list_move(ev) {
                    let page = usize::from(MAX_LIST_ROWS);
                    s.list.go(m, len, page.saturating_sub(1));
                    s.list.show_cursor(page);
                    s.reload_name();
                    s.update(wake);
                }
            }
            FormOutcome::Consumed => {}
        }
    }
    Outcome::Keep
}

/// Button `b` was pressed.
fn press(wake: &Wake, s: &mut State, b: usize) -> Option<Outcome> {
    let btn = *s.bmap.get(b)?;
    match btn {
        Btn::Go => {
            let m = s.model.as_ref()?;
            if m.can_apply() && s.form.buttons.get(b).is_some_and(|x| x.enabled) {
                return Some(Outcome::Close(Some(m.apply_cmd())));
            }
            None
        }
        Btn::Cancel => Some(Outcome::Close(None)),
        Btn::Load => {
            let (_, p, _) = s.presets().into_iter().nth(s.list.cursor)?;
            if let Some(m) = s.model.as_mut() {
                m.use_preset(&p);
            }
            s.values = Values::from_spec(&p.spec, &s.values.preset_name);
            s.rebuild(None);
            s.update(wake);
            None
        }
        Btn::SavePreset => {
            let spec = s.values.spec().ok()?;
            let m = s.model.as_mut()?;
            m.spec = spec;
            m.preset_name = s.values.preset_name.clone();
            m.save_preset();
            s.values.preset_name.clear();
            s.rebuild(None);
            s.update(wake);
            None
        }
        Btn::DeletePreset => {
            if let Some((_, _, Some(i))) = s.presets().into_iter().nth(s.list.cursor)
                && let Some(m) = s.model.as_mut()
            {
                m.delete_preset(i);
            }
            s.update(wake);
            None
        }
        Btn::ResetNames => {
            if let Some(m) = s.model.as_mut() {
                m.reset_editor();
            }
            s.reload_name();
            s.update(wake);
            None
        }
    }
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    let len = s.list_len();
    let before = s.list.cursor;
    match super::form_mouse(&mut s.form, Some((&mut s.list, len)), e) {
        FormOutcome::Changed(i) => {
            s.read_field(i);
            s.update(&core.wake);
        }
        FormOutcome::Pressed(b) => {
            if let Some(o) = press(&core.wake, s, b) {
                return o;
            }
        }
        _ => {
            if s.list.cursor != before {
                s.reload_name();
                s.update(&core.wake);
            }
        }
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

    fn text(s: &str) -> InputEvent {
        InputEvent::Text(s.into())
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

    fn names(s: &State) -> Vec<String> {
        s.model.as_ref().unwrap().rows.iter().map(|r| r.new_name.clone()).collect()
    }

    fn go(s: &State) -> bool {
        s.form.buttons[s.bidx(Btn::Go).unwrap()].enabled
    }

    /// Focus the "Rule" field and pick `page`.
    fn page(wake: &Wake, s: &mut State, page: Page) {
        s.form.focus = s.idx(F::Page).unwrap();
        while s.page != page {
            let _ = handle(wake, s, &[chord("Right")]);
        }
    }

    #[test]
    fn numbering_fields_drive_the_preview() {
        let (wake, mut s) = state(&["a.txt", "b.txt"]);
        assert_eq!(s.form.focused_field(), s.idx(F::Find), "Find has the focus");
        assert!(!go(&s), "nothing changes yet");
        page(&wake, &mut s, Page::Number);
        s.form.focus = s.idx(F::NumAt).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Right"), chord("Right")]);
        assert_eq!(names(&s), ["a 01.txt", "b 02.txt"]);
        assert!(go(&s));
        // A bad counter blocks the rename with a message.
        s.form.focus = s.idx(F::Start).unwrap();
        let _ = handle(&wake, &mut s, &[text("x")]);
        assert!(s.form.error.is_some());
        assert!(!go(&s));
        assert!(matches!(handle(&wake, &mut s, &[chord("Enter")]), Outcome::Keep));
        let _ = handle(&wake, &mut s, &[chord("Backspace")]);
        assert!(s.form.error.is_none());
        // The step counts by two; the page is marked in use.
        s.form.focus = s.idx(F::Step).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Backspace"), text("2")]);
        assert_eq!(names(&s), ["a 01.txt", "b 03.txt"]);
        let marks = s.form.fields[s.idx(F::Page).unwrap()].clone();
        assert!(matches!(marks.kind, FieldKind::Radio { options, .. } if options[6].ends_with('*')));
        match handle(&wake, &mut s, &[chord("Enter")]) {
            Outcome::Close(Some(soyuz_appcore::cmd::Cmd::Meta(_))) => {}
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn every_rule_kind() {
        let (wake, mut s) = state(&["  My  File.TXT", "b.txt"]);
        page(&wake, &mut s, Page::Trim);
        s.form.focus = s.idx(F::Trim).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Space")]);
        assert_eq!(names(&s)[0], "My File.TXT");
        page(&wake, &mut s, Page::Insert);
        s.form.focus = s.idx(F::InsText).unwrap();
        let _ = handle(&wake, &mut s, &[text("x_")]);
        assert_eq!(names(&s)[1], "x_b.txt");
        s.form.focus = s.idx(F::InsAt).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Right")]);
        assert_eq!(names(&s)[1], "bx_.txt");
        page(&wake, &mut s, Page::Remove);
        s.form.focus = s.idx(F::RemCount).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Backspace"), text("1")]);
        assert_eq!(names(&s)[1], "x_.txt", "remove runs before insert");
        page(&wake, &mut s, Page::Case);
        s.form.focus = s.idx(F::Case).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Right"), chord("Right")]);
        assert_eq!(names(&s)[1], "X_.txt");
        page(&wake, &mut s, Page::Template);
        s.form.focus = s.idx(F::Template).unwrap();
        let _ = handle(&wake, &mut s, &[text("t{name}")]);
        assert_eq!(names(&s)[1], "BX_.txt", "template, trim, remove, insert, case in that order");
        // The target: the extension only.
        s.form.focus = s.idx(F::Target).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Right")]);
        assert_eq!(s.model.as_ref().unwrap().spec.target, Target::ALL[1]);
    }

    #[test]
    fn presets_load_and_one_name_per_line() {
        let (wake, mut s) = state(&["a b.txt", "c.txt"]);
        page(&wake, &mut s, Page::Presets);
        assert_eq!(s.list_len(), s.presets().len());
        let underscores =
            s.presets().iter().position(|(_, p, _)| p.name == tr!("dialogs-rename-preset-underscores")).unwrap();
        s.list.select(underscores, s.list_len());
        let load = s.bidx(Btn::Load).unwrap();
        assert!(press(&wake, &mut s, load).is_none());
        assert_eq!(names(&s)[0], "a_b.txt");
        assert!(s.values.trim && s.values.find == " ");
        // The editor mode starts from the preview; Name edits the cursor's line.
        s.form.focus = s.idx(F::Mode).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Right")]);
        assert_eq!(s.mode(), Mode::Editor);
        assert_eq!(s.form.fields[s.idx(F::Name).unwrap()].value(), Some("a_b.txt"));
        s.form.focus = s.idx(F::Name).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Down")]);
        assert_eq!(s.list.cursor, 1);
        let _ = handle(&wake, &mut s, &[chord("Backspace"), chord("Backspace"), chord("Backspace"), text("md")]);
        assert_eq!(names(&s), ["a_b.txt", "c.md"]);
        // A paste of lines fills them from the cursor.
        let _ = handle(&wake, &mut s, &[chord("Up"), text("one.txt\ntwo.txt\n")]);
        assert_eq!(names(&s), ["one.txt", "two.txt"]);
        let reset = s.bidx(Btn::ResetNames).unwrap();
        let _ = press(&wake, &mut s, reset);
        assert_eq!(names(&s), ["a b.txt", "c.txt"]);
    }

    #[test]
    fn duplicates_are_flagged() {
        let (wake, mut s) = state(&["a1.txt", "a2.txt", "b.txt"]);
        let _ = handle(&wake, &mut s, &[text("[0-9]")]);
        s.form.focus = s.idx(F::Regex).unwrap();
        let _ = handle(&wake, &mut s, &[chord("Space")]);
        let rows = &s.model.as_ref().unwrap().rows;
        assert_eq!(rows[0].status, Status::Duplicate);
        assert_eq!(rows[2].status, Status::Unchanged);
        assert_eq!(status_label(&rows[0].status), tr!("tui-ops-status-duplicate"));
        assert!(!go(&s));
        // PageDown scrolls the preview.
        let _ = handle(&wake, &mut s, &[chord("PageDown")]);
        assert_eq!(s.list.cursor, 2);
        assert!(matches!(handle(&wake, &mut s, &[chord("Esc")]), Outcome::Close(None)));
    }

    #[test]
    fn values_round_trip_through_a_spec() {
        let v = Values {
            template: "{name}".into(),
            trim: true,
            find: "a".into(),
            with: "b".into(),
            rem_count: "2".into(),
            ins_text: "x".into(),
            ins_at: 2,
            ins_index: "-1".into(),
            case: 2,
            num_at: 2,
            step: "3".into(),
            ..Values::default()
        };
        let spec = v.spec().unwrap();
        assert_eq!(spec.rules.len(), 7);
        assert_eq!(Values::from_spec(&spec, ""), v);
    }
}
