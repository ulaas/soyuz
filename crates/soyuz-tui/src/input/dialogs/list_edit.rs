//! The list editor of the settings editor (`TuiDialog::ListEdit`): the items of a list key
//! (`terminal.args`, `tui.cell_pixels`), of a list of choices (`columns.details`) or of a
//! `name = value` map (`theme.colors`, `columns.widths`).
//!
//! The field below the list edits the item under the cursor (moving the cursor loads it):
//! Enter puts the field into that item, Add (Alt+A) adds it as a new item after the
//! cursor, Insert clears the field for a new item (Enter then adds it). A list of choices picks the value with Left / Right. Alt+Up / Alt+Down move the
//! item, Remove (Alt+R) deletes it, Save (Alt+S) checks the whole value against the live
//! config and hands it to the editor through its [`Inbox`]; Esc cancels.

use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::settings_draft::Draft;
use soyuz_appcore::models::settings_schema::{self as schema, FieldKind};
use soyuz_appcore::tr;
use soyuz_core::config::{KeyName, Mods};

use super::{DialogInput, DialogMouse, Outcome, TuiDialog, form_mouse};
use crate::app::App;
use crate::input::settings::{Commit, Inbox, edit_text, post};
use crate::widgets::{Button, Field, FieldKind as FormField, Form, FormOutcome, LineEdit, SelectList};

/// The type of the numbers in a list or map (`None`: text).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Num {
    #[default]
    Text,
    Int,
    Float,
}

/// What the items are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListKind {
    /// Strings or numbers.
    Items(Num),
    /// Values of a choice (serialized name, doc).
    Choices(Vec<(String, String)>),
    /// `name = value` entries.
    Map(Num),
}

impl Default for ListKind {
    fn default() -> ListKind {
        ListKind::Items(Num::Text)
    }
}

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    pub title: String,
    /// The key's path.
    pub path: String,
    pub kind: ListKind,
    /// The items as text (maps: `name = value`).
    pub items: Vec<String>,
    pub list: SelectList,
    /// Field 0: the item (text, or the choice); buttons Add, Remove, Save, Cancel.
    pub form: Form,
    /// The item the field holds (`None`: a new item).
    pub editing: Option<usize>,
    pub inbox: Option<Inbox>,
    /// What Save checks the value against (`None`: the live config). The table editor's
    /// element forms pass their own draft: a new element is not live yet.
    pub base: Option<Box<Draft>>,
}

/// Button indices.
const ADD: usize = 0;
const REMOVE: usize = 1;
const SAVE: usize = 2;

fn num_of(v: Option<&toml::Value>) -> Option<Num> {
    match v? {
        toml::Value::Integer(_) => Some(Num::Int),
        toml::Value::Float(_) => Some(Num::Float),
        toml::Value::String(_) => Some(Num::Text),
        _ => None,
    }
}

/// The kind and items of the value `v` (falling back to `default` for the item type).
pub fn read(kind: &FieldKind, v: Option<&toml::Value>, default: Option<&toml::Value>) -> (ListKind, Vec<String>) {
    let first = |v: Option<&toml::Value>| match v {
        Some(toml::Value::Array(a)) => a.first().cloned(),
        Some(toml::Value::Table(t)) => t.values().next().cloned(),
        _ => None,
    };
    let num = num_of(first(v).as_ref()).or_else(|| num_of(first(default).as_ref())).unwrap_or_default();
    let list_kind = match kind {
        FieldKind::EnumList { values } => ListKind::Choices(values.clone()),
        FieldKind::Map => ListKind::Map(num),
        _ => ListKind::Items(num),
    };
    let items = match v {
        Some(toml::Value::Array(a)) => a.iter().map(edit_text).collect(),
        Some(toml::Value::Table(t)) => t.iter().map(|(k, v)| format!("{k} = {}", edit_text(v))).collect(),
        _ => Vec::new(),
    };
    (list_kind, items)
}

