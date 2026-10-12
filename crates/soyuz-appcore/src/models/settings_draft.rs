//! A generic, path-addressed editing copy of the config for a generated settings editor:
//! read and write any key by its dotted path (`"tui.borders"`, `"bookmarks[2].hotkey"`),
//! validated by deserializing the whole `Config` (the error names the key), plus the
//! array-of-tables operations of the table editor (insert, duplicate, remove, reorder).
//!
//! [`apply`] makes a draft's config live and schedules the debounced `config.toml` write.
//! [`store_secret`] puts a secret into the OS credential store and hands back the
//! `credential:<name>` reference that goes into the config instead.

use crossbeam_channel::Receiver;
use soyuz_ai::Secret;
use soyuz_ai::credentials::{self, KeyRef};
use soyuz_core::config::{self, Config};

use crate::core::AppCore;
use crate::host::{Host, Wake};
use crate::models::settings::{SAVE_DELAY, Section};
use crate::models::settings_schema::{
    self as schema, FieldKind, default_table, get_in, get_in_mut, remove_in, set_in, to_table,
};

/// The editing copy.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    /// The config as TOML (every key written), canonical after each successful edit.
    pub table: toml::Table,
    /// The validated config the table describes.
    pub config: Config,
    /// The last failed edit: `(path, message)`; cleared by the next successful one.
    pub error: Option<(String, String)>,
}

impl Draft {
    pub fn from_config(c: &Config) -> Draft {
        Draft { table: to_table(c), config: c.clone(), error: None }
    }

    /// The value at `path` (`None` for an unset optional key or a missing path).
    pub fn value(&self, path: &str) -> Option<&toml::Value> {
        get_in(&self.table, path)
    }

    /// The default value at `path`. Element fields (`"bookmarks[2].hotkey"`) answer with
    /// the field's value in a fresh element.
    pub fn default_value(path: &str) -> Option<toml::Value> {
        if let Some((table, field)) = split_element(path) {
            let el = schema::element_default(table)?;
            return get_in(&el, field).cloned();
        }
        get_in(default_table(), path).cloned()
    }

    pub fn is_default(&self, path: &str) -> bool {
        self.value(path).cloned() == Self::default_value(path)
    }

    /// Set `path` to `v` and validate. Integers and floats follow the key's current type
    /// (`1` into a float key is `1.0`); numbers are clamped to the key's range. On error
    /// nothing changes and the message names the key.
    pub fn set(&mut self, path: &str, v: toml::Value) -> Result<(), String> {
        let current = self.value(path).cloned().or_else(|| Self::default_value(path));
        let v = match current {
            Some(cur) => coerce(&cur, v).map_err(|e| self.fail(path, e))?,
            None => v,
        };
        let v = clamp(path, v);
        let mut t = self.table.clone();
        if !set_in(&mut t, path, v) {
            return Err(self.fail(path, "no such key".into()));
        }
        self.commit(t, path)
    }

    /// Back to the default (an optional key without a default is removed).
    pub fn reset(&mut self, path: &str) -> Result<(), String> {
        let mut t = self.table.clone();
        reset_in(&mut t, path);
        self.commit(t, path)
    }

    /// Reset a whole table (`"tui"`, `"search.index"`) or key to its default.
    pub fn reset_table(&mut self, table: &str) -> Result<(), String> {
        self.reset(table)
    }

    /// Reset a settings page: its [`Section::tables`] except [`Section::kept`]
    /// (Advanced resets everything), exactly what [`Section::reset`] does.
    pub fn reset_section(&mut self, section: Section) -> Result<(), String> {
        let mut t = if section == Section::Advanced {
            default_table().clone()
        } else {
            let mut t = self.table.clone();
            let kept: Vec<(&str, Option<toml::Value>)> =
                section.kept().iter().map(|p| (*p, get_in(&t, p).cloned())).collect();
            for p in section.tables() {
                reset_in(&mut t, p);
            }
            for (p, v) in kept {
                match v {
                    Some(v) => {
                        set_in(&mut t, p, v);
                    }
                    None => remove_in(&mut t, p),
                }
            }
            t
        };
        // The schema version is not a setting.
        if let Some(v) = self.table.get("schema") {
            t.insert("schema".into(), v.clone());
        }
        self.commit(t, section.id())
    }

