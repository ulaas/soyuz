//! The table editor of the settings editor (`TuiDialog::TableEdit`): the elements of an
//! array of tables (`[[bookmarks]]`, `[[ai.profiles]]`, `[[automation.rules]]`, …) or a
//! map of tables (`[table.<name>]`), opened by Enter on such a key.
//!
//! The list shows each element by its key or label field (`name`, `model`, `path`, …).
//! Add puts a fresh element (the config's defaults) after the cursor and opens its form;
//! a map of tables asks for the new key first (taken keys are refused). Duplicate copies
//! the element (a `name` gets a unique suffix), Remove asks first, Alt+Up / Alt+Down move
//! it (maps are sorted by key). Enter opens the element's form (`table_row`), drawn in
//! this dialog's box.
//!
//! Changes are validated on the dialog's own [`Draft`] (`settings_draft`) and each one is
//! handed to the settings editor through its [`Inbox`] (the whole table, replacing an
//! earlier value not applied yet): the editor applies it live and writes `config.toml`
//! as soon as it has the keyboard again. Secrets set in an element form go to the OS
//! credential store (`secret_set`); this dialog reports when the write is done.
//!
//! | Key | Does |
//! |---|---|
//! | Up / Down / PgUp / PgDn / Home / End | element |
//! | Enter | edit the element |
//! | Insert / Alt+A | add |
//! | Alt+D | duplicate |
//! | Delete / Alt+R | remove (asks) |
//! | Alt+Up / Alt+Down | move |
//! | Ctrl+E | edit `config.toml` in `$EDITOR` |
//! | Esc | close (back from a form, cancel a question) |

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::settings_draft::Draft;
use soyuz_appcore::models::toasts::Toast;
use soyuz_appcore::tr;
use soyuz_core::config::{Action, KeyName, Mods};

use super::secret_set::{self, Pending};
use super::table_row::{self, Row, RowDone, post_table};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog, form_mouse};
use crate::app::App;
use crate::input::settings::Inbox;
use crate::layout::HitTarget;
use crate::view::dialogs::MAX_LIST_ROWS;
use crate::widgets::{Button, Field, Form, FormOutcome, SelectList};

/// Button indices of the list.
const ADD: usize = 0;
const DUPLICATE: usize = 1;
const REMOVE: usize = 2;

/// What the dialog asks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The list of elements.
    #[default]
    List,
    /// The key of a new element of a map of tables.
    Name,
    /// Remove element `n`?
    Remove(usize),
}

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    /// Draft path of the table (`"bookmarks"`).
    pub table: String,
    /// The element struct (`"Bookmark"`).
    pub element: String,
    /// A map of tables (`[table.<name>]`) rather than an array.
    pub keyed: bool,
    pub draft: Option<Box<Draft>>,
    pub list: SelectList,
    /// The buttons (and the key field while naming).
    pub form: Form,
    pub mode: Mode,
    /// The element form, when one is open.
    pub row: Option<Box<Row>>,
    /// The settings editor's.
    pub inbox: Option<Inbox>,
    /// Credential store writes to report.
    pub pending: Vec<Pending>,
    /// The table value last handed to the editor.
    pub posted: Option<toml::Value>,
    /// The table value when the dialog opened.
    pub initial: Option<toml::Value>,
    /// Tab moved the keyboard to the buttons (Enter presses them instead of editing).
    pub on_buttons: bool,
}