fn number(num: Num, s: &str) -> Result<toml::Value, String> {
    let t = s.trim();
    let bad = || tr!("tui-settings-not-a-number", text = t);
    match num {
        Num::Text => Ok(toml::Value::String(s.to_string())),
        Num::Int => t.parse::<i64>().map(toml::Value::Integer).map_err(|_| bad()),
        Num::Float => {
            t.replace(',', ".").parse::<f64>().ok().filter(|f| f.is_finite()).map(toml::Value::Float).ok_or_else(bad)
        }
    }
}

/// The value the items describe (an item that is not a number where one is needed, or a
/// map entry without `=`, is an error naming it).
pub fn to_value(kind: &ListKind, items: &[String]) -> Result<toml::Value, String> {
    match kind {
        ListKind::Items(num) => {
            Ok(toml::Value::Array(items.iter().map(|i| number(*num, i)).collect::<Result<_, _>>()?))
        }
        ListKind::Choices(_) => Ok(toml::Value::Array(items.iter().map(|i| toml::Value::String(i.clone())).collect())),
        ListKind::Map(num) => {
            let mut t = toml::Table::new();
            for i in items {
                let Some((k, v)) = i.split_once('=') else {
                    return Err(tr!("tui-settings-list-map-entry", entry = i.as_str()));
                };
                let k = k.trim();
                if k.is_empty() {
                    return Err(tr!("tui-settings-list-map-entry", entry = i.as_str()));
                }
                t.insert(k.to_string(), number(*num, v.trim())?);
            }
            Ok(toml::Value::Table(t))
        }
    }
}

/// Open the list editor for the key at `path`.
pub fn open(app: &mut App, path: &str, inbox: Inbox) {
    let base = Draft::from_config(&app.core.config);
    if let Some(mut s) = state(path, &base, inbox) {
        s.base = None;
        app.push_dialog(TuiDialog::ListEdit(s));
    }
}

/// The list editor for the key at `path` of `base` (Save checks against `base`), for a
/// dialog that shows it inside its own (the table editor's element form).
pub fn state(path: &str, base: &Draft, inbox: Inbox) -> Option<State> {
    let row = schema::row(path)?;
    let (kind, items) = read(&row.kind, base.value(path), Draft::default_value(path).as_ref());
    let field = match &kind {
        ListKind::Choices(values) => {
            Field::radio(tr!("tui-settings-list-item"), values.iter().map(|(v, _)| v.clone()).collect(), 0)
        }
        _ => Field::text(tr!("tui-settings-list-item"), items.first().cloned().unwrap_or_default()),
    };
    let form = Form::new(
        vec![field],
        vec![
            Button::new(tr!("tui-settings-list-add")).accel('a'),
            Button::new(tr!("tui-settings-list-remove")).accel('r'),
            Button::new(tr!("common-save")).accel('s'),
            Button::new(tr!("common-cancel")),
        ],
    );
    let mut s = State {
        title: crate::dialog::safe(path),
        path: path.to_string(),
        kind,
        items,
        list: SelectList::new(),
        form,
        editing: None,
        inbox: Some(inbox),
        base: Some(Box::new(base.clone())),
    };
    load(&mut s);
    Some(s)
}

/// Put the cursor's item into the field.
fn load(s: &mut State) {
    let Some(item) = s.items.get(s.list.cursor).cloned() else {
        s.editing = None;
        return;
    };
    s.editing = Some(s.list.cursor);
    set_field(s, &item);
}

fn set_field(s: &mut State, text: &str) {
    let Some(f) = s.form.fields.first_mut() else { return };
    match &mut f.kind {
        FormField::Text { text: t, edit, .. } => {
            *t = text.to_string();
            *edit = LineEdit::at_end(text);
        }
        FormField::Radio { options, selected } => {
            if let Some(i) = options.iter().position(|o| o == text) {
                *selected = i;
            }
        }
        FormField::Check(_) => {}
    }
}

