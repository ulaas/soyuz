//! `docs/CONFIG.md` from the config schema: key names and types come from the Rust structs
//! in `crates/soyuz-core/src` (parsed as text, doc comments included), defaults from
//! `Config::default()` serialized to TOML, and the inline comments of
//! `docs/config.example.toml` fill in descriptions for keys without a doc comment.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use soyuz_core::config::{Config, to_pretty_string};

#[derive(Debug, Clone)]
struct Field {
    name: String,
    ty: String,
    doc: String,
}

#[derive(Debug, Default)]
struct Struct {
    doc: String,
    fields: Vec<Field>,
}

#[derive(Debug, Default)]
struct Enum {
    /// Serialized variant names (unit variants only); `None` when the enum is not a plain
    /// string enum (untagged, data-carrying, custom serde).
    values: Option<Vec<(String, String)>>,
}

#[derive(Default)]
struct Schema {
    structs: HashMap<String, Struct>,
    enums: HashMap<String, Enum>,
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// Plain-text doc comment: joined lines, intra-doc links reduced to their text.
fn clean_doc(lines: &[String]) -> String {
    let joined = lines.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
    let mut s = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    // [`x`](Self::y) / [x](super::y) → `x` / x
    let link = regex::Regex::new(r"\[([^\]]+)\]\((?:Self|self|super|crate)[^)]*\)").unwrap();
    s = link.replace_all(&s, "$1").into_owned();
    let bare = regex::Regex::new(r"\[(`[^`\]]+`)\]").unwrap();
    s = bare.replace_all(&s, "$1").into_owned();
    s
}

fn rename(variant: &str, rule: &str) -> String {
    let mut snake = String::new();
    for (i, c) in variant.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                snake.push('_');
            }
            snake.extend(c.to_lowercase());
        } else {
            snake.push(c);
        }
    }
    match rule {
        "snake_case" => snake,
        "kebab-case" => snake.replace('_', "-"),
        "lowercase" => variant.to_lowercase(),
        "SCREAMING_SNAKE_CASE" => snake.to_uppercase(),
        _ => variant.to_string(),
    }
}

fn attr_value(line: &str, key: &str) -> Option<String> {
    let i = line.find(&format!("{key} = \""))? + key.len() + 4;
    let rest = &line[i..];
    Some(rest[..rest.find('"')?].to_string())
}

fn parse_schema(src: &Path) -> Schema {
    let mut files = Vec::new();
    rust_files(src, &mut files);
    let mut schema = Schema::default();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let mut docs: Vec<String> = Vec::new();
        let mut attrs: Vec<String> = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let t = lines[i].trim();
            if let Some(d) = t.strip_prefix("///") {
                docs.push(d.to_string());
            } else if t.starts_with("#[") {
                attrs.push(t.to_string());
            } else if let Some(rest) = t.strip_prefix("pub struct ").filter(|r| r.ends_with('{')) {
                let name = rest.trim_end_matches('{').trim().split('<').next().unwrap().to_string();
                let mut st = Struct { doc: clean_doc(&docs), fields: Vec::new() };
                docs.clear();
                attrs.clear();
                i += 1;
                let mut fdocs: Vec<String> = Vec::new();
                let mut fattrs: Vec<String> = Vec::new();
                while i < lines.len() && lines[i].trim() != "}" {
                    let l = lines[i].trim();
                    if let Some(d) = l.strip_prefix("///") {
                        fdocs.push(d.to_string());
                    } else if l.starts_with("#[") {
                        fattrs.push(l.to_string());
                    } else if let Some(rest) = l.strip_prefix("pub ") {
                        if let Some((n, ty)) = rest.split_once(": ") {
                            let mut name = n.trim().to_string();
                            let skip =
                                fattrs.iter().any(|a| a.contains("serde(skip)") || a.contains("skip_deserializing"));
                            if let Some(r) = fattrs.iter().find_map(|a| attr_value(a, "rename")) {
                                name = r;
                            }
                            if !skip {
                                st.fields.push(Field {
                                    name,
                                    ty: ty.trim_end_matches(',').trim().to_string(),
                                    doc: clean_doc(&fdocs),
                                });
                            }
                        }
                        fdocs.clear();
                        fattrs.clear();
                    } else if !l.starts_with("//") {
                        fdocs.clear();
                        fattrs.clear();
                    }
                    i += 1;
                }
                schema.structs.entry(name).or_insert(st);
            } else if let Some(rest) = t.strip_prefix("pub enum ").filter(|r| r.ends_with('{')) {
                let name = rest.trim_end_matches('{').trim().to_string();
                let derives_de = attrs.iter().any(|a| a.contains("Deserialize"));
                let untagged = attrs.iter().any(|a| a.contains("untagged"));
                let rule = attrs.iter().find_map(|a| attr_value(a, "rename_all")).unwrap_or_default();
                docs.clear();
                attrs.clear();
                let mut values = Vec::new();
                let mut plain = derives_de && !untagged;
                let mut vdocs: Vec<String> = Vec::new();
                let mut vrename: Option<String> = None;
                i += 1;
                let mut depth = 0i32;
                while i < lines.len() {
                    let l = lines[i].trim();
                    if depth == 0 && l == "}" {
                        break;
                    }
                    if depth > 0 {
                        depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
                    } else if let Some(d) = l.strip_prefix("///") {
                        vdocs.push(d.to_string());
                    } else if l.starts_with("#[") {
                        if let Some(r) = attr_value(l, "rename") {
                            vrename = Some(r);
                        }
                    } else if !l.is_empty() && !l.starts_with("//") {
                        let ident: String = l.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        let after = l[ident.len()..].trim_start();
                        if after.starts_with('(') || after.starts_with('{') {
                            plain = false;
                        }
                        if after.starts_with('{') {
                            depth = 1 - l.matches('}').count() as i32;
                        }
                        if !ident.is_empty() {
                            let v = vrename.take().unwrap_or_else(|| rename(&ident, &rule));
                            values.push((v, clean_doc(&vdocs)));
                        }
                        vdocs.clear();
                    }
                    i += 1;
                }
                schema.enums.entry(name).or_insert(Enum { values: plain.then_some(values) });
            } else if !t.is_empty() && !t.starts_with("//") {
                docs.clear();
                attrs.clear();
            }
            i += 1;
        }
    }
    schema
}

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