impl State {
    /// Elements in the table.
    pub fn len(&self) -> usize {
        self.draft.as_ref().map_or(0, |d| d.table_len(&self.table))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Open the table editor for the table at `path`; results go to the editor's `inbox`.
pub fn open(app: &mut App, path: &str, inbox: Inbox) {
    if let Some(s) = state(&app.core, path, inbox) {
        app.push_dialog(TuiDialog::TableEdit(s));
    }
}

/// The dialog for the table at `path` (`None` when it is not one).
pub fn state(core: &AppCore, path: &str, inbox: Inbox) -> Option<State> {
    let (element, keyed) = table_row::element_of(path)?;
    let draft = Draft::from_config(&core.config);
    let initial = draft.value(path).cloned();
    let mut s = State {
        title: tr!("tui-tables-title", table = crate::dialog::safe(path)),
        table: path.to_string(),
        element,
        keyed,
        draft: Some(Box::new(draft)),
        inbox: Some(inbox),
        posted: initial.clone(),
        initial,
        ..State::default()
    };
    list_mode(&mut s);
    Some(s)
}

/// The label of element `i`: its key, else the first identifying text field.
pub fn label(draft: &Draft, table: &str, i: usize) -> String {
    if let Some(k) = draft.element_key(table, i) {
        return k;
    }
    for f in ["name", "model", "path", "watch", "command", "url"] {
        if let Some(s) = draft.element_value(table, i, f).and_then(toml::Value::as_str)
            && !s.trim().is_empty()
        {
            return s.to_string();
        }
    }
    tr!("tui-tables-entry", n = i + 1)
}

/// A second fact about element `i` for the right column (path, kind, command, …).
pub fn detail(draft: &Draft, table: &str, i: usize) -> String {
    let shown = label(draft, table, i);
    for f in ["path", "kind", "command", "url", "watch", "access"] {
        if let Some(s) = draft.element_value(table, i, f).and_then(toml::Value::as_str)
            && !s.trim().is_empty()
            && s != shown
        {
            return s.to_string();
        }
    }
    String::new()
}

fn list_mode(s: &mut State) {
    s.mode = Mode::List;
    s.on_buttons = false;
    let has = !s.is_empty();
    let mut dup = Button::new(tr!("tui-tables-duplicate")).accel('d');
    dup.enabled = has;
    let mut remove = Button::new(tr!("tui-settings-list-remove")).accel('r');
    remove.enabled = has;
    let mut form = Form::new(
        Vec::new(),
        vec![Button::new(tr!("tui-settings-list-add")).accel('a'), dup, remove, Button::new(tr!("common-close"))],
    );
    form.default_button = None;
    s.form = form;
    let n = s.len();
    s.list.clamp(n, usize::from(MAX_LIST_ROWS));
}

fn name_mode(s: &mut State) {
    s.mode = Mode::Name;
    s.form = Form::new(
        vec![Field::text(tr!("tui-tables-key"), "")],
        vec![Button::new(tr!("common-ok")), Button::new(tr!("common-cancel"))],
    );
}

fn remove_mode(s: &mut State, i: usize) {
    s.mode = Mode::Remove(i);
    let mut form =
        Form::new(Vec::new(), vec![Button::new(tr!("tui-settings-list-remove")), Button::new(tr!("common-cancel"))]);
    form.focus_button(1);
    form.default_button = Some(1);
    s.form = form;
}

/// Open the form of element `i`.
fn edit(s: &mut State, i: usize) {
    if let Some(d) = &s.draft {
        s.row = Row::new(d, &s.table, i).map(Box::new);
    }
}

/// A new element after the cursor (a map of tables asks for its key first).
fn add(s: &mut State) {
    if s.keyed {
        name_mode(s);
        return;
    }
    let at = if s.is_empty() { 0 } else { s.list.cursor + 1 };
    let Some(d) = s.draft.as_mut() else { return };
    match d.table_insert(&s.table, at, None) {
        Ok(i) => {
            list_mode(s);
            s.list.select(i, s.len());
            s.list.show_cursor(usize::from(MAX_LIST_ROWS));
            edit(s, i);
        }
        Err(e) => s.form.error = Some(tr!("tui-settings-invalid", error = e)),
    }
}

/// The key typed for a new element of a map of tables.
fn add_named(s: &mut State) {
    let name = s.form.fields.first().and_then(Field::value).unwrap_or_default().trim().to_string();
    let Some(d) = s.draft.as_mut() else { return };
    if let Err(e) = key_error(d, &s.table, &name) {
        s.form.error = Some(e);
        return;
    }
    match d.table_insert(&s.table, 0, Some(&name)) {
        Ok(i) => {
            list_mode(s);
            s.list.select(i, s.len());
            edit(s, i);
        }
        Err(e) => s.form.error = Some(tr!("tui-settings-invalid", error = e)),
    }
}

/// Why `name` cannot be the key of a new element of the map at `table`.
pub fn key_error(d: &Draft, table: &str, name: &str) -> Result<(), String> {
    let taken: Vec<String> = (0..d.table_len(table)).filter_map(|i| d.element_key(table, i)).collect();
    check_key(&taken, name)
}

/// A key must be given and must not be taken.
pub fn check_key(taken: &[String], name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(tr!("tui-tables-key-needed").into());
    }
    if taken.iter().any(|t| t == name) {
        return Err(tr!("tui-tables-name-taken", name = name));
    }
    Ok(())
}

fn duplicate(s: &mut State) {
    let i = s.list.cursor;
    let Some(d) = s.draft.as_mut() else { return };
    match d.table_duplicate(&s.table, i) {
        Ok(j) => {
            list_mode(s);
            s.list.select(j, s.len());
            s.list.show_cursor(usize::from(MAX_LIST_ROWS));
        }
        Err(e) => s.form.error = Some(tr!("tui-settings-invalid", error = e)),
    }
}

fn remove(s: &mut State, i: usize) {
    if let Some(d) = s.draft.as_mut()
        && let Err(e) = d.table_remove(&s.table, i)
    {
        s.form.error = Some(tr!("tui-settings-invalid", error = e));
    }
    list_mode(s);
}

fn move_by(s: &mut State, down: bool) {
    if s.keyed {
        return;
    }
    let i = s.list.cursor;
    let j = if down { i + 1 } else { i.wrapping_sub(1) };
    let n = s.len();
    if j >= n {
        return;
    }
    if let Some(d) = s.draft.as_mut()
        && d.table_move(&s.table, i, j).is_ok()
    {
        s.list.select(j, n);
        s.list.show_cursor(usize::from(MAX_LIST_ROWS));
    }
}

/// Ctrl+E: `config.toml` in `$EDITOR`. Changes made here are applied first (the editor
/// takes them when the dialog closes), so then Ctrl+E is pressed again in the editor.
fn edit_config(core: &mut AppCore, s: &State) -> Outcome {
    if s.posted != s.initial {
        core.ui.toasts.push(Toast::info(tr!("tui-tables-editor-after")));
        Outcome::Close(None)
    } else {
        Outcome::Close(Some(Cmd::Action(Action::EditConfigFile)))
    }
}

/// A button of the list (or of a question).
fn pressed(s: &mut State, b: usize) -> Option<Outcome> {
    match (&s.mode, b) {
        (Mode::List, ADD) => add(s),
        (Mode::List, DUPLICATE) => duplicate(s),
        (Mode::List, REMOVE) => {
            if !s.is_empty() {
                remove_mode(s, s.list.cursor);
            }
        }
        (Mode::List, _) => return Some(Outcome::Close(None)),
        (Mode::Name, 0) => add_named(s),
        (Mode::Remove(i), 0) => {
            let i = *i;
            remove(s, i);
        }
        _ => list_mode(s),
    }
    None
}

fn list_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Option<Outcome> {
    let n = s.len();
    for ev in &i.events {
        if s.row.is_some() {
            break;
        }
        if let InputEvent::Chord(c) = ev {
            if c.mods == Mods::CTRL && c.key == KeyName::E {
                return Some(edit_config(core, s));
            }
            if s.mode == Mode::List {
                let plain = c.mods.is_empty();
                if plain && s.list.apply(ev, n, usize::from(MAX_LIST_ROWS)) {
                    s.on_buttons = false;
                    s.form.error = None;
                    continue;
                }
                if c.mods == Mods::ALT && matches!(c.key, KeyName::ArrowUp | KeyName::ArrowDown) {
                    move_by(s, c.key == KeyName::ArrowDown);
                    continue;
                }
                if plain && c.key == KeyName::Insert {
                    add(s);
                    continue;
                }
                if plain && c.key == KeyName::Delete && n > 0 {
                    remove_mode(s, s.list.cursor);
                    continue;
                }
                if plain && c.key == KeyName::Enter && !s.on_buttons {
                    if n == 0 {
                        add(s);
                    } else {
                        edit(s, s.list.cursor);
                    }
                    continue;
                }
                if c.key == KeyName::Tab {
                    s.on_buttons = true;
                }
            }
        }
        match s.form.apply(ev) {
            FormOutcome::Pressed(b) => {
                if let Some(out) = pressed(s, b) {
                    return Some(out);
                }
            }
            FormOutcome::Cancel => {
                if s.mode == Mode::List {
                    return Some(Outcome::Close(None));
                }
                list_mode(s);
            }
            FormOutcome::Changed(_) => s.form.error = None,
            _ => {}
        }
    }
    None
}

/// After every input: report finished credential writes, hand changes to the editor.
fn after(core: &mut AppCore, s: &mut State, out: Option<Outcome>) -> Outcome {
    let running = secret_set::poll(core, &mut s.pending);
    if let Some(d) = &s.draft {
        post_table(core, s.inbox.as_ref(), d, &s.table, &mut s.posted);
    }
    if s.row.is_none() && s.mode == Mode::List {
        // The buttons follow the list (Duplicate / Remove need an element).
        let has = !s.is_empty();
        for b in [DUPLICATE, REMOVE] {
            if let Some(btn) = s.form.buttons.get_mut(b) {
                btn.enabled = has;
            }
        }
        let n = s.len();
        s.list.clamp(n, usize::from(MAX_LIST_ROWS));
    }
    let out = out.unwrap_or(Outcome::Keep);
    if matches!(out, Outcome::Close(_)) && running && s.pending.iter().any(|p| p.mcp_client) {
        // Closing before the write finished: let the MCP server re-read its tokens anyway.
        core.services.mcp.refresh_tokens();
    }
    out
}

fn row_done(core: &mut AppCore, s: &mut State, done: RowDone) -> Option<Outcome> {
    match done {
        RowDone::Keep => None,
        RowDone::Close => {
            if let Some(row) = s.row.take() {
                s.list.select(row.index, s.len());
                s.list.show_cursor(usize::from(MAX_LIST_ROWS));
            }
            list_mode(s);
            None
        }
        RowDone::EditConfig => Some(edit_config(core, s)),
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let out = match (s.row.as_mut(), s.draft.as_mut()) {
        (Some(row), Some(d)) => {
            let done = table_row::input(core, row, d, &mut s.pending, i);
            row_done(core, s, done)
        }
        _ => list_input(core, s, i),
    };
    after(core, s, out)
}

/// A mouse event: a row selects (a second click edits it), the buttons press, the wheel
/// scrolls.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    let out = match (s.row.as_mut(), s.draft.as_mut()) {
        (Some(row), Some(d)) => {
            let done = table_row::mouse(core, row, d, &mut s.pending, m);
            row_done(core, s, done)
        }
        _ => {
            let before = s.list.cursor;
            let n = if s.mode == Mode::List { s.len() } else { 0 };
            match form_mouse(&mut s.form, Some((&mut s.list, n)), m) {
                FormOutcome::Pressed(b) => pressed(s, b),
                _ => {
                    let down = matches!(m.event.kind, crossterm::event::MouseEventKind::Down(_));
                    if s.mode == Mode::List
                        && down
                        && let Some(HitTarget::DialogRow(i)) = m.target
                        && i == before
                        && i < n
                    {
                        edit(s, i);
                    }
                    None
                }
            }
        }
    };
    after(core, s, out)
}

