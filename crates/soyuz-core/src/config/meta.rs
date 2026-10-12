//! Runtime metadata of the config schema: key names, Rust types, doc comments and enum
//! values, parsed (as text) from the embedded sources of the types `Config` is built from.
//!
//! One parser serves `docs/CONFIG.md` (`crates/soyuz-app/tests/docs_gen/config.rs`) and the
//! terminal UI's settings editor (`soyuz-appcore::models::settings_schema`). The sources are
//! embedded with `include_str!` and parsed once, lazily ([`schema`]). A unit test checks that
//! every file under `src/` declaring a `Deserialize` type is in [`SOURCES`].

use std::collections::HashMap;
use std::sync::OnceLock;

/// One field of a config struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldMeta {
    /// TOML key (after `#[serde(rename)]`).
    pub name: String,
    /// Rust type as written (`Vec<Bookmark>`, `super::ai::Ai`, …).
    pub ty: String,
    /// Doc comment, joined into one line (intra-doc links reduced to their text).
    pub doc: String,
}

/// A config struct (a TOML table).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructMeta {
    pub doc: String,
    /// Serialized fields in declaration order (`#[serde(skip)]` fields left out).
    pub fields: Vec<FieldMeta>,
}

/// A config enum.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnumMeta {
    /// Serialized variant names with their docs (unit variants only); `None` when the enum
    /// is not a plain string enum (untagged, data-carrying, custom serde).
    pub values: Option<Vec<(String, String)>>,
}

/// Every struct and enum declared in the embedded sources, by type name (the first
/// declaration of a name wins).
#[derive(Debug, Default)]
pub struct Schema {
    pub structs: HashMap<String, StructMeta>,
    pub enums: HashMap<String, EnumMeta>,
}

/// The embedded sources, `(path under src/, text)`, in the order a sorted recursive walk of
/// `src/` visits them (declaration order decides which of two equal names wins).
pub const SOURCES: &[(&str, &str)] = &[
    ("columns.rs", include_str!("../columns.rs")),
    ("config/actions.rs", include_str!("actions.rs")),
    ("config/ai.rs", include_str!("ai.rs")),
    ("config/ai_agent.rs", include_str!("ai_agent.rs")),
    ("config/automation.rs", include_str!("automation.rs")),
    ("config/defaults.rs", include_str!("defaults.rs")),
    ("config/fonts.rs", include_str!("fonts.rs")),
    ("config/keys.rs", include_str!("keys.rs")),
    ("config/mcp.rs", include_str!("mcp.rs")),
    ("config/schema.rs", include_str!("schema.rs")),
    ("entry.rs", include_str!("../entry.rs")),
    ("error.rs", include_str!("../error.rs")),
    ("filter.rs", include_str!("../filter.rs")),
    ("location.rs", include_str!("../location.rs")),
    ("policy.rs", include_str!("../policy.rs")),
    ("progress.rs", include_str!("../progress.rs")),
    ("size.rs", include_str!("../size.rs")),
    ("sort.rs", include_str!("../sort.rs")),
    ("time.rs", include_str!("../time.rs")),
];

/// The schema of the embedded sources, parsed on first use.
pub fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| parse(SOURCES.iter().map(|(_, text)| *text)))
}

/// Plain-text doc comment: joined lines, intra-doc links reduced to their text.
pub fn clean_doc(lines: &[String]) -> String {
    static LINK: OnceLock<regex::Regex> = OnceLock::new();
    static BARE: OnceLock<regex::Regex> = OnceLock::new();
    let joined = lines.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
    let mut s = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    // [`x`](Self::y) / [x](super::y) → `x` / x
    let link = LINK.get_or_init(|| regex::Regex::new(r"\[([^\]]+)\]\((?:Self|self|super|crate)[^)]*\)").unwrap());
    s = link.replace_all(&s, "$1").into_owned();
    let bare = BARE.get_or_init(|| regex::Regex::new(r"\[(`[^`\]]+`)\]").unwrap());
    s = bare.replace_all(&s, "$1").into_owned();
    s
}

