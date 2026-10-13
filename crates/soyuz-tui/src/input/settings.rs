//! Keys and mouse of the settings editor, a full-screen overlay (Phase T6), and of its
//! key bindings page (the Keyboard section, `crate::input::settings_keys`).
//!
//! The editor is generated from the config schema (`soyuz_appcore::models::settings_schema`):
//! every section of `Section::ALL` lists the keys of its tables, each edited by its kind.
//! Edits go to a [`Draft`] (validated by deserializing the whole config) and every
//! committed edit is applied live (`settings_draft::apply`: theme, key map, panes, and a
//! debounced `config.toml` write that keeps the user's comments).
//!
//! | Key | Does |
//! |---|---|
//! | Up / Down / PgUp / PgDn / Home / End | move (sections in the section list) |
//! | Tab / Shift+Tab | section list ↔ keys |
//! | Ctrl+PgUp / Ctrl+PgDn | previous / next section |
//! | Enter | edit: a switch toggles, a choice opens its list, numbers and text are edited in place, lists and maps open the list editor, tables the table editor |
//! | Space | toggle a switch, next value of a choice |
//! | Left / Right (Shift: ×10) | previous / next value, number down / up (clamped to its range) |
//! | Backspace / Delete | reset the key to its default |
//! | Ctrl+R | reset the whole section (asks first) |
//! | `/`, Ctrl+F, or just typing | search every section |
//! | F1 / `?` | show / hide the description lines (shown by default from 24 rows) |
//! | Ctrl+E | edit `config.toml` in `$EDITOR` (the editor is rebuilt from the file afterwards) |
//! | Esc | cancel the edit, clear the search, close |
//!
//! Other chords reach the core only for the actions that make sense over the overlay
//! ([`PASSTHROUGH`]: quit, settings toggles, config reload). Mouse: a click selects (a
//! second click on the selected key edits it), the wheel scrolls, `×` closes.
//!
//! The list editor and the shortcut recorder are dialogs (`dialogs::{list_edit,
//! keybind}`); they hand their result back through the [`Inbox`] the editor drains every
//! frame.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use crossterm::event::{Event, MouseButton, MouseEvent, MouseEventKind};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::settings::{self as settings_model, Section, matches};
use soyuz_appcore::models::settings_draft::{self, Draft};
use soyuz_appcore::models::settings_schema::{self as schema, FieldKind, FieldRow};
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_core::config::{Action, KeyChord, KeyContext, KeyName, Mods};

use super::InputState;
use super::settings_keys::{self, KeyFilter};
use crate::app::App;
use crate::input::dialogs::{self, TuiDialog};
use crate::layout::HitTarget;
use crate::widgets::{Button, EditOutcome, Form, FormOutcome, LineEdit, SelectList};

/// Actions a chord may still run while the editor is open (everything else is the
/// editor's or does nothing: no new tabs or file operations under the overlay).
pub const PASSTHROUGH: &[Action] = &[
    Action::Quit,
    Action::Settings,
    Action::ShowKeybindings,
    Action::ReloadConfig,
    Action::OpenScripts,
    Action::OpenAutomations,
];

/// A result handed back by a dialog of the editor (`list_edit`, `keybind`).
#[derive(Clone, Debug, PartialEq)]
pub enum Commit {
    /// Set a key (validated by the dialog against the live config).
    Set { path: String, value: toml::Value },
    /// The bindings of an action (`[keys.bindings]`).
    Bindings { action: Action, list: Vec<String> },
}

/// Where dialogs leave their [`Commit`]s for the editor.
pub type Inbox = Arc<Mutex<Vec<Commit>>>;

/// Hand `c` to the editor (and ask for a frame so it is applied at once).
pub fn post(inbox: &Inbox, core: &AppCore, c: Commit) {
    if let Ok(mut v) = inbox.lock() {
        v.push(c);
    }
    (core.wake)();
}

/// Who has the keyboard inside the editor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Part {
    /// The section list.
    Nav,
    /// The keys of the section (or the search results).
    #[default]
    Rows,
    /// The search field.
    Search,
}

/// One line of the key list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// A table / group heading.
    Header(String),
    /// A warning line (key conflicts).
    Note(String),
    /// A config key by its path.
    Field(String),
    /// A filter switch of the key bindings page.
    Toggle(KeyFilter),
    /// An action of the key bindings page (index into `settings_keys::State::rows`).
    Action(usize),
}

impl Line {
    /// The cursor can stop on it.
    pub fn selectable(&self) -> bool {
        matches!(self, Line::Field(_) | Line::Toggle(_) | Line::Action(_))
    }
}

/// A key edited in place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    pub path: String,
    pub text: String,
    pub line: LineEdit,
}

/// A small box over the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    /// Pick one value of a choice.
    Choose { path: String, values: Vec<(String, String)>, list: SelectList },
    /// Reset a whole section?
    Confirm { section: Section, form: Form },
}