    /// Next (or previous) value of an enum key, wrapping.
    pub fn cycle_enum(&mut self, path: &str, forward: bool) -> Result<(), String> {
        let values = match schema::row(path).map(|r| &r.kind) {
            Some(FieldKind::Enum { values }) => values,
            _ => return Err(self.fail(path, "not an enumeration".into())),
        };
        let cur = self.value(path).and_then(toml::Value::as_str).unwrap_or_default().to_string();
        let next = next_value(values, &cur, forward).to_string();
        self.set(path, toml::Value::String(next))
    }

    /// Add `delta` to a number (clamped to its range; floats rounded to 2 decimals).
    pub fn nudge(&mut self, path: &str, delta: f64) -> Result<(), String> {
        let v = match self.value(path).cloned().or_else(|| Self::default_value(path)) {
            Some(toml::Value::Integer(i)) => toml::Value::Integer(i.saturating_add(delta.round() as i64)),
            Some(toml::Value::Float(f)) => toml::Value::Float(((f + delta) * 100.0).round() / 100.0),
            _ => return Err(self.fail(path, "not a number".into())),
        };
        self.set(path, v)
    }

    /// Flip a boolean.
    pub fn toggle(&mut self, path: &str) -> Result<(), String> {
        match self.value(path).and_then(toml::Value::as_bool) {
            Some(b) => self.set(path, toml::Value::Boolean(!b)),
            None => Err(self.fail(path, "not a switch".into())),
        }
    }

    /// Elements in the array / map of tables at `table` (0 when absent).
    pub fn table_len(&self, table: &str) -> usize {
        match self.value(table) {
            Some(toml::Value::Array(a)) => a.len(),
            Some(toml::Value::Table(t)) => t.len(),
            _ => 0,
        }
    }

    /// The key of element `index` of a map of tables (sorted, as TOML writes them).
    pub fn element_key(&self, table: &str, index: usize) -> Option<String> {
        self.value(table)?.as_table()?.keys().nth(index).cloned()
    }

    /// Insert a fresh element (defaults) at `at` (clamped); a map of tables needs `name`.
    /// Returns the new element's index.
    pub fn table_insert(&mut self, table: &str, at: usize, name: Option<&str>) -> Result<usize, String> {
        let Some(el) = schema::element_default(table) else {
            return Err(self.fail(table, "not a list of tables".into()));
        };
        self.insert_element(table, at, name, el)
    }

    /// Copy element `index` right after it (a `name` field gets a unique suffix).
    pub fn table_duplicate(&mut self, table: &str, index: usize) -> Result<usize, String> {
        let Some(mut el) = self.element(table, index).cloned() else {
            return Err(self.fail(table, format!("no element {index}")));
        };
        let keyed = self.value(table).is_some_and(toml::Value::is_table);
        let taken: Vec<String> = (0..self.table_len(table))
            .filter_map(|i| {
                if keyed {
                    self.element_key(table, i)
                } else {
                    self.element(table, i)?.get("name")?.as_str().map(str::to_string)
                }
            })
            .collect();
        let base = if keyed {
            self.element_key(table, index)
        } else {
            el.get("name").and_then(toml::Value::as_str).map(str::to_string)
        };
        let unique = base
            .filter(|b| !b.is_empty())
            .map(|b| (2..).map(|n| format!("{b} {n}")).find(|c| !taken.contains(c)).expect("an unused name exists"));
        if !keyed && let Some(u) = &unique {
            el.insert("name".into(), toml::Value::String(u.clone()));
        }
        self.insert_element(table, index + 1, unique.as_deref(), el)
    }

    pub fn table_remove(&mut self, table: &str, index: usize) -> Result<(), String> {
        let mut t = self.table.clone();
        let ok = match get_in_mut(&mut t, table) {
            Some(toml::Value::Array(a)) if index < a.len() => {
                a.remove(index);
                true
            }
            Some(toml::Value::Table(m)) => match m.keys().nth(index).cloned() {
                Some(k) => m.remove(&k).is_some(),
                None => false,
            },
            _ => false,
        };
        if !ok {
            return Err(self.fail(table, format!("no element {index}")));
        }
        self.commit(t, table)
    }