/// A variant name under a serde `rename_all` rule.
pub fn rename(variant: &str, rule: &str) -> String {
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

/// The quoted value of `key = "…"` inside an attribute line.
pub fn attr_value(line: &str, key: &str) -> Option<String> {
    let i = line.find(&format!("{key} = \""))? + key.len() + 4;
    let rest = &line[i..];
    Some(rest[..rest.find('"')?].to_string())
}

/// Parse struct and enum declarations out of Rust source texts.
pub fn parse<'a>(sources: impl IntoIterator<Item = &'a str>) -> Schema {
    let mut schema = Schema::default();
    for text in sources {
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
                let mut st = StructMeta { doc: clean_doc(&docs), fields: Vec::new() };
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
                                st.fields.push(FieldMeta {
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
                schema.enums.entry(name).or_insert(EnumMeta { values: plain.then_some(values) });
            } else if !t.is_empty() && !t.starts_with("//") {
                docs.clear();
                attrs.clear();
            }
            i += 1;
        }
    }
    schema
}

/// Last path segment of a type, without generics (`super::ai::Ai` → `Ai`, `Vec<X>` → `Vec`).
pub fn last_segment(ty: &str) -> &str {
    let base = ty.split('<').next().unwrap();
    base.rsplit("::").next().unwrap()
}

/// Top-level generic arguments (`BTreeMap<String, Vec<u8>>` → `["String", "Vec<u8>"]`).
pub fn generic_args(ty: &str) -> Vec<String> {
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

/// How a field's type maps to TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind<'a> {
    /// A nested table of the struct type.
    Table(&'a str),
    /// `Vec<Struct>`: `[[path]]`.
    ArrayOfTables(&'a str),
    /// `BTreeMap<String, Struct>` / `HashMap<…>`: `[path.<name>]`.
    MapOfTables(&'a str),
    /// Anything else (scalars, enums, lists of values, maps of values).
    Value,
}

pub fn kind_of<'a>(schema: &Schema, ty: &'a str) -> Kind<'a> {
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

/// Human type label for `docs/CONFIG.md` (`integer`, `array of strings`, `` `"a"` | `"b"` ``).
pub fn type_label(schema: &Schema, ty: &str) -> String {
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

/// Plural of a scalar type label.
pub fn plural(s: &str) -> String {
    match s {
        "string" => "strings".into(),
        "integer" => "integers".into(),
        "float" => "floats".into(),
        "boolean" => "booleans".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

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

    /// True when a source declares a type with a derived (or hand-written) deserializer.
    fn declares_deserialize(text: &str) -> bool {
        let derive = ["#[derive(", "Deserialize"];
        let manual = ["impl<'de> ", "Deserialize<'de> for "];
        let mut in_derive = false;
        for l in text.lines().map(str::trim) {
            if l.starts_with(derive[0]) {
                in_derive = true;
            }
            if in_derive && l.contains(derive[1]) {
                return true;
            }
            if in_derive && l.contains(')') {
                in_derive = false;
            }
            if l.starts_with(manual[0]) && l.contains(manual[1]) {
                return true;
            }
        }
        false
    }

    /// Every file under `src/` that declares a deserializable type is embedded, in the order
    /// of a sorted walk (so a new config file cannot silently drop out of the schema).
    #[test]
    fn every_deserialize_file_is_embedded_in_walk_order() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let wanted: Vec<String> = files
            .iter()
            .filter(|p| declares_deserialize(&std::fs::read_to_string(p).unwrap()))
            .map(|p| p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        let embedded: Vec<&str> = SOURCES.iter().map(|(p, _)| *p).collect();
        assert_eq!(embedded, wanted, "update config::meta::SOURCES");
        for (p, text) in SOURCES {
            let disk = std::fs::read_to_string(src.join(p)).unwrap();
            assert_eq!(disk.lines().collect::<Vec<_>>(), text.lines().collect::<Vec<_>>(), "{p}");
        }
    }

    #[test]
    fn schema_has_the_config_tree() {
        let s = schema();
        let config = &s.structs["Config"];
        assert!(config.fields.iter().any(|f| f.name == "tui" && f.ty == "TuiSection"));
        assert!(matches!(kind_of(s, "Vec<Bookmark>"), Kind::ArrayOfTables("Bookmark")));
        assert!(matches!(kind_of(s, "TuiSection"), Kind::Table("TuiSection")));
        let mode = s.enums["TuiShellMode"].values.as_ref().unwrap();
        assert_eq!(mode.iter().map(|(v, _)| v.as_str()).collect::<Vec<_>>(), ["pane", "suspend"]);
        assert_eq!(type_label(s, "Option<u32>"), "integer (optional)");
        assert_eq!(generic_args("BTreeMap<String, Vec<(u8, u8)>>"), ["String", "Vec<(u8, u8)>"]);
        assert_eq!(rename("TrueColor", "kebab-case"), "true-color");
    }
}