/// The dialog went away by itself: never (the editor may close under it; the inbox then
/// holds the last changes, applied when the editor opens again).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use soyuz_core::config::Config;

    use super::*;

    fn st(table: &str) -> State {
        let (element, keyed) = table_row::element_of(table).unwrap();
        let draft = Draft::from_config(&Config::default());
        let mut s = State { table: table.into(), element, keyed, draft: Some(Box::new(draft)), ..State::default() };
        list_mode(&mut s);
        s
    }

    #[test]
    fn add_duplicate_move_and_remove() {
        let mut s = st("bookmarks");
        assert!(s.is_empty());
        assert!(!s.form.buttons[REMOVE].enabled);
        add(&mut s);
        assert_eq!(s.len(), 1);
        assert!(s.row.is_some(), "a new element opens its form");
        s.row = None;
        let d = s.draft.as_mut().unwrap();
        d.element_set("bookmarks", 0, "name", toml::Value::String("Code".into())).unwrap();
        duplicate(&mut s);
        assert_eq!(s.len(), 2);
        assert_eq!(s.list.cursor, 1);
        assert_eq!(label(s.draft.as_ref().unwrap(), "bookmarks", 1), "Code 2");
        move_by(&mut s, false);
        assert_eq!(s.list.cursor, 0);
        assert_eq!(s.draft.as_ref().unwrap().config.bookmarks[0].name, "Code 2");
        remove_mode(&mut s, 0);
        assert_eq!(s.form.focused_button(), Some(1), "Cancel is the safe default");
        assert!(pressed(&mut s, 0).is_none());
        assert_eq!(s.mode, Mode::List);
        assert_eq!(s.draft.as_ref().unwrap().config.bookmarks.len(), 1);
        assert_eq!(s.draft.as_ref().unwrap().config.bookmarks[0].name, "Code");
    }

    #[test]
    fn labels_fall_back_to_identifying_fields() {
        let mut d = Draft::from_config(&Config::default());
        d.table_insert("ai.prices", 0, None).unwrap();
        d.element_set("ai.prices", 0, "model", toml::Value::String("claude-*".into())).unwrap();
        assert!(label(&d, "ai.prices", 0).starts_with("claude-*"));
        d.table_insert("sidebar.portal", 0, None).unwrap();
        assert!(!label(&d, "sidebar.portal", 0).is_empty());
        d.element_set("sidebar.portal", 0, "path", toml::Value::String("/p".into())).unwrap();
        assert_eq!(label(&d, "sidebar.portal", 0), "/p");
    }

    #[test]
    fn keys_of_maps_are_unique() {
        let taken = vec!["work".to_string(), "home".to_string()];
        assert!(check_key(&taken, "work").is_err(), "a taken key is refused");
        assert!(check_key(&taken, " home ").is_err());
        assert!(check_key(&taken, "  ").is_err(), "a key is needed");
        assert!(check_key(&taken, "travel").is_ok());
    }
}
