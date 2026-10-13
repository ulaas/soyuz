//! The form of one element of an array / map of tables in the settings editor
//! (`TuiDialog::TableRow`; usually shown inside the table editor, `table_edit`): every
//! field of the element struct (`settings_schema::element_rows`), edited by its kind the
//! way the top-level editor edits keys.
//!
//! The list shows the fields with their values; the field below it edits the one under
//! the cursor: a switch, a choice (Left / Right), or a text field (numbers are checked,
//! Enter applies). Lists and maps open the list editor, chords the shortcut recorder,
//! credential references (`secret_ref`) take a `credential:<name>` / `env:<VAR>` reference
//! or "Set secret…" (`secret_set`): the secret goes to the OS credential store, never into
//! `config.toml`. Fields a form cannot hold (automation rule actions, a saved search's
//! query) say "Edit in $EDITOR (Ctrl+E)".
//!
//! Every change is validated on the dialog's [`Draft`] (`settings_draft`: the whole config
//! must deserialize; the message names the key and shows under the field); names stay
//! unique within the list.
//!
//! | Key | Does |
//! |---|---|
//! | Up / Down / PgUp / PgDn / Home / End | field |
//! | Enter | apply the text, toggle a switch, open the list editor / recorder |
//! | Left / Right, Space | change a choice / switch |
//! | Alt+R | reset the field to the default of a new element |
//! | Alt+S | set the secret (credential reference fields) |
//! | Ctrl+E | edit `config.toml` in `$EDITOR` |
//! | Esc | back |

use std::sync::{Arc, Mutex};

use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::settings_draft::{self, Draft};
use soyuz_appcore::models::settings_schema::{self as schema, FieldKind, FieldRow};
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::secret_set::{self, Pending};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog, form_mouse, keybind, list_edit};
use crate::app::App;
use crate::input::settings::{Commit, Inbox, edit_text};
use crate::layout::HitTarget;
use crate::view::dialogs::MAX_LIST_ROWS;
use crate::widgets::{Button, Field, FieldKind as FormField, Form, FormOutcome, SelectList};

/// Button indices.
const RESET: usize = 0;
const SECRET: usize = 1;

/// A dialog shown inside the form.
#[derive(Debug)]
pub enum Sub {
    List(Box<list_edit::State>),
    Keybind(keybind::State),
    Secret(secret_set::State),
}

/// The form of element `index` of the table at `table`.
#[derive(Debug)]
pub struct Row {
    /// Draft path of the table (`"bookmarks"`, `"ai.profiles"`).
    pub table: String,
    pub index: usize,
    /// The element struct (`"Bookmark"`).
    pub element: String,
    /// Cursor over the fields.
    pub list: SelectList,
    /// The editor of the field under the cursor (0 or 1 field) and the buttons Reset,
    /// Set secret…, Close.
    pub form: Form,
    /// The last failed change: (field index, message).
    pub error: Option<(usize, String)>,
    pub sub: Option<Sub>,
    /// Where the list editor and the recorder leave their results.
    pub child: Inbox,
    /// Tab moved the keyboard to the buttons (Enter presses them instead of editing).
    pub on_buttons: bool,
}

/// What an input did to the form.
#[derive(Debug, PartialEq, Eq)]
pub enum RowDone {
    Keep,
    /// Back to the list (or closed, standalone).
    Close,
    /// Ctrl+E: edit `config.toml` in `$EDITOR`.
    EditConfig,
}

/// The fields of element struct `element`.
pub fn fields(element: &str) -> &'static [FieldRow] {
    schema::element_rows(element).map(|t| t.rows.as_slice()).unwrap_or_default()
}

/// The element struct of the table at `table`, and whether it is a map of tables.
pub fn element_of(table: &str) -> Option<(String, bool)> {
    match &schema::row(table)?.kind {
        FieldKind::Table { element, keyed } => Some((element.clone(), *keyed)),
        _ => None,
    }
}