    /// Move element `from` to position `to` (arrays only; maps are sorted by name).
    pub fn table_move(&mut self, table: &str, from: usize, to: usize) -> Result<(), String> {
        let mut t = self.table.clone();
        match get_in_mut(&mut t, table) {
            Some(toml::Value::Array(a)) if from < a.len() && to < a.len() => {
                let el = a.remove(from);
                a.insert(to, el);
            }
            _ => return Err(self.fail(table, format!("cannot move {from} to {to}"))),
        }
        self.commit(t, table)
    }

    /// Set `field` (dotted inside the element: `"hotkey"`, `"sub.key"`) of element `index`.
    pub fn element_set(&mut self, table: &str, index: usize, field: &str, v: toml::Value) -> Result<(), String> {
        let path = self.element_path(table, index, field);
        self.set(&path, v)
    }

    /// The value of `field` of element `index`.
    pub fn element_value(&self, table: &str, index: usize, field: &str) -> Option<&toml::Value> {
        get_in(self.element(table, index)?, field)
    }

    /// The draft path of `field` of element `index` (`"bookmarks[2].hotkey"`; maps:
    /// `"table.<key>.field"`), for [`Draft::set`], [`Draft::cycle_enum`], … .
    pub fn element_path(&self, table: &str, index: usize, field: &str) -> String {
        match self.element_key(table, index) {
            Some(k) => format!("{table}.{k}.{field}"),
            None => format!("{table}[{index}].{field}"),
        }
    }

    fn element(&self, table: &str, index: usize) -> Option<&toml::Table> {
        match self.value(table)? {
            toml::Value::Array(a) => a.get(index)?.as_table(),
            toml::Value::Table(m) => m.values().nth(index)?.as_table(),
            _ => None,
        }
    }

    fn insert_element(&mut self, table: &str, at: usize, name: Option<&str>, el: toml::Table) -> Result<usize, String> {
        let keyed = schema::row(table).is_some_and(|r| matches!(r.kind, FieldKind::Table { keyed: true, .. }));
        let mut t = self.table.clone();
        if get_in(&t, table).is_none() {
            let empty = if keyed { toml::Value::Table(toml::Table::new()) } else { toml::Value::Array(Vec::new()) };
            set_in(&mut t, table, empty);
        }
        let index = match get_in_mut(&mut t, table) {
            Some(toml::Value::Array(a)) => {
                let at = at.min(a.len());
                a.insert(at, toml::Value::Table(el));
                at
            }
            Some(toml::Value::Table(m)) => {
                let Some(name) = name.filter(|n| !n.trim().is_empty()) else {
                    return Err(self.fail(table, "a name is needed".into()));
                };
                if m.contains_key(name) {
                    return Err(self.fail(table, format!("{name} exists")));
                }
                m.insert(name.to_string(), toml::Value::Table(el));
                m.keys().position(|k| k == name).unwrap_or_default()
            }
            _ => return Err(self.fail(table, "not a list of tables".into())),
        };
        self.commit(t, table)?;
        Ok(index)
    }

    /// Validate `t` as a config; on success it becomes the draft.
    fn commit(&mut self, t: toml::Table, path: &str) -> Result<(), String> {
        let text = toml::to_string(&t).map_err(|e| self.fail(path, e.to_string()))?;
        let loaded = config::parse_str(&text).map_err(|e| {
            let msg = match e {
                config::ConfigError::Parse { msg, .. } => msg,
                other => other.to_string(),
            };
            self.fail(path, msg.trim().to_string())
        })?;
        if let Some(w) = loaded.warnings.iter().find_map(|w| w.strip_prefix("unknown config key: ")) {
            return Err(self.fail(w, "unknown key".into()));
        }
        self.table = to_table(&loaded.config);
        self.config = loaded.config;
        self.error = None;
        Ok(())
    }

    fn fail(&mut self, path: &str, msg: String) -> String {
        let text = format!("{path}: {msg}");
        self.error = Some((path.to_string(), msg));
        text
    }
}

/// `"x[2].field"` → `("x", "field")` (only the innermost array of tables).
fn split_element(path: &str) -> Option<(&str, &str)> {
    let (head, field) = path.rsplit_once("].")?;
    let open = head.rfind('[')?;
    Some((&head[..open], field))
}

fn reset_in(t: &mut toml::Table, path: &str) {
    match Draft::default_value(path) {
        Some(v) => {
            set_in(t, path, v);
        }
        None => remove_in(t, path),
    }
}

