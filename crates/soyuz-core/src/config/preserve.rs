//! Format-preserving `config.toml` writes: the new configuration is merged into the
//! user's existing document with `toml_edit`, so comments, key order, blank lines and
//! untouched values survive a save from the settings window.

use toml_edit::{DocumentMut, Item, Table, Value};

use super::{Config, ConfigError, to_pretty_string};

/// Text for `config` that keeps `existing`'s comments and layout. Only values that
/// changed are rewritten (their trailing comments are kept); new keys / tables are
/// appended unless they equal the built-in default (a key the user never wrote keeps
/// meaning "default"); keys the configuration no longer has (removed map entries) are
/// dropped.
/// Falls back to [`to_pretty_string`] when `existing` is empty or does not parse.
pub fn to_string_preserving(existing: &str, config: &Config) -> Result<String, ConfigError> {
    let fresh = to_pretty_string(config)?;
    if existing.trim().is_empty() {
        return Ok(fresh);
    }
    let Ok(mut doc) = existing.parse::<DocumentMut>() else { return Ok(fresh) };
    let new: DocumentMut =
        fresh.parse().map_err(|e: toml_edit::TomlError| ConfigError::Parse { line: 0, col: 0, msg: e.to_string() })?;
    let defaults: DocumentMut = to_pretty_string(&Config::default())?
        .parse()
        .map_err(|e: toml_edit::TomlError| ConfigError::Parse { line: 0, col: 0, msg: e.to_string() })?;
    merge_table(doc.as_table_mut(), new.as_table(), Some(defaults.as_table()));
    Ok(doc.to_string())
}

fn merge_table(old: &mut Table, new: &Table, defaults: Option<&Table>) {
    let stale: Vec<String> = old.iter().map(|(k, _)| k.to_string()).filter(|k| !new.contains_key(k)).collect();
    for k in stale {
        old.remove(&k);
    }
    for (key, new_item) in new.iter() {
        let default = defaults.and_then(|d| d.get(key));
        let Some(old_item) = old.get_mut(key) else {
            match new_item {
                Item::Table(n) => {
                    let mut t = Table::new();
                    merge_table(&mut t, n, default.and_then(Item::as_table));
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
            (Item::Table(o), Item::Table(n)) => merge_table(o, n, default.and_then(Item::as_table)),
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
    fn falls_back_on_broken_or_empty_files() {
        let c = Config::default();
        assert_eq!(to_string_preserving("", &c).unwrap(), to_pretty_string(&c).unwrap());
        assert_eq!(to_string_preserving("[[[ nope", &c).unwrap(), to_pretty_string(&c).unwrap());
    }
}