/// What the editor keeps between frames (`InputState::settings`).
#[derive(Debug)]
pub struct State {
    /// The editor is open (it owns the keyboard: `TuiFocus::Settings`).
    pub open: bool,
    pub section: Section,
    pub part: Part,
    /// The search text (the key filter on the Keyboard page).
    pub query: String,
    pub search: LineEdit,
    /// Cursor and scroll of the section list.
    pub nav: SelectList,
    /// Cursor and scroll of the key list.
    pub list: SelectList,
    /// The key list ([`refresh`]).
    pub lines: Vec<Line>,
    /// Rows of the key list shown last frame.
    pub page: usize,
    /// Height of the editor last frame (description lines show from 24 rows).
    pub height: u16,
    pub draft: Option<Draft>,
    pub edit: Option<Edit>,
    pub popup: Option<Popup>,
    /// The last failed edit: (path, translated message).
    pub error: Option<(String, String)>,
    /// F1 / `?`: description lines on / off (`None`: by height).
    pub help: Option<bool>,
    /// Restart-only keys already announced this session.
    pub restart_told: BTreeSet<String>,
    pub inbox: Inbox,
    /// The key bindings page.
    pub keys: settings_keys::State,
    /// What the view keeps (the fields' visible parts).
    pub view: crate::view::settings::Kept,
}

impl Default for State {
    fn default() -> State {
        State {
            open: false,
            section: Section::General,
            part: Part::Rows,
            query: String::new(),
            search: LineEdit::new(),
            nav: SelectList::new(),
            list: SelectList::new(),
            lines: Vec::new(),
            page: 10,
            height: 24,
            draft: None,
            edit: None,
            popup: None,
            error: None,
            help: None,
            restart_told: BTreeSet::new(),
            inbox: Arc::new(Mutex::new(Vec::new())),
            keys: settings_keys::State::default(),
            view: Default::default(),
        }
    }
}

impl State {
    /// The Keyboard section shows the key bindings page.
    pub fn keys_page(&self) -> bool {
        self.section == Section::Keyboard
    }

    /// The search filters every section (not on the Keyboard page, where it filters the
    /// actions).
    pub fn searching(&self) -> bool {
        !self.query.trim().is_empty() && !self.keys_page()
    }

    /// The line under the cursor.
    pub fn current(&self) -> Option<&Line> {
        self.lines.get(self.list.cursor)
    }

    /// The config path under the cursor.
    pub fn current_path(&self) -> Option<&str> {
        match self.current() {
            Some(Line::Field(p)) => Some(p),
            _ => None,
        }
    }

    /// Description lines are shown for an editor `height` rows high.
    pub fn help_shown(&self, height: u16) -> bool {
        self.help.unwrap_or(height >= 24)
    }
}

/// It owns the keyboard this frame (`input::focus`).
pub fn has_focus(_core: &AppCore, st: &InputState) -> bool {
    st.settings.open
}

/// While open, every mouse event is the editor's (dialogs on top get theirs first).
pub fn captures_mouse(_core: &AppCore, st: &InputState) -> bool {
    st.settings.open
}

/// Open the settings editor, on `section` when given (`FrontendCmd::OpenSettings`).
/// Without a section while it is open, it closes (the Settings action toggles).
pub fn open(app: &mut App, section: Option<Section>) {
    let st = &mut app.input.settings;
    if st.open && section.is_none() {
        close(app);
        return;
    }
    if !st.open {
        st.open = true;
        st.part = Part::Rows;
        st.query.clear();
        st.search = LineEdit::new();
        st.edit = None;
        st.popup = None;
        st.error = None;
        st.draft = Some(Draft::from_config(&app.core.config));
    }
    if let Some(s) = section {
        set_section(st, s);
    }
    st.nav.cursor = Section::ALL.iter().position(|s| *s == st.section).unwrap_or(0);
    app.core.ui.settings_open = true;
    if app.core.ui.quicklook.open {
        soyuz_appcore::models::quicklook::close(&mut app.core);
    }
    refresh(&app.core, &mut app.input.settings);
}

/// Close the settings editor (`FrontendCmd::CloseSettings`), writing a pending save.
pub fn close(app: &mut App) {
    let st = &mut app.input.settings;
    if !st.open {
        return;
    }
    st.open = false;
    st.edit = None;
    st.popup = None;
    st.error = None;
    st.draft = None;
    app.core.ui.settings_open = false;
    settings_model::flush(&mut app.core);
}

/// The config was reloaded from the file (after `$EDITOR`, `crate::suspend`): start the
/// draft again from it.
pub fn reload(app: &mut App) {
    let st = &mut app.input.settings;
    if st.open {
        st.draft = Some(Draft::from_config(&app.core.config));
        st.edit = None;
        st.error = None;
        refresh(&app.core, st);
    }
}

fn set_section(st: &mut State, s: Section) {
    if st.section != s {
        st.section = s;
        st.list = SelectList::new();
        st.keys.chip = 0;
        st.edit = None;
        st.error = None;
    }
}