/// The value after (or before) `current` in `values`, wrapping; the first value when
/// `current` is not one of them.
pub fn next_value<'a>(values: &'a [(String, String)], current: &str, forward: bool) -> &'a str {
    let Some(first) = values.first() else { return "" };
    let n = values.len();
    match values.iter().position(|(v, _)| v == current) {
        Some(i) if forward => &values[(i + 1) % n].0,
        Some(i) => &values[(i + n - 1) % n].0,
        None => &first.0,
    }
}

/// `v` in the TOML type of `cur` where numbers differ (int ↔ float), element-wise for arrays.
fn coerce(cur: &toml::Value, v: toml::Value) -> Result<toml::Value, String> {
    use toml::Value as V;
    Ok(match (cur, v) {
        (V::Float(_), V::Integer(i)) => V::Float(i as f64),
        (V::Integer(_), V::Float(f)) => {
            if f.fract() != 0.0 || !f.is_finite() {
                return Err(format!("{f} is not a whole number"));
            }
            V::Integer(f as i64)
        }
        (V::Array(c), V::Array(items)) if !c.is_empty() => {
            V::Array(items.into_iter().map(|i| coerce(&c[0], i)).collect::<Result<_, _>>()?)
        }
        (_, v) => v,
    })
}

/// Clamp a number (or a list of numbers) to the key's range hint.
fn clamp(path: &str, v: toml::Value) -> toml::Value {
    use toml::Value as V;
    let range = schema::HINTS
        .iter()
        .find(|(p, h)| *p == schema::normalize(path) && matches!(h, schema::Hint::Range(..)))
        .map(|(_, h)| *h);
    let Some(schema::Hint::Range(lo, hi)) = range else { return v };
    let one = |v: V| match v {
        V::Integer(i) => V::Integer(i.clamp(lo as i64, hi as i64)),
        V::Float(f) => V::Float(f.clamp(lo, hi)),
        other => other,
    };
    match v {
        V::Array(items) => V::Array(items.into_iter().map(one).collect()),
        other => one(other),
    }
}

/// Make `c` the live config (theme, key map, panels, …) and write `config.toml` shortly
/// after (debounced, comments kept). No-op when nothing changed.
pub fn apply(app: &mut AppCore, host: &dyn Host, c: Config) {
    if c == app.config {
        return;
    }
    crate::services::config_watch::apply_config(app, host, c);
    app.config_save.schedule();
    host.redraw_after(SAVE_DELAY);
}

/// The credential-store name for the secret of a credential-reference row (`row_path` as
/// in the schema: `"ai.profiles[].key"`, `"ai.mcp_clients[].token"`,
/// `"ai.mcp_servers[].token"`) of the element named `element_name`, the way the GUI names it.
pub fn secret_name(row_path: &str, element_name: &str) -> String {
    match schema::normalize(row_path).as_str() {
        "ai.mcp_clients[].token" => soyuz_mcp::trust::token_credential(element_name),
        "ai.mcp_servers[].token" => crate::services::ai_service::credential_name(&format!("mcp-server-{element_name}")),
        _ => crate::services::ai_service::credential_name(element_name),
    }
}