fn last_segment(ty: &str) -> &str {
    let base = ty.split('<').next().unwrap();
    base.rsplit("::").next().unwrap()
}

fn generic_args(ty: &str) -> Vec<String> {
    let Some(start) = ty.find('<') else { return Vec::new() };
    let inner = &ty[start + 1..ty.len() - 1];
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in inner.chars() {
        match c {
            '<' | '(' => {
                depth += 1;
                cur.push(c);
            }
            '>' | ')' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

enum Kind<'a> {
    Table(&'a str),
    ArrayOfTables(&'a str),
    MapOfTables(&'a str),
    Value,
}

fn kind_of<'a>(schema: &Schema, ty: &'a str) -> Kind<'a> {
    let name = last_segment(ty);
    match name {
        "Vec" => {
            let args = generic_args(ty);
            if let Some(inner) = args.first()
                && schema.structs.contains_key(last_segment(inner))
            {
                let start = ty.find(inner.as_str()).unwrap();
                return Kind::ArrayOfTables(&ty[start..start + inner.len()]);
            }
            Kind::Value
        }
        "BTreeMap" | "HashMap" => {
            let args = generic_args(ty);
            if let Some(v) = args.get(1)
                && schema.structs.contains_key(last_segment(v))
            {
                let start = ty.rfind(v.as_str()).unwrap();
                return Kind::MapOfTables(&ty[start..start + v.len()]);
            }
            Kind::Value
        }
        n if schema.structs.contains_key(n) => Kind::Table(ty),
        _ => Kind::Value,
    }
}

fn type_label(schema: &Schema, ty: &str) -> String {
    let name = last_segment(ty);
    let args = generic_args(ty);
    match name {
        "bool" => "boolean".into(),
        "u8" | "u16" | "u32" | "u64" | "usize" | "i32" | "i64" => "integer".into(),
        "f32" | "f64" => "float".into(),
        "String" | "PathBuf" | "str" => "string".into(),
        "Option" => format!("{} (optional)", type_label(schema, &args[0])),
        "Vec" => format!("array of {}", plural(&type_label(schema, &args[0]))),
        "BTreeMap" | "HashMap" => format!("table: name → {}", type_label(schema, &args[1])),
        "Table" => "table".into(),
        "KeyChord" => "string (key chord)".into(),
        other => match schema.enums.get(other).and_then(|e| e.values.as_ref()) {
            Some(vals) => vals.iter().map(|(v, _)| format!("`\"{v}\"`")).collect::<Vec<_>>().join(" | "),
            None => "string".into(),
        },
    }
}

fn plural(s: &str) -> String {
    match s {
        "string" => "strings".into(),
        "integer" => "integers".into(),
        "float" => "floats".into(),
        "boolean" => "booleans".into(),
        other => other.to_string(),
    }
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

fn describe(ctx: &Ctx<'_>, path: &str, f: &Field) -> String {
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
    let schema = parse_schema(&root.join("crates/soyuz-core/src"));
    let example = std::fs::read_to_string(root.join("docs/config.example.toml")).unwrap_or_default();
    let defaults: toml::Table = toml::from_str(&to_pretty_string(&Config::default()).unwrap()).unwrap();
    let mut ctx = Ctx { schema: &schema, comments: example_comments(&example), sections: Vec::new() };
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