/// Rebuild the key list for the section / search (and keep the cursor on a key).
pub fn refresh(core: &AppCore, st: &mut State) {
    // The line under the cursor, to find it again when lines come or go above it.
    let was_action = |st: &State, i: usize| st.keys.rows.get(i).map(|r| r.action.clone());
    let before = match st.current() {
        Some(Line::Action(i)) => was_action(st, *i).map(Ok),
        Some(l @ (Line::Field(_) | Line::Toggle(_))) => Some(Err(l.clone())),
        _ => None,
    };
    st.lines = if st.keys_page() {
        let mut lines = section_lines(Section::Keyboard, &st.query);
        settings_keys::refresh(&mut st.keys, &core.config, &st.query);
        settings_keys::lines(&st.keys, &mut lines);
        lines
    } else if st.searching() {
        search_lines(&st.query)
    } else {
        section_lines(st.section, "")
    };
    let again = before.and_then(|b| {
        st.lines.iter().position(|l| match (&b, l) {
            (Ok(a), Line::Action(i)) => st.keys.rows.get(*i).is_some_and(|r| r.action == *a),
            (Err(x), l) => x == l,
            _ => false,
        })
    });
    if let Some(i) = again {
        st.list.cursor = i;
    }
    let len = st.lines.len();
    st.list.clamp(len, st.page);
    st.list.show_cursor(st.page.max(1));
    if !st.lines.get(st.list.cursor).is_some_and(Line::selectable) {
        st.list.cursor =
            seek(&st.lines, st.list.cursor, true).or_else(|| seek(&st.lines, st.list.cursor, false)).unwrap_or(0);
    }
}

/// Keys the editor never lists: the key bindings map (the Keyboard page edits it).
fn listed(r: &FieldRow) -> bool {
    r.path != "keys.bindings"
}

/// The words a key is found by.
fn search_texts(sec: Section, r: &FieldRow) -> String {
    let values = match &r.kind {
        FieldKind::Enum { values } | FieldKind::EnumList { values } => {
            values.iter().map(|(v, _)| v.as_str()).collect::<Vec<_>>().join(" ")
        }
        _ => String::new(),
    };
    [r.path.as_str(), &r.name.replace('_', " "), &r.doc, sec.title(), sec.english_title(), &values].join(" ")
}

/// The keys of `section` (those matching `query`), under their table headings.
fn section_lines(section: Section, query: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for t in schema::tables_of(section) {
        let rows: Vec<&FieldRow> =
            t.rows.iter().filter(|r| listed(r) && matches(query, &[&search_texts(section, r)])).collect();
        if rows.is_empty() {
            continue;
        }
        if !t.table.is_empty() {
            out.push(Line::Header(format!("[{}]", t.table)));
        }
        out.extend(rows.into_iter().map(|r| Line::Field(r.path.clone())));
    }
    out
}

/// Keys of every section matching `query`, under "section · [table]" headings.
fn search_lines(query: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for sec in Section::ALL {
        for t in schema::tables_of(sec) {
            let rows: Vec<&FieldRow> =
                t.rows.iter().filter(|r| listed(r) && matches(query, &[&search_texts(sec, r)])).collect();
            if rows.is_empty() {
                continue;
            }
            let head =
                if t.table.is_empty() { sec.title().to_string() } else { format!("{} · [{}]", sec.title(), t.table) };
            out.push(Line::Header(head));
            out.extend(rows.into_iter().map(|r| Line::Field(r.path.clone())));
        }
    }
    if out.is_empty() {
        out.push(Line::Header(tr!("tui-settings-no-results").to_string()));
    }
    out
}

/// The nearest selectable line from `from` (inclusive) going down (`forward`) or up.
fn seek(lines: &[Line], from: usize, forward: bool) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let from = from.min(lines.len() - 1);
    if forward {
        (from..lines.len()).find(|&i| lines[i].selectable())
    } else {
        (0..=from).rev().find(|&i| lines[i].selectable())
    }
}

/// Move the key cursor by `delta` lines (skipping headings), or to an end.
fn step(st: &mut State, delta: isize) {
    let n = st.lines.len();
    if n == 0 {
        return;
    }
    let cur = st.list.cursor as isize;
    let target = (cur + delta).clamp(0, n as isize - 1) as usize;
    let forward = delta >= 0;
    let found = seek(&st.lines, target, forward).or_else(|| seek(&st.lines, target, !forward));
    if let Some(i) = found {
        st.list.cursor = i;
    }
    st.keys.chip = 0;
    st.list.show_cursor(st.page.max(1));
    // A heading right above the first key stays in view.
    if st.list.cursor > 0 && st.list.top == st.list.cursor && !st.lines[st.list.cursor - 1].selectable() {
        st.list.top -= 1;
    }
}

fn to_end(st: &mut State, last: bool) {
    let n = st.lines.len() as isize;
    if last {
        step(st, n);
    } else {
        st.list.top = 0;
        step(st, -n);
    }
}

/// Change the section by `delta` (wrapping).
fn section_by(st: &mut State, delta: isize) {
    let n = Section::ALL.len() as isize;
    let i = Section::ALL.iter().position(|s| *s == st.section).unwrap_or(0) as isize;
    let next = Section::ALL[((i + delta).rem_euclid(n)) as usize];
    set_section(st, next);
    st.nav.cursor = ((i + delta).rem_euclid(n)) as usize;
    st.nav.show_cursor(st.page.max(1));
}

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Used {
    No,
    Yes,
    /// Used, and the text the key typed (if it follows) too.
    SkipText,
}

