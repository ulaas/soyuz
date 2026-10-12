//! `docs/CONFIG.md` from the config schema: key names and types come from the Rust structs
//! in `crates/soyuz-core/src` (parsed as text by `soyuz_core::config::meta`, doc comments
//! included), defaults from `Config::default()` serialized to TOML, and the inline comments of
//! `docs/config.example.toml` fill in descriptions for keys without a doc comment.

use std::collections::HashMap;
use std::path::Path;

use soyuz_core::config::meta::{FieldMeta, Kind, Schema, kind_of, last_segment, schema, type_label};
use soyuz_core::config::{Config, to_pretty_string};

/// `docs/config.example.toml` inline comments, by dotted key (`general.start_location`)
/// and by table (`fonts.ui`).
fn example_comments(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut table = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        let (code, comment) = match t.find(" #") {
            Some(i) if !t[..i].contains('"') || t[..i].matches('"').count() % 2 == 0 => {
                (t[..i].trim(), t[i + 2..].trim().to_string())
            }
            _ => (t, String::new()),
        };
        if code.starts_with('[') {
            table = code.trim_matches(|c| c == '[' || c == ']').trim().to_string();
            if !comment.is_empty() {
                out.insert(table.clone(), comment);
            }
            continue;
        }
        if let Some((k, _)) = code.split_once('=') {
            let k = k.trim().trim_matches('"');
            if !comment.is_empty() && !k.is_empty() {
                let key = if table.is_empty() { k.to_string() } else { format!("{table}.{k}") };
                out.entry(key).or_insert(comment);
            }
        }
    }
    out
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn render_default(v: Option<&toml::Value>) -> String {
    let Some(v) = v else { return "—".into() };
    let s = match v {
        toml::Value::String(s) if s.is_empty() => "`\"\"`".to_string(),
        toml::Value::Array(a) if a.is_empty() => "`[]`".to_string(),
        toml::Value::Table(t) if t.is_empty() => "`{}`".to_string(),
        toml::Value::Table(t) => format!("{} entries", t.len()),
        other => {
            let s = other.to_string();
            if s.chars().count() > 90 {
                let cut: String = s.chars().take(80).collect();
                format!("`{cut}…`")
            } else {
                format!("`{s}`")
            }
        }
    };
    md_cell(&s)
}

struct Section {
    header: String,
    anchor_note: String,
    doc: String,
    rows: Vec<String>,
}

struct Ctx<'a> {
    schema: &'a Schema,
    comments: HashMap<String, String>,
    sections: Vec<Section>,
}

/// True when `text` only lists the enum's values (`same_tab | new_tab`): the type column
/// already says that.
fn only_values(text: &str, vals: &[(String, String)]) -> bool {
    !text.is_empty()
        && text.split('|').all(|part| {
            let p = part.trim().trim_matches(|c| c == '"' || c == '`');
            vals.iter().any(|(v, _)| v == p)
        })
}

fn describe(ctx: &Ctx<'_>, path: &str, f: &FieldMeta) -> String {
    let mut d = f.doc.clone();
    let example = ctx.comments.get(path).cloned().unwrap_or_default();
    let name = last_segment(&f.ty);
    let vals = ctx.schema.enums.get(name).and_then(|e| e.values.as_ref());
    if d.is_empty() && !vals.is_some_and(|v| only_values(&example, v)) {
        d = example;
    }
    // Enum value docs, when the values carry their own doc comments.
    if let Some(vals) = vals {
        let documented: Vec<String> = vals
            .iter()
            .filter(|(_, doc)| !doc.is_empty())
            .map(|(v, doc)| {
                let doc = if doc.ends_with("..") { doc.as_str() } else { doc.strip_suffix('.').unwrap_or(doc) };
                format!("`{v}`: {doc}")
            })
            .collect();
        if !documented.is_empty() && vals.len() <= 8 {
            let tail = documented.join("; ");
            d = if d.is_empty() { format!("{tail}.") } else { format!("{d} ({tail})") };
        }
    }
    md_cell(&d)
}

