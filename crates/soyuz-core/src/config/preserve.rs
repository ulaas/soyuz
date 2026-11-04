//! Format-preserving `config.toml` writes: the new configuration is merged into the
//! user's existing document with `toml_edit`, so comments, key order, blank lines and
//! untouched values survive a save from the settings window.

use std::collections::HashSet;

use toml_edit::{DocumentMut, Item, Table, Value};

use super::{Config, ConfigError, parse_reporting_unknown, to_pretty_string};

/// Text for `config` that keeps `existing`'s comments and layout. Only values that
/// changed are rewritten (their trailing comments are kept); new keys / tables are
/// appended unless they equal the built-in default (a key the user never wrote keeps
/// meaning "default"); keys of the schema the configuration no longer has (removed map
/// entries) are dropped, and keys this version does not know stay for the user (or a
/// newer version) to deal with.
///
/// An empty `existing` gives [`to_pretty_string`]. Text that does not load as a
/// configuration (bad TOML, a bad value, a newer schema) is an error: writing over it
/// would replace the user's settings with defaults.
pub fn to_string_preserving(existing: &str, config: &Config) -> Result<String, ConfigError> {
    let fresh = to_pretty_string(config)?;
    if existing.trim().is_empty() {
        return Ok(fresh);
    }
    let (_, unknown) = parse_reporting_unknown(existing)?;
    let unknown: HashSet<String> = unknown.into_iter().collect();
    let mut doc: DocumentMut = existing.parse().map_err(|e: toml_edit::TomlError| ConfigError::Parse {
        line: 0,
        col: 0,
        msg: e.to_string(),
    })?;
    let new: DocumentMut =
        fresh.parse().map_err(|e: toml_edit::TomlError| ConfigError::Parse { line: 0, col: 0, msg: e.to_string() })?;
    let defaults: DocumentMut = to_pretty_string(&Config::default())?
        .parse()
        .map_err(|e: toml_edit::TomlError| ConfigError::Parse { line: 0, col: 0, msg: e.to_string() })?;
    merge_table(doc.as_table_mut(), new.as_table(), Some(defaults.as_table()), "", &unknown);
    Ok(doc.to_string())
}

/// `ours` rebased onto `theirs`: what changed between `base` and `ours` (settings not yet
/// saved) applied on top of `theirs` (the file as it was just reloaded). Keys are compared
/// one by one; an array counts as one value.
pub fn rebase(base: &Config, ours: &Config, theirs: &Config) -> Result<Config, ConfigError> {
    let parse_err = |e: toml::de::Error| ConfigError::Parse { line: 0, col: 0, msg: e.to_string() };
    let table =
        |c: &Config| -> Result<toml::Table, ConfigError> { toml::from_str(&to_pretty_string(c)?).map_err(parse_err) };
    let mut merged = table(theirs)?;
    merge_changes(&table(base)?, &table(ours)?, &mut merged);
    toml::Value::Table(merged).try_into().map_err(parse_err)
}

fn merge_changes(base: &toml::Table, ours: &toml::Table, theirs: &mut toml::Table) {
    for key in base.keys().chain(ours.keys().filter(|k| !base.contains_key(*k))) {
        match (base.get(key), ours.get(key)) {
            (Some(b), Some(o)) if value_eq(b, o) => {}
            (Some(toml::Value::Table(b)), Some(toml::Value::Table(o))) => {
                if let Some(toml::Value::Table(t)) = theirs.get_mut(key) {
                    merge_changes(b, o, t);
                } else {
                    theirs.insert(key.clone(), toml::Value::Table(o.clone()));
                }
            }
            (_, Some(o)) => {
                theirs.insert(key.clone(), o.clone());
            }
            (_, None) => {
                theirs.remove(key);
            }
        }
    }
}