/// This frame's events while it has the focus.
pub fn route(app: &mut App, events: &[Event]) {
    if !app.input.settings.open {
        return;
    }
    sync(app);
    let mut frame = crate::keys::frame(events, app.core.config.keys.cmd_is_ctrl, cfg!(windows));
    let n = frame.events.len();
    let mut i = 0;
    while i < n {
        if !app.input.settings.open || dialogs::is_open(&app.core, &app.input.dialogs) {
            break;
        }
        let ev = frame.events[i].clone();
        let used = key(app, &ev);
        if used != Used::No {
            frame.consumed[i] = true;
        }
        if used == Used::SkipText
            && let Some(InputEvent::Text(_)) = frame.events.get(i + 1)
        {
            frame.consumed[i + 1] = true;
            i += 1;
        }
        i += 1;
    }
    passthrough(app, &frame);
    if app.input.settings.open {
        refresh(&app.core, &mut app.input.settings);
    }
}

/// Apply the dialogs' results and follow config changes made elsewhere (hot reload,
/// `$EDITOR`, the core's own toggles): the draft always describes the live config.
pub fn sync(app: &mut App) {
    let commits: Vec<Commit> = app.input.settings.inbox.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
    for c in commits {
        match c {
            Commit::Set { path, value } => {
                commit(app, &path, |d| d.set(&path, value));
            }
            Commit::Bindings { action, list } => {
                let path = "keys.bindings".to_string();
                commit(app, &path, |d| {
                    let mut c = d.config.clone();
                    soyuz_appcore::models::settings_keys::set_override(&mut c, &action, list);
                    let v = toml::Value::try_from(&c.keys.bindings).map_err(|e| e.to_string())?;
                    d.set("keys.bindings", v)
                });
            }
        }
    }
    let st = &mut app.input.settings;
    if st.open && st.draft.as_ref().is_none_or(|d| d.config != app.core.config) {
        st.draft = Some(Draft::from_config(&app.core.config));
    }
}

/// Run `f` on the draft; on success make it live (and say once when the key needs a
/// restart), on failure keep the message for the key's row.
fn commit(app: &mut App, path: &str, f: impl FnOnce(&mut Draft) -> Result<(), String>) -> bool {
    let st = &mut app.input.settings;
    let draft = st.draft.get_or_insert_with(|| Draft::from_config(&app.core.config));
    let before = draft.value(path).cloned();
    match f(draft) {
        Ok(()) => {
            st.error = None;
            let changed = draft.value(path).cloned() != before;
            let config = draft.config.clone();
            if changed && schema::row(path).is_some_and(|r| r.restart) && st.restart_told.insert(path.to_string()) {
                app.core.ui.toasts.push(Toast::info(tr!("tui-settings-restart")).body(path));
            }
            settings_draft::apply(&mut app.core, &app.host, config);
            true
        }
        Err(e) => {
            st.error = Some((path.to_string(), tr!("tui-settings-invalid", error = e)));
            false
        }
    }
}

/// Run the core's action for `c` when it is one of [`PASSTHROUGH`].
fn passthrough(app: &mut App, frame: &soyuz_appcore::keys::input::InputFrame) {
    for (ev, used) in frame.events.iter().zip(&frame.consumed) {
        if *used || !app.input.settings.open {
            continue;
        }
        let InputEvent::Chord(c) = ev else { continue };
        let fkey = matches!(
            c.key,
            KeyName::F1
                | KeyName::F2
                | KeyName::F3
                | KeyName::F4
                | KeyName::F5
                | KeyName::F6
                | KeyName::F7
                | KeyName::F8
                | KeyName::F9
                | KeyName::F10
                | KeyName::F11
                | KeyName::F12
        );
        if !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD) && !fkey {
            continue;
        }
        let Some(a) = app.core.keymap.lookup(KeyContext::Dialog, *c).cloned() else { continue };
        if PASSTHROUGH.contains(&a) {
            soyuz_appcore::actions::execute(&mut app.core, a, &app.host);
        }
    }
}

fn plain(c: &KeyChord) -> bool {
    !c.mods.intersects(Mods::CTRL | Mods::ALT | Mods::CMD | Mods::SHIFT)
}

fn ctrl(c: &KeyChord) -> bool {
    c.mods.contains(Mods::CTRL) && !c.mods.intersects(Mods::ALT | Mods::CMD | Mods::SHIFT)
}

/// One event, by who has the keyboard inside the editor.
fn key(app: &mut App, ev: &InputEvent) -> Used {
    if app.input.settings.popup.is_some() {
        return popup_key(app, ev);
    }
    if app.input.settings.edit.is_some() {
        return edit_key(app, ev);
    }
    if app.input.settings.part == Part::Search {
        let st = &mut app.input.settings;
        match st.search.apply(&mut st.query, ev) {
            EditOutcome::Consumed => {
                st.list = SelectList::new();
                refresh(&app.core, st);
                return Used::Yes;
            }
            EditOutcome::Submit | EditOutcome::Down | EditOutcome::Tab => {
                st.part = Part::Rows;
                st.list = SelectList::new();
                refresh(&app.core, st);
                return Used::Yes;
            }
            EditOutcome::Cancel => {
                st.query.clear();
                st.search = LineEdit::new();
                st.part = Part::Rows;
                refresh(&app.core, st);
                return Used::Yes;
            }
            EditOutcome::Up | EditOutcome::ShiftTab | EditOutcome::PageUp | EditOutcome::PageDown => return Used::Yes,
            EditOutcome::Ignored => {}
        }
        return common_key(app, ev);
    }
    let InputEvent::Chord(c) = ev else {
        // Typing with no field focused starts a search.
        if let InputEvent::Text(t) = ev
            && !t.trim().is_empty()
        {
            start_search(app);
            let st = &mut app.input.settings;
            st.search.apply(&mut st.query, ev);
            refresh(&app.core, st);
            return Used::Yes;
        }
        return Used::No;
    };
    if c.key == KeyName::Slash && plain(c) {
        start_search(app);
        return Used::SkipText;
    }
    if c.key == KeyName::Slash && c.mods == Mods::SHIFT {
        toggle_help(&mut app.input.settings);
        return Used::SkipText;
    }
    let used = if app.input.settings.part == Part::Nav { nav_key(app, c) } else { rows_key(app, c) };
    if used != Used::No { used } else { common_key(app, ev) }
}