/// Document struct `ty` as the table at `path` with defaults from `value`.
fn walk(ctx: &mut Ctx<'_>, path: &str, header: String, ty: &str, value: Option<&toml::Table>, note: String) {
    let name = last_segment(ty).to_string();
    let Some(st) = ctx.schema.structs.get(&name) else { return };
    let fields = st.fields.clone();
    let mut section = Section {
        header,
        anchor_note: note,
        doc: if path.is_empty() { String::new() } else { st.doc.clone() },
        rows: Vec::new(),
    };
    if let Some(c) = ctx.comments.get(path)
        && section.doc.is_empty()
    {
        section.doc = c.clone();
    }
    // A doc comment that only names the table (`[fonts]`.) adds nothing.
    if section.doc.trim_end_matches('.').trim_matches('`') == section.header.trim_matches('`') {
        section.doc.clear();
    }
    let mut nested: Vec<(String, String, String, Kind<'_>)> = Vec::new();
    for f in &fields {
        let fpath = if path.is_empty() { f.name.clone() } else { format!("{path}.{}", f.name) };
        match kind_of(ctx.schema, &f.ty) {
            Kind::Value => {
                let default = render_default(value.and_then(|t| t.get(&f.name)));
                section.rows.push(format!(
                    "| `{}` | {} | {} | {} |",
                    f.name,
                    md_cell(&type_label(ctx.schema, &f.ty)),
                    default,
                    describe(ctx, &fpath, f)
                ));
            }
            k => nested.push((fpath, f.name.clone(), f.ty.clone(), k)),
        }
    }
    ctx.sections.push(section);
    for (fpath, fname, _ty, kind) in nested {
        let sub = value.and_then(|t| t.get(&fname));
        match kind {
            Kind::Table(t) => {
                let t = t.to_string();
                let tbl = sub.and_then(toml::Value::as_table);
                walk(ctx, &fpath, format!("[{fpath}]"), &t, tbl, String::new());
            }
            Kind::ArrayOfTables(inner) => {
                let inner = inner.to_string();
                let arr = sub.and_then(toml::Value::as_array);
                let n = arr.map_or(0, Vec::len);
                let first = arr.and_then(|a| a.first()).and_then(toml::Value::as_table);
                let note = if n == 0 {
                    "A list of tables; empty by default. Defaults below are for a new entry's omitted keys.".into()
                } else {
                    format!("A list of tables; {n} entries by default. The default column shows the first entry.")
                };
                walk(ctx, &fpath, format!("[[{fpath}]]"), &inner, first, note);
            }
            Kind::MapOfTables(inner) => {
                let inner = inner.to_string();
                let map = sub.and_then(toml::Value::as_table);
                let first = map.and_then(|m| m.values().next()).and_then(toml::Value::as_table);
                walk(
                    ctx,
                    &format!("{fpath}.<name>"),
                    format!("[{fpath}.<name>]"),
                    &inner,
                    first,
                    "One table per name.".into(),
                );
            }
            Kind::Value => unreachable!(),
        }
    }
}

fn anchor(header: &str) -> String {
    header
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | '0'..='9' | '-' | '_' => Some(c),
            'A'..='Z' => Some(c.to_ascii_lowercase()),
            ' ' => Some('-'),
            _ => None,
        })
        .collect()
}

pub fn markdown(root: &Path) -> String {
    let schema = schema();
    let example = std::fs::read_to_string(root.join("docs/config.example.toml")).unwrap_or_default();
    let defaults: toml::Table = toml::from_str(&to_pretty_string(&Config::default()).unwrap()).unwrap();
    let mut ctx = Ctx { schema, comments: example_comments(&example), sections: Vec::new() };
    walk(&mut ctx, "", "Top level".into(), "Config", Some(&defaults), String::new());

    let mut s = String::new();
    s.push_str("# Configuration reference\n\n");
    s.push_str(
        "<!-- Generated from crates/soyuz-core/src/config (schema, doc comments, Config::default()) and the\n     \
         comments in docs/config.example.toml by crates/soyuz-app/tests/docs_sync.rs. Do not edit by hand:\n     \
         run `SOYUZ_BLESS_DOCS=1 cargo test -p soyuz-app --test docs_sync`. -->\n\n",
    );
    s.push_str(
        "Everything Soyuz does is configured in one TOML file, `config.toml`. Every key is optional:\n\
         a missing key takes the default shown here, unknown keys are reported as warnings, and the file\n\
         is hot-reloaded (edits apply within a second; a parse error keeps the previous settings and\n\
         shows the line and column). The Settings window (<kbd>Ctrl</kbd>+<kbd>,</kbd>) edits the same\n\
         file and keeps your comments. An annotated sample is [config.example.toml](config.example.toml).\n\n\
         | OS | Location |\n|---|---|\n\
         | Windows | `%APPDATA%\\soyuz\\config\\config.toml` |\n\
         | Linux | `~/.config/soyuz/config.toml` |\n\
         | macOS | `~/Library/Application Support/soyuz/config.toml` |\n\
         | Portable | `portable-data\\config.toml` next to the executable when an empty file named `portable` sits beside it |\n\n\
         Secrets never go in this file: API keys, remote passwords and MCP tokens live in the OS\n\
         credential store, and config values only reference them (`credential:<name>`).\n\n\
         Types: *string*, *integer*, *float*, *boolean*, arrays (`[\"a\", \"b\"]`) and tables. A type\n\
         written as a list of quoted values is an enumeration: use one of them.\n\n",
    );
    s.push_str("## Sections\n\n");
    for sec in &ctx.sections {
        s.push_str(&format!("- [`{}`](#{})\n", sec.header, anchor(&sec.header)));
    }
    s.push('\n');
    for sec in &ctx.sections {
        s.push_str(&format!("## `{}`\n\n", sec.header));
        if !sec.doc.is_empty() {
            s.push_str(&sec.doc);
            s.push_str("\n\n");
        }
        if !sec.anchor_note.is_empty() {
            s.push_str(&sec.anchor_note);
            s.push_str("\n\n");
        }
        if sec.rows.is_empty() {
            s.push_str("Only sub-tables (below).\n\n");
            continue;
        }
        s.push_str("| Key | Type | Default | Description |\n|---|---|---|---|\n");
        for r in &sec.rows {
            s.push_str(r);
            s.push('\n');
        }
        s.push('\n');
    }
    s
}