fn merge_table(old: &mut Table, new: &Table, defaults: Option<&Table>, path: &str, unknown: &HashSet<String>) {
    let at = |k: &str| if path.is_empty() { k.to_string() } else { format!("{path}.{k}") };
    let stale: Vec<String> =
        old.iter().map(|(k, _)| k.to_string()).filter(|k| !new.contains_key(k) && !unknown.contains(&at(k))).collect();
    for k in stale {
        old.remove(&k);
    }
    for (key, new_item) in new.iter() {
        let default = defaults.and_then(|d| d.get(key));
        let Some(old_item) = old.get_mut(key) else {
            match new_item {
                Item::Table(n) => {
                    let mut t = Table::new();
                    merge_table(&mut t, n, default.and_then(Item::as_table), &at(key), unknown);
                    if !t.is_empty() {
                        let only_tables = t.iter().all(|(_, i)| i.is_table() || i.is_array_of_tables());
                        t.set_implicit(only_tables);
                        old.insert(key, Item::Table(t));
                    }
                }
                _ if default.is_some_and(|d| same(d, new_item)) => {}
                _ => {
                    old.insert(key, detached(new_item));
                }
            }
            continue;
        };
        if same(old_item, new_item) {
            continue;
        }
        match (old_item, new_item) {
            (Item::Table(o), Item::Table(n)) => merge_table(o, n, default.and_then(Item::as_table), &at(key), unknown),
            (Item::Value(o), Item::Table(n)) if o.is_inline_table() => {
                let decor = o.decor().clone();
                let mut v = Value::InlineTable(n.clone().into_inline_table());
                *v.decor_mut() = decor;
                *o = v;
            }
            (Item::Value(o), Item::Value(n)) => {
                let decor = o.decor().clone();
                let mut v = n.clone();
                *v.decor_mut() = decor;
                *o = v;
            }
            (o, n) => *o = detached(n),
        }
    }
}

/// A copy of `item` without document positions (so it sorts after the existing tables).
fn detached(item: &Item) -> Item {
    let mut item = item.clone();
    clear_positions(&mut item);
    item
}

fn clear_positions(item: &mut Item) {
    match item {
        Item::Table(t) => {
            t.set_position(None);
            for (_, child) in t.iter_mut() {
                clear_positions(child);
            }
        }
        Item::ArrayOfTables(a) => {
            for t in a.iter_mut() {
                t.set_position(None);
                for (_, child) in t.iter_mut() {
                    clear_positions(child);
                }
            }
        }
        _ => {}
    }
}

/// Semantic equality of two items (formatting, comments and int/float spelling ignored).
fn same(a: &Item, b: &Item) -> bool {
    match (semantic(a), semantic(b)) {
        (Some(x), Some(y)) => value_eq(&x, &y),
        _ => false,
    }
}

fn semantic(item: &Item) -> Option<toml::Value> {
    let mut d = DocumentMut::new();
    d.insert("v", detached(item));
    let mut t: toml::Table = toml::from_str(&d.to_string()).ok()?;
    t.remove("v")
}