fn start_search(app: &mut App) {
    let st = &mut app.input.settings;
    st.part = Part::Search;
    st.search = LineEdit::at_end(&st.query);
}

/// Keys that work wherever the editor's keyboard is (but in the popups and in-place
/// edits).
fn common_key(app: &mut App, ev: &InputEvent) -> Used {
    let InputEvent::Chord(c) = ev else { return Used::No };
    let st = &mut app.input.settings;
    if plain(c) {
        match c.key {
            KeyName::Escape => {
                if st.part == Part::Search || !st.query.is_empty() {
                    st.query.clear();
                    st.search = LineEdit::new();
                    st.part = Part::Rows;
                    refresh(&app.core, st);
                } else {
                    close(app);
                }
                return Used::Yes;
            }
            KeyName::F1 => {
                toggle_help(st);
                return Used::Yes;
            }
            KeyName::Tab => {
                st.part = if st.part == Part::Nav { Part::Rows } else { Part::Nav };
                return Used::Yes;
            }
            _ => {}
        }
    }
    if c.mods == Mods::SHIFT && c.key == KeyName::Tab {
        st.part = if st.part == Part::Nav { Part::Rows } else { Part::Nav };
        return Used::Yes;
    }
    if ctrl(c) {
        match c.key {
            KeyName::F => {
                start_search(app);
                return Used::Yes;
            }
            KeyName::E => {
                soyuz_appcore::actions::execute(&mut app.core, Action::EditConfigFile, &app.host);
                return Used::Yes;
            }
            KeyName::R => {
                let section = st.section;
                st.popup = Some(Popup::Confirm {
                    section,
                    form: Form::new(
                        Vec::new(),
                        vec![Button::new(tr!("common-reset")), Button::new(tr!("common-cancel"))],
                    )
                    .default_button(1),
                });
                if let Some(Popup::Confirm { form, .. }) = &mut st.popup {
                    form.focus_button(1);
                }
                return Used::Yes;
            }
            KeyName::PageUp => {
                section_by(st, -1);
                refresh(&app.core, st);
                return Used::Yes;
            }
            KeyName::PageDown => {
                section_by(st, 1);
                refresh(&app.core, st);
                return Used::Yes;
            }
            _ => {}
        }
    }
    Used::No
}

fn toggle_help(st: &mut State) {
    let shown = st.help_shown(st.height);
    st.help = Some(!shown);
}

/// Keys of the section list.
fn nav_key(app: &mut App, c: &KeyChord) -> Used {
    if !plain(c) {
        return Used::No;
    }
    let st = &mut app.input.settings;
    let i = Section::ALL.iter().position(|s| *s == st.section).unwrap_or(0) as isize;
    let n = Section::ALL.len() as isize;
    let page = st.page.max(2) as isize - 1;
    let to = match c.key {
        KeyName::ArrowUp => i - 1,
        KeyName::ArrowDown => i + 1,
        KeyName::PageUp => i - page,
        KeyName::PageDown => i + page,
        KeyName::Home => 0,
        KeyName::End => n - 1,
        KeyName::Enter | KeyName::ArrowRight => {
            st.part = Part::Rows;
            return Used::Yes;
        }
        _ => return Used::No,
    }
    .clamp(0, n - 1);
    set_section(st, Section::ALL[to as usize]);
    st.nav.cursor = to as usize;
    st.query.clear();
    refresh(&app.core, st);
    Used::Yes
}

/// Keys of the key list.
fn rows_key(app: &mut App, c: &KeyChord) -> Used {
    let st = &mut app.input.settings;
    let page = st.page.max(2) as isize - 1;
    if plain(c) {
        match c.key {
            KeyName::ArrowUp => step(st, -1),
            KeyName::ArrowDown => step(st, 1),
            KeyName::PageUp => step(st, -page),
            KeyName::PageDown => step(st, page),
            KeyName::Home => to_end(st, false),
            KeyName::End => to_end(st, true),
            _ => return line_key(app, c),
        }
        return Used::Yes;
    }
    line_key(app, c)
}