fn field_value(s: &State) -> String {
    match s.form.fields.first().map(|f| &f.kind) {
        Some(FormField::Text { text, .. }) => text.clone(),
        Some(FormField::Radio { options, selected }) => options.get(*selected).cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

/// Enter: the field replaces the item it holds, or becomes a new item after the cursor.
fn apply_field(s: &mut State) {
    let v = field_value(s);
    if v.trim().is_empty() {
        return;
    }
    if matches!(s.kind, ListKind::Choices(_)) && s.items.contains(&v) && s.editing.is_none_or(|i| s.items[i] != v) {
        return;
    }
    match s.editing.filter(|&i| i < s.items.len()) {
        Some(i) => s.items[i] = v,
        None => {
            let at = if s.items.is_empty() { 0 } else { (s.list.cursor + 1).min(s.items.len()) };
            s.items.insert(at, v);
            s.list.select(at, s.items.len());
            s.editing = Some(at);
        }
    }
    s.list.show_cursor(usize::from(crate::view::dialogs::MAX_LIST_ROWS));
    s.form.error = None;
}

/// Start a new item (Insert, Add).
fn new_item(s: &mut State) {
    s.editing = None;
    if !matches!(s.kind, ListKind::Choices(_)) {
        set_field(s, "");
    }
    s.form.focus = 0;
}

fn remove(s: &mut State) {
    if s.list.cursor < s.items.len() {
        s.items.remove(s.list.cursor);
        s.list.clamp(s.items.len(), usize::from(crate::view::dialogs::MAX_LIST_ROWS));
        load(s);
    }
}

fn move_item(s: &mut State, down: bool) {
    let i = s.list.cursor;
    let j = if down { i + 1 } else { i.wrapping_sub(1) };
    if i < s.items.len() && j < s.items.len() {
        s.items.swap(i, j);
        s.list.select(j, s.items.len());
        s.editing = Some(j);
        s.list.show_cursor(usize::from(crate::view::dialogs::MAX_LIST_ROWS));
    }
}

/// Save: check the value against the live config, hand it over.
fn save(core: &AppCore, s: &mut State) -> Outcome {
    let checked = to_value(&s.kind, &s.items).and_then(|v| {
        let mut d = s.base.as_deref().cloned().unwrap_or_else(|| Draft::from_config(&core.config));
        d.set(&s.path, v.clone()).map(|()| v)
    });
    match checked {
        Ok(value) => {
            if let Some(inbox) = &s.inbox {
                post(inbox, core, Commit::Set { path: s.path.clone(), value });
            }
            Outcome::Close(None)
        }
        Err(e) => {
            s.form.error = Some(tr!("tui-settings-invalid", error = e));
            Outcome::Keep
        }
    }
}

fn pressed(core: &AppCore, s: &mut State, b: usize) -> Outcome {
    match b {
        ADD => {
            // The field becomes a new item after the cursor.
            s.editing = None;
            apply_field(s);
            Outcome::Keep
        }
        REMOVE => {
            remove(s);
            Outcome::Keep
        }
        SAVE => save(core, s),
        _ => Outcome::Close(None),
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    let rows = usize::from(crate::view::dialogs::MAX_LIST_ROWS);
    for ev in &i.events {
        if let InputEvent::Chord(c) = ev {
            let plain = c.mods.is_empty();
            if plain && s.list.apply(ev, s.items.len(), rows) {
                load(s);
                continue;
            }
            if c.mods == Mods::ALT && matches!(c.key, KeyName::ArrowUp | KeyName::ArrowDown) {
                move_item(s, c.key == KeyName::ArrowDown);
                continue;
            }
            if plain && c.key == KeyName::Insert {
                new_item(s);
                continue;
            }
            if plain && c.key == KeyName::Enter && s.form.focused_button().is_none() {
                apply_field(s);
                continue;
            }
        }
        match s.form.apply(ev) {
            FormOutcome::Pressed(b) => {
                if let Outcome::Close(c) = pressed(core, s, b) {
                    return Outcome::Close(c);
                }
            }
            FormOutcome::Cancel => return Outcome::Close(None),
            _ => {}
        }
    }
    Outcome::Keep
}

/// A mouse event: a row loads its item, the buttons press.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    let before = s.list.cursor;
    let len = s.items.len();
    match form_mouse(&mut s.form, Some((&mut s.list, len)), m) {
        FormOutcome::Pressed(b) => pressed(core, s, b),
        _ => {
            if s.list.cursor != before
                || matches!(m.target, Some(crate::layout::HitTarget::DialogRow(_)))
                    && matches!(m.event.kind, crossterm::event::MouseEventKind::Down(_))
            {
                load(s);
            }
            Outcome::Keep
        }
    }
}

/// The dialog went away by itself: never.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(kind: ListKind, items: &[&str]) -> State {
        let field = match &kind {
            ListKind::Choices(v) => Field::radio("Item", v.iter().map(|(a, _)| a.clone()).collect(), 0),
            _ => Field::text("Item", ""),
        };
        let mut s = State {
            kind,
            items: items.iter().map(|s| s.to_string()).collect(),
            form: Form::new(vec![field], vec![Button::new("Add"), Button::new("Remove"), Button::new("Save")]),
            ..State::default()
        };
        load(&mut s);
        s
    }

    #[test]
    fn values_round_trip() {
        let arr = toml::Value::Array(vec![toml::Value::Integer(10), toml::Value::Integer(20)]);
        let (kind, items) = read(&FieldKind::TextList, Some(&arr), None);
        assert_eq!(kind, ListKind::Items(Num::Int));
        assert_eq!(items, ["10", "20"]);
        assert_eq!(to_value(&kind, &items).unwrap(), arr);
        assert!(to_value(&kind, &["x".into()]).is_err());
        // An empty list takes its item type from the default.
        let empty = toml::Value::Array(Vec::new());
        let (kind, _) = read(&FieldKind::TextList, Some(&empty), Some(&arr));
        assert_eq!(kind, ListKind::Items(Num::Int));
        let mut map = toml::Table::new();
        map.insert("name".into(), toml::Value::Float(240.0));
        let map = toml::Value::Table(map);
        let (kind, items) = read(&FieldKind::Map, Some(&map), None);
        assert_eq!(kind, ListKind::Map(Num::Float));
        assert_eq!(items, ["name = 240"]);
        assert_eq!(to_value(&kind, &items).unwrap(), map);
        assert!(to_value(&kind, &["no equals".into()]).is_err());
        assert!(to_value(&ListKind::Map(Num::Text), &["a = #fff".into()]).is_ok());
    }

    #[test]
    fn enter_edits_insert_adds_and_items_move() {
        let mut s = st(ListKind::Items(Num::Text), &["-l", "-i"]);
        assert_eq!(field_value(&s), "-l");
        set_field(&mut s, "--login");
        apply_field(&mut s);
        assert_eq!(s.items, ["--login", "-i"]);
        new_item(&mut s);
        set_field(&mut s, "-x");
        apply_field(&mut s);
        assert_eq!(s.items, ["--login", "-x", "-i"]);
        assert_eq!(s.list.cursor, 1);
        move_item(&mut s, true);
        assert_eq!(s.items, ["--login", "-i", "-x"]);
        remove(&mut s);
        assert_eq!(s.items, ["--login", "-i"]);
        // Choices never repeat.
        let values = vec![("name".to_string(), String::new()), ("size".to_string(), String::new())];
        let mut c = st(ListKind::Choices(values), &["name"]);
        new_item(&mut c);
        apply_field(&mut c);
        assert_eq!(c.items, ["name"]);
    }
}