impl Row {
    /// The form of element `index` of `table` (`None`: not a table, or no such element).
    pub fn new(draft: &Draft, table: &str, index: usize) -> Option<Row> {
        let (element, _) = element_of(table)?;
        if index >= draft.table_len(table) || fields(&element).is_empty() {
            return None;
        }
        let mut row = Row {
            table: table.to_string(),
            index,
            element,
            list: SelectList::new(),
            form: Form::default(),
            error: None,
            sub: None,
            child: Arc::new(Mutex::new(Vec::new())),
            on_buttons: false,
        };
        load(&mut row, draft);
        Some(row)
    }

    /// The field under the cursor.
    pub fn field(&self) -> Option<&'static FieldRow> {
        fields(&self.element).get(self.list.cursor)
    }

    /// The draft path of `r` in this element (`"bookmarks[2].hotkey"`).
    pub fn path(&self, draft: &Draft, r: &FieldRow) -> String {
        draft.element_path(&self.table, self.index, &r.name)
    }
}

/// The name of the element (for titles and secret names): its key, else its `name`.
pub fn element_name(draft: &Draft, table: &str, index: usize) -> String {
    draft
        .element_key(table, index)
        .or_else(|| draft.element_value(table, index, "name").and_then(toml::Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// Another element of the list already has the name `name`.
pub fn name_taken(draft: &Draft, table: &str, index: usize, name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && (0..draft.table_len(table)).filter(|&j| j != index).any(|j| {
            draft.element_value(table, j, "name").and_then(toml::Value::as_str).is_some_and(|n| n.trim() == name)
        })
}

/// Put the field under the cursor into the form.
pub fn load(row: &mut Row, draft: &Draft) {
    let n = fields(&row.element).len();
    row.list.clamp(n, usize::from(MAX_LIST_ROWS));
    let Some(r) = row.field() else { return };
    let path = row.path(draft, r);
    let v = draft.value(&path).cloned().or_else(|| Draft::default_value(&path));
    let field = match &r.kind {
        FieldKind::Bool => {
            Some(Field::check(r.name.clone(), v.as_ref().and_then(toml::Value::as_bool).unwrap_or(false)))
        }
        FieldKind::Enum { values } => {
            let cur = v.as_ref().and_then(toml::Value::as_str).unwrap_or_default();
            let i = values.iter().position(|(x, _)| x == cur).unwrap_or(0);
            Some(Field::radio(r.name.clone(), values.iter().map(|(x, _)| x.clone()).collect(), i))
        }
        FieldKind::Int { .. } | FieldKind::Float { .. } | FieldKind::Text | FieldKind::Path | FieldKind::Color => {
            Some(Field::text(r.name.clone(), v.as_ref().map(edit_text).unwrap_or_default()))
        }
        _ => None,
    };
    let mut secret = Button::new(tr!("tui-tables-set-secret")).accel('s');
    secret.enabled = r.secret_ref;
    let mut form = Form::new(
        field.into_iter().collect(),
        vec![Button::new(tr!("common-reset")).accel('r'), secret, Button::new(tr!("common-close"))],
    );
    form.default_button = None;
    row.form = form;
    row.on_buttons = false;
}

/// Run a change of field `i`: on success the error goes, on failure it shows under it.
fn change(row: &mut Row, i: usize, f: impl FnOnce() -> Result<(), String>) -> bool {
    match f() {
        Ok(()) => {
            row.error = None;
            true
        }
        Err(e) => {
            row.error = Some((i, tr!("tui-settings-invalid", error = e)));
            false
        }
    }
}

/// The value typed for `r` (numbers parsed; `None` with a message when it does not).
fn parse(r: &FieldRow, text: &str) -> Result<toml::Value, String> {
    let t = text.trim();
    let bad = || tr!("tui-settings-not-a-number", text = t);
    match &r.kind {
        FieldKind::Int { .. } => t.parse::<i64>().map(toml::Value::Integer).map_err(|_| bad()),
        FieldKind::Float { .. } => {
            t.replace(',', ".").parse::<f64>().ok().filter(|f| f.is_finite()).map(toml::Value::Float).ok_or_else(bad)
        }
        _ => Ok(toml::Value::String(text.to_string())),
    }
}

/// Enter in the text field: apply it.
fn apply_text(row: &mut Row, draft: &mut Draft) {
    let Some(r) = row.field() else { return };
    let i = row.list.cursor;
    let text = row.form.fields.first().and_then(Field::value).unwrap_or_default().to_string();
    let path = row.path(draft, r);
    if r.secret_ref
        && let Err(e) = settings_draft::check_reference(&text)
    {
        row.error = Some((i, e));
        return;
    }
    if r.name == "name" && name_taken(draft, &row.table, row.index, &text) {
        row.error = Some((i, tr!("tui-tables-name-taken", name = text.trim())));
        return;
    }
    if text.trim().is_empty() && Draft::default_value(&path).is_none() {
        change(row, i, || draft.reset(&path));
    } else {
        match parse(r, &text) {
            Ok(v) => {
                change(row, i, || draft.set(&path, v));
            }
            Err(e) => row.error = Some((i, e)),
        }
    }
    if row.error.is_none() {
        load(row, draft);
    }
}

/// A switch or choice changed in the form: apply it at once.
fn apply_pick(row: &mut Row, draft: &mut Draft) {
    let Some(r) = row.field() else { return };
    let i = row.list.cursor;
    let path = row.path(draft, r);
    let v = match row.form.fields.first().map(|f| &f.kind) {
        Some(FormField::Check(on)) => toml::Value::Boolean(*on),
        Some(FormField::Radio { options, selected }) => {
            toml::Value::String(options.get(*selected).cloned().unwrap_or_default())
        }
        _ => return,
    };
    if !change(row, i, || draft.set(&path, v)) {
        load(row, draft);
    }
}

/// Enter on a field that is not edited in the form: open its editor.
fn activate(row: &mut Row, draft: &mut Draft) {
    let Some(r) = row.field() else { return };
    let path = row.path(draft, r);
    match &r.kind {
        FieldKind::Bool => {
            if let Some(FormField::Check(on)) = row.form.fields.first_mut().map(|f| &mut f.kind) {
                *on = !*on;
            }
            apply_pick(row, draft);
        }
        FieldKind::Enum { .. } => {}
        FieldKind::Int { .. } | FieldKind::Float { .. } | FieldKind::Text | FieldKind::Path | FieldKind::Color => {
            apply_text(row, draft)
        }
        FieldKind::Chord => {
            let form = Form::new(
                Vec::new(),
                vec![
                    Button::new(tr!("common-save")),
                    Button::new(tr!("tui-settings-record-clear")),
                    Button::new(tr!("common-cancel")),
                ],
            );
            row.sub = Some(Sub::Keybind(keybind::State {
                title: tr!("tui-settings-record-title", action = path.as_str()),
                target: Some(keybind::Target::Field { path }),
                chords: Vec::new(),
                form,
                inbox: Some(row.child.clone()),
            }));
        }
        FieldKind::EnumList { .. } | FieldKind::TextList | FieldKind::Map => {
            row.sub = list_edit::state(&path, draft, row.child.clone()).map(|s| Sub::List(Box::new(s)));
        }
        // Nested tables and opaque values: the body says "Edit in $EDITOR (Ctrl+E)".
        FieldKind::Table { .. } | FieldKind::Opaque => {}
    }
}

/// "Set secret…" for the credential reference under the cursor.
fn open_secret(row: &mut Row, draft: &Draft) {
    let Some(r) = row.field().filter(|r| r.secret_ref) else { return };
    let path = row.path(draft, r);
    let current = draft.value(&path).and_then(toml::Value::as_str).unwrap_or_default().to_string();
    let name = element_name(draft, &row.table, row.index);
    row.sub = Some(Sub::Secret(secret_set::state(&path, &name, &current)));
}

fn reset(row: &mut Row, draft: &mut Draft) {
    let Some(r) = row.field() else { return };
    let path = row.path(draft, r);
    let i = row.list.cursor;
    change(row, i, || draft.reset(&path));
    load(row, draft);
}

/// Take what the list editor / recorder handed back.
fn drain(row: &mut Row, draft: &mut Draft) {
    let commits: Vec<Commit> = row.child.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
    for c in commits {
        if let Commit::Set { path, value } = c {
            let i = row.list.cursor;
            change(row, i, || draft.set(&path, value));
        }
    }
}

/// What the dialog shown inside did.
fn sub_done(
    row: &mut Row,
    draft: &mut Draft,
    pending: &mut Vec<Pending>,
    closed: bool,
    set: Option<(String, Pending)>,
) {
    if let Some((reference, p)) = set {
        if let Some(r) = row.field() {
            let path = row.path(draft, r);
            let i = row.list.cursor;
            change(row, i, || draft.set(&path, toml::Value::String(reference)));
        }
        pending.push(p);
    }
    drain(row, draft);
    if closed {
        row.sub = None;
        load(row, draft);
    }
}

fn sub_input(core: &mut AppCore, row: &mut Row, draft: &mut Draft, pending: &mut Vec<Pending>, i: &DialogInput) {
    let (closed, set) = match row.sub.as_mut() {
        Some(Sub::List(s)) => (matches!(list_edit::on_input(core, s, i), Outcome::Close(_)), None),
        Some(Sub::Keybind(s)) => (matches!(keybind::on_input(core, s, i), Outcome::Close(_)), None),
        Some(Sub::Secret(s)) => match secret_set::input(core, s, i) {
            secret_set::Done::Keep => (false, None),
            secret_set::Done::Cancel => (true, None),
            secret_set::Done::Set { reference, pending } => (true, Some((reference, pending))),
            secret_set::Done::Generated { reference, pending } => (false, Some((reference, pending))),
        },
        None => return,
    };
    sub_done(row, draft, pending, closed, set);
}

fn sub_mouse(core: &mut AppCore, row: &mut Row, draft: &mut Draft, pending: &mut Vec<Pending>, m: &DialogMouse) {
    let (closed, set) = match row.sub.as_mut() {
        Some(Sub::List(s)) => (matches!(list_edit::on_mouse(core, s, m), Outcome::Close(_)), None),
        Some(Sub::Keybind(s)) => (matches!(keybind::on_mouse(core, s, m), Outcome::Close(_)), None),
        Some(Sub::Secret(s)) => match secret_set::mouse(core, s, m) {
            secret_set::Done::Keep => (false, None),
            secret_set::Done::Cancel => (true, None),
            secret_set::Done::Set { reference, pending } => (true, Some((reference, pending))),
            secret_set::Done::Generated { reference, pending } => (false, Some((reference, pending))),
        },
        None => return,
    };
    sub_done(row, draft, pending, closed, set);
}

fn pressed(row: &mut Row, draft: &mut Draft, b: usize) -> RowDone {
    match b {
        RESET => reset(row, draft),
        SECRET => open_secret(row, draft),
        _ => return RowDone::Close,
    }
    RowDone::Keep
}

/// A key press or paste.
pub fn input(
    core: &mut AppCore,
    row: &mut Row,
    draft: &mut Draft,
    pending: &mut Vec<Pending>,
    i: &DialogInput,
) -> RowDone {
    if row.sub.is_some() {
        sub_input(core, row, draft, pending, i);
        return RowDone::Keep;
    }
    let n = fields(&row.element).len();
    for ev in &i.events {
        if row.sub.is_some() {
            break;
        }
        if let InputEvent::Chord(c) = ev {
            if c.mods.is_empty() && row.list.apply(ev, n, usize::from(MAX_LIST_ROWS)) {
                row.error = None;
                load(row, draft);
                continue;
            }
            if c.mods == Mods::CTRL && c.key == KeyName::E {
                return RowDone::EditConfig;
            }
            if c.mods.is_empty() && c.key == KeyName::Enter && !(row.on_buttons && row.form.focused_button().is_some())
            {
                activate(row, draft);
                continue;
            }
            if c.key == KeyName::Tab {
                row.on_buttons = true;
            }
        }
        match row.form.apply(ev) {
            FormOutcome::Pressed(b) => {
                let done = pressed(row, draft, b);
                if done != RowDone::Keep {
                    return done;
                }
            }
            FormOutcome::Cancel => return RowDone::Close,
            FormOutcome::Changed(_) => {
                if matches!(row.form.fields.first().map(|f| &f.kind), Some(FormField::Text { .. })) {
                    row.error = None;
                } else {
                    apply_pick(row, draft);
                }
            }
            _ => {}
        }
    }
    RowDone::Keep
}

/// A mouse event: a field row selects (a second click opens its editor), the buttons press.
pub fn mouse(
    core: &mut AppCore,
    row: &mut Row,
    draft: &mut Draft,
    pending: &mut Vec<Pending>,
    m: &DialogMouse,
) -> RowDone {
    if row.sub.is_some() {
        sub_mouse(core, row, draft, pending, m);
        return RowDone::Keep;
    }
    let before = row.list.cursor;
    let n = fields(&row.element).len();
    match form_mouse(&mut row.form, Some((&mut row.list, n)), m) {
        FormOutcome::Pressed(b) => return pressed(row, draft, b),
        FormOutcome::Changed(_) => apply_pick(row, draft),
        _ => {
            let down = matches!(m.event.kind, crossterm::event::MouseEventKind::Down(_));
            if let Some(HitTarget::DialogRow(i)) = m.target
                && down
            {
                if i == before {
                    activate(row, draft);
                } else {
                    row.error = None;
                    load(row, draft);
                }
            }
        }
    }
    RowDone::Keep
}

/// Hand the table at `root` to the editor when the draft changed it since `posted`
/// (replacing an earlier, not yet applied value of the same table).
pub fn post_table(core: &AppCore, inbox: Option<&Inbox>, draft: &Draft, root: &str, posted: &mut Option<toml::Value>) {
    let now = draft.value(root).cloned();
    if now == *posted {
        return;
    }
    *posted = now.clone();
    let (Some(inbox), Some(value)) = (inbox, now) else { return };
    if let Ok(mut v) = inbox.lock() {
        v.retain(|c| !matches!(c, Commit::Set { path, .. } if path == root));
        v.push(Commit::Set { path: root.to_string(), value });
    }
    (core.wake)();
}

/// What the standalone dialog keeps (`TuiDialog::TableRow`).
#[derive(Debug, Default)]
pub struct State {
    pub draft: Option<Box<Draft>>,
    pub row: Option<Row>,
    pub inbox: Option<Inbox>,
    pub pending: Vec<Pending>,
    /// The table value last handed to the editor.
    pub posted: Option<toml::Value>,
}

/// Open the form of element `index` of `table` as a dialog of its own; changes go to the
/// settings editor through `inbox`.
pub fn open(app: &mut App, table: &str, index: usize, inbox: Inbox) {
    let draft = Draft::from_config(&app.core.config);
    let Some(row) = Row::new(&draft, table, index) else { return };
    let posted = draft.value(table).cloned();
    app.push_dialog(TuiDialog::TableRow(State {
        draft: Some(Box::new(draft)),
        row: Some(row),
        inbox: Some(inbox),
        pending: Vec::new(),
        posted,
    }));
}

fn standalone(core: &mut AppCore, s: &mut State, done: RowDone) -> Outcome {
    secret_set::poll(core, &mut s.pending);
    if let (Some(d), Some(row)) = (&s.draft, &s.row) {
        let table = row.table.clone();
        post_table(core, s.inbox.as_ref(), d, &table, &mut s.posted);
    }
    match done {
        RowDone::Keep => Outcome::Keep,
        RowDone::Close | RowDone::EditConfig => Outcome::Close(None),
    }
}

/// A key press or paste (standalone).
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let done = match (s.draft.as_mut(), s.row.as_mut()) {
        (Some(d), Some(row)) => input(core, row, d, &mut s.pending, i),
        _ => RowDone::Close,
    };
    standalone(core, s, done)
}

/// A mouse event (standalone).
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    let done = match (s.draft.as_mut(), s.row.as_mut()) {
        (Some(d), Some(row)) => mouse(core, row, d, &mut s.pending, m),
        _ => RowDone::Keep,
    };
    standalone(core, s, done)
}

/// The dialog went away by itself: never.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use soyuz_core::config::Config;

    use super::*;

    fn bookmarks() -> Draft {
        let mut d = Draft::from_config(&Config::default());
        let i = d.table_insert("bookmarks", 0, None).unwrap();
        d.element_set("bookmarks", i, "name", toml::Value::String("Code".into())).unwrap();
        d
    }

    fn at(row: &mut Row, name: &str) {
        row.list.cursor = fields(&row.element).iter().position(|r| r.name == name).unwrap();
    }

    fn type_into(row: &mut Row, text: &str) {
        let f = &mut row.form.fields[0];
        if let FormField::Text { text: t, edit, .. } = &mut f.kind {
            *t = text.to_string();
            *edit = crate::widgets::LineEdit::at_end(text);
        }
    }

    #[test]
    fn fields_are_edited_by_kind_and_validated() {
        let mut d = bookmarks();
        let mut row = Row::new(&d, "bookmarks", 0).unwrap();
        assert_eq!(row.element, "Bookmark");
        at(&mut row, "path");
        load(&mut row, &d);
        type_into(&mut row, "/src");
        apply_text(&mut row, &mut d);
        assert_eq!(d.config.bookmarks[0].path, std::path::PathBuf::from("/src"));
        assert!(row.error.is_none());
        // A second element may not take the first one's name.
        d.table_insert("bookmarks", 1, None).unwrap();
        let mut second = Row::new(&d, "bookmarks", 1).unwrap();
        at(&mut second, "name");
        load(&mut second, &d);
        type_into(&mut second, "Code");
        apply_text(&mut second, &mut d);
        assert!(second.error.is_some());
        assert_eq!(d.config.bookmarks[1].name, "");
        assert!(name_taken(&d, "bookmarks", 1, " Code "));
        assert!(!name_taken(&d, "bookmarks", 0, "Code"));
        assert_eq!(element_name(&d, "bookmarks", 0), "Code");
    }

    #[test]
    fn numbers_choices_and_references() {
        let mut d = Draft::from_config(&Config::default());
        d.table_insert("ai.profiles", 0, None).unwrap();
        let mut row = Row::new(&d, "ai.profiles", 0).unwrap();
        at(&mut row, "max_tokens");
        load(&mut row, &d);
        type_into(&mut row, "many");
        apply_text(&mut row, &mut d);
        assert!(row.error.is_some(), "not a number");
        type_into(&mut row, "4096");
        apply_text(&mut row, &mut d);
        assert_eq!(d.config.ai.profiles[0].max_tokens, 4096);
        // A choice applies as soon as it changes.
        at(&mut row, "kind");
        load(&mut row, &d);
        let before = d.config.ai.profiles[0].kind;
        let right = DialogInput { key: None, events: vec![InputEvent::Chord("Right".parse().unwrap())] };
        let mut f = row.form.clone();
        assert!(matches!(f.apply(&right.events[0]), FormOutcome::Changed(0)));
        row.form = f;
        apply_pick(&mut row, &mut d);
        assert_ne!(d.config.ai.profiles[0].kind, before);
        // The key field takes references only: a raw key is refused.
        at(&mut row, "key");
        load(&mut row, &d);
        assert!(row.field().unwrap().secret_ref);
        assert!(row.form.buttons[SECRET].enabled);
        type_into(&mut row, "sk-raw-secret");
        apply_text(&mut row, &mut d);
        assert!(row.error.is_some());
        assert_eq!(d.config.ai.profiles[0].key, "");
        type_into(&mut row, "env:MY_KEY");
        apply_text(&mut row, &mut d);
        assert_eq!(d.config.ai.profiles[0].key, "env:MY_KEY");
    }
}