/// A key for the line under the cursor.
fn line_key(app: &mut App, c: &KeyChord) -> Used {
    match app.input.settings.current().cloned() {
        Some(Line::Field(path)) => field_key(app, &path, c),
        Some(Line::Toggle(f)) => {
            if plain(c) && matches!(c.key, KeyName::Enter | KeyName::Space) {
                settings_keys::toggle_filter(&mut app.input.settings.keys, f);
                refresh(&app.core, &mut app.input.settings);
                return Used::SkipText;
            }
            settings_keys::page_key(app, c)
        }
        Some(Line::Action(i)) => settings_keys::action_key(app, i, c),
        _ => {
            if app.input.settings.keys_page() {
                settings_keys::page_key(app, c)
            } else {
                Used::No
            }
        }
    }
}

/// Keys on a config key's line.
fn field_key(app: &mut App, path: &str, c: &KeyChord) -> Used {
    let Some(row) = schema::row(path) else { return Used::No };
    let shift = c.mods == Mods::SHIFT;
    if !plain(c) && !(shift && matches!(c.key, KeyName::ArrowLeft | KeyName::ArrowRight)) {
        return Used::No;
    }
    match c.key {
        KeyName::Enter => {
            activate(app, path);
            Used::Yes
        }
        KeyName::Space => {
            match &row.kind {
                FieldKind::Bool => toggle(app, path),
                FieldKind::Enum { .. } => {
                    commit(app, path, |d| d.cycle_enum(path, true));
                }
                _ => {}
            }
            Used::SkipText
        }
        KeyName::ArrowLeft | KeyName::ArrowRight => {
            let forward = c.key == KeyName::ArrowRight;
            match &row.kind {
                FieldKind::Bool => toggle(app, path),
                FieldKind::Enum { .. } => {
                    commit(app, path, |d| d.cycle_enum(path, forward));
                }
                FieldKind::Int { .. } | FieldKind::Float { .. } => {
                    let step = nudge_step(&row.kind) * if shift { 10.0 } else { 1.0 };
                    let delta = if forward { step } else { -step };
                    commit(app, path, |d| d.nudge(path, delta));
                }
                _ => return Used::No,
            }
            Used::Yes
        }
        KeyName::Backspace | KeyName::Delete => {
            commit(app, path, |d| d.reset(path));
            Used::Yes
        }
        _ => Used::No,
    }
}

/// One step of Left / Right on a number: whole numbers by 1; fractions by a step that
/// suits their range.
pub fn nudge_step(kind: &FieldKind) -> f64 {
    match kind {
        FieldKind::Float { min: Some(lo), max: Some(hi) } => {
            let span = hi - lo;
            if span <= 2.0 {
                0.05
            } else if span <= 20.0 {
                0.5
            } else {
                1.0
            }
        }
        FieldKind::Float { .. } => 0.1,
        _ => 1.0,
    }
}

fn toggle(app: &mut App, path: &str) {
    let unset = app.input.settings.draft.as_ref().is_none_or(|d| d.value(path).is_none());
    if unset {
        commit(app, path, |d| d.set(path, toml::Value::Boolean(true)));
    } else {
        commit(app, path, |d| d.toggle(path));
    }
}

/// Enter on a key: edit it the way its kind is edited.
pub fn activate(app: &mut App, path: &str) {
    let Some(row) = schema::row(path) else { return };
    let current = app.input.settings.draft.as_ref().and_then(|d| d.value(path).cloned());
    match &row.kind {
        FieldKind::Bool => toggle(app, path),
        FieldKind::Enum { values } => {
            let cur = current.as_ref().and_then(toml::Value::as_str).unwrap_or_default();
            let mut list = SelectList::new();
            list.select(values.iter().position(|(v, _)| v == cur).unwrap_or(0), values.len());
            list.show_cursor(usize::from(crate::view::dialogs::MAX_LIST_ROWS));
            app.input.settings.popup = Some(Popup::Choose { path: path.to_string(), values: values.clone(), list });
        }
        FieldKind::Int { .. } | FieldKind::Float { .. } | FieldKind::Text | FieldKind::Path | FieldKind::Color => {
            let text = current.as_ref().map(edit_text).unwrap_or_default();
            app.input.settings.edit = Some(Edit { path: path.to_string(), line: LineEdit::at_end(&text), text });
        }
        FieldKind::Chord => {
            let inbox = app.input.settings.inbox.clone();
            dialogs::keybind::open_field(app, path, inbox);
        }
        FieldKind::EnumList { .. } | FieldKind::TextList | FieldKind::Map => {
            let inbox = app.input.settings.inbox.clone();
            dialogs::list_edit::open(app, path, inbox);
        }
        // The [[table]] editor (Wave 3) owns this dialog.
        FieldKind::Table { .. } => app.push_dialog(TuiDialog::TableEdit(Default::default())),
        FieldKind::Opaque => {
            app.core.ui.toasts.push(Toast::info(tr!("tui-settings-opaque-hint")).body(path));
        }
    }
}

/// The text a value is edited as.
pub fn edit_text(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Float(f) => format_float(*f),
        other => other.to_string(),
    }
}