fn value_eq(a: &toml::Value, b: &toml::Value) -> bool {
    use toml::Value as V;
    match (a, b) {
        (V::Integer(x), V::Float(y)) | (V::Float(y), V::Integer(x)) => (*x as f64 - y).abs() < 1e-9,
        (V::Float(x), V::Float(y)) => (x - y).abs() < 1e-9 || (x.is_nan() && y.is_nan()),
        (V::Array(x), V::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| value_eq(p, q)),
        (V::Table(x), V::Table(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| value_eq(v, w)))
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Bookmark, parse_str};

    const USER: &str = "\
# My Soyuz config
schema = 1

[general]
# I like hidden files
show_hidden = true   # important!
natural_sort = true

[appearance]
theme = \"dark\" # night owl
sidebar_width = 240

[keys.bindings]
copy = [\"Ctrl+Insert\"]
paste = [\"Shift+Insert\"]
";

    #[test]
    fn keeps_comments_and_changes_values() {
        let mut c = parse_str(USER).unwrap().config;
        c.appearance.theme = "light".into();
        c.keys.bindings.remove("paste");
        c.bookmarks.push(Bookmark { name: "P".into(), path: "/p".into(), ..Default::default() });
        let out = to_string_preserving(USER, &c).unwrap();
        assert!(out.starts_with("# My Soyuz config"), "{out}");
        assert!(out.contains("# I like hidden files"), "{out}");
        assert!(out.contains("show_hidden = true   # important!"), "{out}");
        assert!(out.contains("theme = \"light\" # night owl"), "{out}");
        assert!(out.contains("sidebar_width = 240\n"), "int spelling of an unchanged float is kept: {out}");
        assert!(!out.contains("Shift+Insert"), "{out}");
        let back = parse_str(&out).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        assert_eq!(back.config, c);
    }

    #[test]
    fn unchanged_config_keeps_the_user_text() {
        let c = parse_str(USER).unwrap().config;
        let out = to_string_preserving(USER, &c).unwrap();
        assert!(out.starts_with(USER.trim_end()), "{out}");
        assert_eq!(parse_str(&out).unwrap().config, c);
    }

    #[test]
    fn portal_pins_round_trip_and_unpin() {
        use crate::config::PortalPin;
        let mut c = parse_str(USER).unwrap().config;
        c.sidebar.portal.push(PortalPin { path: "C:\\Work\\a".into(), name: String::new() });
        c.sidebar.portal.push(PortalPin { path: "/b".into(), name: "Bee".into() });
        let out = to_string_preserving(USER, &c).unwrap();
        assert!(out.contains("# I like hidden files"), "{out}");
        let back = parse_str(&out).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        assert_eq!(back.config, c);
        c.sidebar.portal.remove(0);
        let out2 = to_string_preserving(&out, &c).unwrap();
        assert_eq!(parse_str(&out2).unwrap().config, c, "{out2}");
        c.sidebar.portal.clear();
        let out3 = to_string_preserving(&out2, &c).unwrap();
        assert_eq!(parse_str(&out3).unwrap().config, c, "{out3}");
    }

    #[test]
    fn empty_file_gets_the_pretty_text() {
        let c = Config::default();
        assert_eq!(to_string_preserving("", &c).unwrap(), to_pretty_string(&c).unwrap());
        assert_eq!(to_string_preserving("  \n", &c).unwrap(), to_pretty_string(&c).unwrap());
    }

    #[test]
    fn refuses_text_that_does_not_load() {
        let c = Config::default();
        assert!(to_string_preserving("[[[ nope", &c).is_err(), "bad TOML");
        assert!(to_string_preserving("[appearance]\nsidebar_width = \"wide\"\n", &c).is_err(), "a bad value");
        assert!(to_string_preserving("schema = 99999\n", &c).is_err(), "a newer schema");
    }

    #[test]
    fn keeps_keys_this_version_does_not_know() {
        let text = "\
# top
future_key = 1 # keep me

[general]
show_hiddn = true # a typo to fix by hand
show_hidden = false

[future_table]
x = 1

[future_table.nested]
y = 2

[keys.bindings]
copy = [\"Ctrl+Insert\"]
paste = [\"Shift+Insert\"]
";
        let mut c = parse_str(text).unwrap().config;
        c.general.show_hidden = true;
        c.keys.bindings.remove("paste");
        let out = to_string_preserving(text, &c).unwrap();
        assert!(out.contains("future_key = 1 # keep me"), "{out}");
        assert!(out.contains("show_hiddn = true # a typo to fix by hand"), "{out}");
        assert!(out.contains("[future_table]") && out.contains("y = 2"), "{out}");
        assert!(out.contains("show_hidden = true"), "{out}");
        assert!(!out.contains("Shift+Insert"), "a map entry the user deleted goes: {out}");
        assert_eq!(parse_str(&out).unwrap().config, c);
    }

    #[test]
    fn removed_settings_still_leave_the_file() {
        let text = "[appearance]\nstartup_placeholder = true\ntheme = \"dark\"\n";
        let c = parse_str(text).unwrap().config;
        let out = to_string_preserving(text, &c).unwrap();
        assert!(!out.contains("startup_placeholder"), "{out}");
    }

    #[test]
    fn rebase_keeps_pending_changes_on_top_of_a_reload() {
        let base = Config::default();
        let mut ours = base.clone();
        ours.general.show_hidden = !base.general.show_hidden;
        ours.keys.bindings.insert("copy".into(), vec!["Ctrl+Insert".into()]);
        let mut theirs = base.clone();
        theirs.appearance.theme = "external".into();
        theirs.general.natural_sort = !base.general.natural_sort;
        let merged = rebase(&base, &ours, &theirs).unwrap();
        assert_eq!(merged.appearance.theme, "external", "the external edit stays");
        assert_eq!(merged.general.natural_sort, theirs.general.natural_sort, "same table, another key");
        assert_eq!(merged.general.show_hidden, ours.general.show_hidden, "the pending change stays");
        assert_eq!(merged.keys.bindings, ours.keys.bindings);
        // Nothing pending: the reload wins whole.
        assert_eq!(rebase(&base, &base, &theirs).unwrap(), theirs);
        // A pending removal of a map entry.
        let mut with_entry = base.clone();
        with_entry.keys.bindings.insert("paste".into(), vec!["F9".into()]);
        let merged = rebase(&with_entry, &base, &with_entry).unwrap();
        assert!(merged.keys.bindings.is_empty());
    }
}