/// Store `secret` in the OS credential store (on a worker thread; `wake` when done) and
/// return the reference to put into the config: the current reference's name when it
/// already is a `credential:<name>`, else `name`. The secret itself never reaches the
/// config. (Settings → AI in the GUI goes through the same `ai_service::store_secret`.)
pub fn store_secret(
    current_ref: &str,
    name: String,
    secret: Secret,
    wake: Wake,
) -> (String, Receiver<Result<(), String>>) {
    let name = match credentials::parse_ref(current_ref) {
        Ok(KeyRef::Credential(n)) => n,
        _ => name,
    };
    (format!("credential:{name}"), crate::services::ai_service::store_secret(name.clone(), secret, wake))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_core::config::{Bookmark, TuiBorders};

    fn draft() -> Draft {
        Draft::from_config(&Config::default())
    }

    #[test]
    fn set_validates_by_deserializing_and_names_the_key() {
        let mut d = draft();
        d.set("tui.borders", toml::Value::String("plain".into())).unwrap();
        assert_eq!(d.config.tui.borders, TuiBorders::Plain);
        let err = d.set("tui.borders", toml::Value::String("wobbly".into())).unwrap_err();
        assert!(err.starts_with("tui.borders: "), "{err}");
        assert_eq!(d.config.tui.borders, TuiBorders::Plain, "failed edit leaves the draft alone");
        assert_eq!(d.error.as_ref().map(|(p, _)| p.as_str()), Some("tui.borders"));
        let err = d.set("general.show_hidden", toml::Value::String("yes".into())).unwrap_err();
        assert!(err.starts_with("general.show_hidden: "), "{err}");
        let err = d.set("tui.no_such_key", toml::Value::Boolean(true)).unwrap_err();
        assert!(err.contains("tui.no_such_key"), "{err}");
        d.set("tui.borders", toml::Value::String("rounded".into())).unwrap();
        assert!(d.error.is_none());
    }

    #[test]
    fn every_enum_value_round_trips() {
        let s = schema::build();
        let mut n = 0;
        for t in &s.tables {
            for r in &t.rows {
                let FieldKind::Enum { values } = &r.kind else { continue };
                let mut d = draft();
                for (v, _) in values {
                    d.set(&r.path, toml::Value::String(v.clone())).unwrap_or_else(|e| panic!("{e}"));
                    assert_eq!(d.value(&r.path).and_then(toml::Value::as_str), Some(v.as_str()), "{}", r.path);
                    n += 1;
                }
                // cycle_enum walks the same list and wraps.
                let start = d.value(&r.path).cloned();
                for _ in 0..values.len() {
                    d.cycle_enum(&r.path, true).unwrap();
                }
                assert_eq!(d.value(&r.path).cloned(), start, "{}", r.path);
            }
        }
        assert!(n > 50, "{n}");
    }

    #[test]
    fn numbers_keep_their_type_and_range() {
        let mut d = draft();
        d.set("tui.preview_ratio", toml::Value::Integer(0)).unwrap();
        assert!(matches!(d.value("tui.preview_ratio"), Some(toml::Value::Float(_))));
        assert_eq!(d.config.tui.preview_ratio, 0.2, "clamped to the range");
        d.set("tui.sidebar_width", toml::Value::Float(30.0)).unwrap();
        assert_eq!(d.value("tui.sidebar_width"), Some(&toml::Value::Integer(30)));
        assert!(d.set("tui.sidebar_width", toml::Value::Float(30.5)).is_err());
        d.set("tui.sidebar_width", toml::Value::Integer(1000)).unwrap();
        assert_eq!(d.config.tui.sidebar_width, TuiSection::SIDEBAR_WIDTH_MAX);
        d.nudge("tui.sidebar_width", -5.0).unwrap();
        assert_eq!(d.config.tui.sidebar_width, TuiSection::SIDEBAR_WIDTH_MAX - 5);
        d.nudge("tui.ai_ratio", 0.05).unwrap();
        assert!((d.config.tui.ai_ratio - 0.45).abs() < 1e-6, "{}", d.config.tui.ai_ratio);
        d.set("tui.cell_pixels", toml::Value::Array(vec![toml::Value::Integer(0), toml::Value::Integer(15)])).unwrap();
        assert_eq!(d.config.tui.cell_pixels, [1, 15]);
        let before = d.config.general.show_hidden;
        d.toggle("general.show_hidden").unwrap();
        assert_eq!(d.config.general.show_hidden, !before);
        assert!(!d.is_default("general.show_hidden"));
        d.reset("general.show_hidden").unwrap();
        assert!(d.is_default("general.show_hidden"));
    }

    use soyuz_core::config::TuiSection;

    #[test]
    fn reset_section_equals_section_reset() {
        // A config with every scalar key changed (where the change deserializes) and
        // user data in the kept lists.
        let mut base = draft();
        for t in &schema::build().tables {
            for r in &t.rows {
                let Some(v) = base.value(&r.path).cloned() else { continue };
                let changed = match (&r.kind, v) {
                    (FieldKind::Bool, toml::Value::Boolean(b)) => toml::Value::Boolean(!b),
                    (FieldKind::Int { .. }, toml::Value::Integer(i)) => {
                        toml::Value::Integer(if i > 1 { i - 1 } else { i + 1 })
                    }
                    (FieldKind::Float { .. }, toml::Value::Float(f)) => toml::Value::Float(f * 0.9 + 0.01),
                    (FieldKind::Enum { values }, toml::Value::String(s)) => {
                        toml::Value::String(next_value(values, &s, true).to_string())
                    }
                    _ => continue,
                };
                let _ = base.set(&r.path, changed);
            }
        }
        base.error = None;
        let mut c = base.config.clone();
        c.bookmarks.push(Bookmark { name: "b".into(), path: "/b".into(), ..Default::default() });
        c.sidebar.portal.push(soyuz_core::config::PortalPin { path: "/p".into(), name: String::new() });
        c.ai.profiles.push(soyuz_core::config::AiProfile { name: "p".into(), ..Default::default() });
        c.automation.rules.push(soyuz_core::config::AutomationRule { name: "r".into(), ..Default::default() });
        c.search.saved.push(soyuz_core::config::SavedSearchEntry { name: "s".into(), query: toml::Table::new() });
        assert_ne!(c, Config::default());
        for sec in Section::ALL {
            let mut expected = c.clone();
            sec.reset(&mut expected);
            let mut d = Draft::from_config(&c);
            d.reset_section(sec).unwrap();
            assert!(d.config == expected, "{sec:?}: reset_section differs from Section::reset");
        }
    }

    #[test]
    fn table_ops_round_trip() {
        let mut d = draft();
        assert_eq!(d.table_len("bookmarks"), 0);
        let i = d.table_insert("bookmarks", 0, None).unwrap();
        d.element_set("bookmarks", i, "name", toml::Value::String("Code".into())).unwrap();
        d.element_set("bookmarks", i, "path", toml::Value::String("/src".into())).unwrap();
        d.element_set("bookmarks", i, "hotkey", toml::Value::String("Ctrl+1".into())).unwrap();
        let j = d.table_duplicate("bookmarks", i).unwrap();
        assert_eq!(j, 1);
        assert_eq!(d.config.bookmarks.len(), 2);
        assert_eq!(d.config.bookmarks[1].name, "Code 2");
        assert_eq!(d.config.bookmarks[1].path, d.config.bookmarks[0].path);
        d.table_move("bookmarks", 1, 0).unwrap();
        assert_eq!(d.config.bookmarks[0].name, "Code 2");
        assert_eq!(d.element_value("bookmarks", 1, "name").and_then(toml::Value::as_str), Some("Code"));
        d.table_remove("bookmarks", 0).unwrap();
        assert_eq!(d.config.bookmarks.len(), 1);
        assert!(d.table_remove("bookmarks", 5).is_err());
        assert!(d.table_move("bookmarks", 0, 3).is_err());

        // AI profiles: enum fields of an element cycle through element paths.
        let i = d.table_insert("ai.profiles", 9, None).unwrap();
        assert_eq!(i, 0);
        d.element_set("ai.profiles", i, "name", toml::Value::String("work".into())).unwrap();
        let path = d.element_path("ai.profiles", i, "kind");
        assert_eq!(path, "ai.profiles[0].kind");
        let before = d.config.ai.profiles[0].kind;
        d.cycle_enum(&path, true).unwrap();
        assert_ne!(d.config.ai.profiles[0].kind, before);
        let err = d.element_set("ai.profiles", i, "max_tokens", toml::Value::String("many".into())).unwrap_err();
        assert!(err.starts_with("ai.profiles[0].max_tokens: "), "{err}");
        d.table_duplicate("ai.profiles", 0).unwrap();
        assert_eq!(d.config.ai.profiles[1].name, "work 2");
        assert!(Draft::default_value("ai.profiles[1].max_tokens").is_some());
        // Round trip through config.toml text.
        let back = Draft::from_config(&config::parse_str(&toml::to_string(&d.table).unwrap()).unwrap().config);
        assert_eq!(back, Draft { error: None, ..d.clone() });
        // reset_table empties the list again.
        d.reset_table("ai.profiles").unwrap();
        assert!(d.config.ai.profiles.is_empty());
    }

    #[test]
    fn secret_names_follow_the_gui() {
        assert_eq!(secret_name("ai.profiles[0].key", "My Claude"), "my-claude");
        assert_eq!(secret_name("ai.mcp_clients[].token", "Desk"), soyuz_mcp::trust::token_credential("Desk"));
        assert_eq!(secret_name("ai.mcp_servers[2].token", "git"), "mcp-server-git");
        // An existing credential reference keeps its name (no store call is made here).
        let parsed = credentials::parse_ref("credential:kept").unwrap();
        assert_eq!(parsed, KeyRef::Credential("kept".into()));
    }
}