/// A float without the noise of `f32` → `f64` (`0.4`, not `0.4000000059604645`).
pub fn format_float(f: f64) -> String {
    let s = format!("{f:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
}

/// Keys while a key is edited in place.
fn edit_key(app: &mut App, ev: &InputEvent) -> Used {
    let st = &mut app.input.settings;
    let Some(e) = st.edit.as_mut() else { return Used::No };
    match e.line.apply(&mut e.text, ev) {
        EditOutcome::Submit => {
            let Edit { path, text, .. } = st.edit.take().unwrap_or_default();
            submit_text(app, &path, &text);
            Used::Yes
        }
        EditOutcome::Cancel => {
            st.edit = None;
            st.error = None;
            Used::Yes
        }
        EditOutcome::Ignored => Used::No,
        _ => Used::Yes,
    }
}

/// Commit the text typed for a key: numbers must parse (else the edit stays open with
/// the message), an empty optional key is unset.
fn submit_text(app: &mut App, path: &str, text: &str) {
    let Some(row) = schema::row(path) else { return };
    let t = text.trim();
    let optional = Draft::default_value(path).is_none();
    if t.is_empty() && optional {
        commit(app, path, |d| d.reset(path));
        return;
    }
    let value = match &row.kind {
        FieldKind::Int { .. } => t.parse::<i64>().ok().map(toml::Value::Integer),
        FieldKind::Float { .. } => {
            t.replace(',', ".").parse::<f64>().ok().filter(|f| f.is_finite()).map(toml::Value::Float)
        }
        _ => Some(toml::Value::String(text.to_string())),
    };
    let Some(value) = value else {
        let st = &mut app.input.settings;
        st.error = Some((path.to_string(), tr!("tui-settings-not-a-number", text = t)));
        st.edit = Some(Edit { path: path.to_string(), line: LineEdit::at_end(text), text: text.to_string() });
        return;
    };
    if !commit(app, path, |d| d.set(path, value)) {
        app.input.settings.edit =
            Some(Edit { path: path.to_string(), line: LineEdit::at_end(text), text: text.to_string() });
    }
}

/// Keys of the open popup.
fn popup_key(app: &mut App, ev: &InputEvent) -> Used {
    let st = &mut app.input.settings;
    match st.popup.as_mut() {
        Some(Popup::Choose { path, values, list }) => {
            if list.apply(ev, values.len(), usize::from(crate::view::dialogs::MAX_LIST_ROWS)) {
                return Used::Yes;
            }
            let InputEvent::Chord(c) = ev else { return Used::Yes };
            if !plain(c) {
                return Used::No;
            }
            match c.key {
                KeyName::Escape => st.popup = None,
                KeyName::Enter | KeyName::Space => {
                    let path = path.clone();
                    let value = values.get(list.cursor).map(|(v, _)| v.clone());
                    st.popup = None;
                    if let Some(v) = value {
                        commit(app, &path, |d| d.set(&path, toml::Value::String(v)));
                    }
                    return Used::SkipText;
                }
                _ => {}
            }
            Used::Yes
        }
        Some(Popup::Confirm { form, .. }) => {
            let out = form.apply(ev);
            confirm_outcome(app, out);
            Used::SkipText
        }
        None => Used::No,
    }
}

fn confirm_outcome(app: &mut App, out: FormOutcome) {
    let st = &mut app.input.settings;
    let Some(Popup::Confirm { section, .. }) = &st.popup else { return };
    let section = *section;
    match out {
        FormOutcome::Pressed(0) => {
            st.popup = None;
            commit(app, section.id(), |d| d.reset_section(section));
            app.core.ui.toasts.push(Toast::info(tr!("tui-settings-section-reset", section = section.title())));
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => st.popup = None,
        _ => {}
    }
}

/// A mouse event over one of its click targets (`input::mouse` hands every event over
/// while it is open); returns whether it was taken.
pub fn on_mouse(app: &mut App, target: HitTarget, ev: &MouseEvent) -> bool {
    if !app.input.settings.open {
        return false;
    }
    let st = &mut app.input.settings;
    match ev.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let rows =
                if ev.kind == MouseEventKind::ScrollUp { -super::mouse::WHEEL_ROWS } else { super::mouse::WHEEL_ROWS };
            match (&mut st.popup, target) {
                (Some(Popup::Choose { values, list, .. }), _) => {
                    list.wheel(rows, values.len(), usize::from(crate::view::dialogs::MAX_LIST_ROWS))
                }
                (_, HitTarget::SettingsNav(_)) => st.nav.wheel(rows, Section::ALL.len(), st.page),
                _ => st.list.wheel(rows, st.lines.len(), st.page),
            }
            return true;
        }
        MouseEventKind::Down(MouseButton::Left) => {}
        _ => return true,
    }
    if st.popup.is_some() {
        popup_click(app, target);
        return true;
    }
    if st.edit.is_some() {
        if matches!(target, HitTarget::SettingsRow(i) | HitTarget::SettingsKeyRow(i) if i == st.list.cursor) {
            // A click on the edited key keeps editing.
            return true;
        }
        // A click elsewhere commits the edit first.
        if let Some(e) = st.edit.take() {
            submit_text(app, &e.path, &e.text);
        }
    }
    let st = &mut app.input.settings;
    match target {
        HitTarget::SettingsClose => close(app),
        HitTarget::SettingsSearch => start_search(app),
        HitTarget::SettingsNav(i) => {
            if let Some(&s) = Section::ALL.get(i) {
                set_section(st, s);
                st.nav.cursor = i;
                st.query.clear();
                st.part = Part::Nav;
                refresh(&app.core, st);
            }
        }
        HitTarget::SettingsRow(i) | HitTarget::SettingsKeyRow(i) => {
            if !st.lines.get(i).is_some_and(Line::selectable) {
                return true;
            }
            let again = st.list.cursor == i && st.part == Part::Rows;
            st.list.cursor = i;
            st.part = Part::Rows;
            if again {
                match st.lines[i].clone() {
                    Line::Field(p) => activate(app, &p),
                    Line::Toggle(f) => {
                        settings_keys::toggle_filter(&mut st.keys, f);
                        refresh(&app.core, st);
                    }
                    Line::Action(a) => settings_keys::record(app, a, None),
                    _ => {}
                }
            } else if let Line::Toggle(f) = st.lines[i].clone() {
                settings_keys::toggle_filter(&mut st.keys, f);
                refresh(&app.core, st);
            }
        }
        HitTarget::SettingsKeyChip { row, chip } => settings_keys::chip_click(app, row, chip),
        HitTarget::SettingsKeyAdd(row) => {
            st.list.cursor = row;
            st.part = Part::Rows;
            settings_keys::record(app, row, None);
        }
        _ => {}
    }
    true
}

fn popup_click(app: &mut App, target: HitTarget) {
    let st = &mut app.input.settings;
    match (st.popup.as_mut(), target) {
        (Some(Popup::Choose { path, values, list }), HitTarget::DialogRow(i)) => {
            if list.cursor == i {
                let path = path.clone();
                let value = values.get(i).map(|(v, _)| v.clone());
                st.popup = None;
                if let Some(v) = value {
                    commit(app, &path, |d| d.set(&path, toml::Value::String(v)));
                }
            } else {
                list.select(i, values.len());
            }
        }
        (Some(Popup::Confirm { form, .. }), HitTarget::DialogButton(i)) => {
            let out = form.click_button(i);
            confirm_outcome(app, out);
        }
        (Some(_), HitTarget::DialogRow(_) | HitTarget::DialogButton(_) | HitTarget::DialogField(_)) => {}
        // A click outside the box closes it.
        _ => st.popup = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_lists_keys() {
        for s in Section::ALL {
            let lines = section_lines(s, "");
            assert!(lines.iter().any(Line::selectable), "{s:?} has no keys");
            assert!(!lines.contains(&Line::Field("keys.bindings".into())));
        }
        let tui = section_lines(Section::TerminalUi, "");
        assert_eq!(tui.first(), Some(&Line::Header("[tui]".into())));
        assert!(tui.contains(&Line::Field("tui.borders".into())));
    }

    #[test]
    fn search_finds_keys_in_every_section() {
        let lines = search_lines("borders");
        assert!(lines.contains(&Line::Field("tui.borders".into())), "{lines:?}");
        assert!(matches!(lines.first(), Some(Line::Header(_))));
        // By a value of a choice and by words of the name.
        assert!(search_lines("truecolor").contains(&Line::Field("tui.color_mode".into())));
        assert!(search_lines("show hidden").contains(&Line::Field("general.show_hidden".into())));
        let none = search_lines("zzqqxx-nothing");
        assert_eq!(none.len(), 1);
        assert!(!none[0].selectable());
    }

    #[test]
    fn the_cursor_skips_headings() {
        let mut st = State { lines: section_lines(Section::Preview, ""), page: 5, ..State::default() };
        st.list.cursor = 0;
        refresh_cursor(&mut st);
        assert!(st.lines[st.list.cursor].selectable());
        let headings: Vec<usize> =
            st.lines.iter().enumerate().filter(|(_, l)| !l.selectable()).map(|(i, _)| i).collect();
        assert!(headings.len() > 1, "Preview has several tables");
        for _ in 0..st.lines.len() {
            step(&mut st, 1);
            assert!(st.lines[st.list.cursor].selectable());
        }
        assert_eq!(st.list.cursor, st.lines.len() - 1);
        to_end(&mut st, false);
        assert_eq!(Some(st.list.cursor), seek(&st.lines, 0, true));
        assert_eq!(st.list.top, 0, "the first heading shows");
    }

    fn refresh_cursor(st: &mut State) {
        if !st.lines[st.list.cursor].selectable() {
            st.list.cursor = seek(&st.lines, st.list.cursor, true).unwrap();
        }
    }

    #[test]
    fn floats_read_well() {
        assert_eq!(format_float(0.4000000059604645), "0.4");
        assert_eq!(format_float(12.0), "12");
        assert_eq!(format_float(-0.25), "-0.25");
        assert_eq!(format_float(0.0), "0");
        assert_eq!(edit_text(&toml::Value::Integer(5)), "5");
        assert_eq!(edit_text(&toml::Value::String("x y".into())), "x y");
    }

    #[test]
    fn nudge_steps_suit_the_range() {
        let f = |lo, hi| FieldKind::Float { min: Some(lo), max: Some(hi) };
        assert_eq!(nudge_step(&f(0.2, 0.6)), 0.05);
        assert_eq!(nudge_step(&f(11.0, 17.0)), 0.5);
        assert_eq!(nudge_step(&f(0.0, 1000.0)), 1.0);
        assert_eq!(nudge_step(&FieldKind::Int { min: None, max: None }), 1.0);
    }
}
