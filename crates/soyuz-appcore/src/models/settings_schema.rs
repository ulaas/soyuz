//! The editable view of the config schema for a generated settings editor (the terminal
//! UI's): every key as a [`FieldRow`] with its kind, range, doc comment and restart flag,
//! grouped into TOML tables ([`TableRows`]), plus the element forms of arrays / maps of
//! tables (`[[bookmarks]]`, `[[ai.profiles]]`, …).
//!
//! Names, types, docs and enum values come from `soyuz_core::config::meta` (the same parser
//! that writes `docs/CONFIG.md`); what prose cannot say (ranges, "this string is a path",
//! "needs a restart") comes from [`HINTS`], which tests check against the schema so it
//! cannot go stale.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use soyuz_core::config::meta::{self, Kind, Schema};
use soyuz_core::config::{Config, TuiSection};

use crate::models::settings::Section;

/// How a key is edited.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Bool,
    Int {
        min: Option<i64>,
        max: Option<i64>,
    },
    Float {
        min: Option<f64>,
        max: Option<f64>,
    },
    /// Free string.
    Text,
    /// A path (or a program looked up on PATH): path completion.
    Path,
    /// `"#RRGGBB"` (or a documented keyword).
    Color,
    /// A key chord string (`"Ctrl+Shift+P"`), recorded rather than typed.
    Chord,
    /// One of `values` (serialized name, doc).
    Enum {
        values: Vec<(String, String)>,
    },
    /// A list of enum values (`Vec<Column>`, `Vec<CommandContext>`, …).
    EnumList {
        values: Vec<(String, String)>,
    },
    /// A list of strings / paths / numbers (`Vec<String>`, `[u16; 2]`).
    TextList,
    /// `name = value` rows (`BTreeMap<String, String | f32>`).
    Map,
    /// An array of tables (`keyed == false`, `[[path]]`) or a map of tables (`keyed`,
    /// `[path.<name>]`) whose elements have the fields of struct `element`
    /// ([`element_rows`]).
    Table {
        element: String,
        keyed: bool,
    },
    /// Not representable as a form (raw TOML tables, tagged enums with data, the
    /// keybinding map that the keyboard page edits): shown as a summary.
    Opaque,
}

/// One key.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldRow {
    /// Dotted path: `"tui.sidebar_width"`; in an element form `"bookmarks[].hotkey"`
    /// (fields of nested tables inside an element: `"x[].sub.key"`).
    pub path: String,
    /// The key name (`"sidebar_width"`; `"sub.key"` inside an element form).
    pub name: String,
    /// Doc comment (English, like `docs/CONFIG.md`).
    pub doc: String,
    pub kind: FieldKind,
    /// The change takes effect after a restart.
    pub restart: bool,
    /// The value is a reference into the OS credential store (`credential:<name>`), never
    /// the secret itself: offer "Set secret…" ([`crate::models::settings_draft::store_secret`]).
    pub secret_ref: bool,
}

/// The keys of one TOML table (`""` = the top level), or of one element form.
#[derive(Clone, Debug, PartialEq)]
pub struct TableRows {
    /// `""` | `"tui"` | `"search.index"`; element forms: `"bookmarks[]"`.
    pub table: String,
    pub doc: String,
    pub rows: Vec<FieldRow>,
}

/// Every table of `Config` (in `docs/CONFIG.md` order) and every element form.
#[derive(Clone, Debug, Default)]
pub struct SettingsSchema {
    pub tables: Vec<TableRows>,
    /// Element forms by struct name (`"Bookmark"`, `"AiProfile"`, …).
    pub elements: BTreeMap<String, TableRows>,
}

/// What the schema text cannot say.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hint {
    /// Allowed range (inclusive), as the GUI's number fields clamp it.
    Range(f64, f64),
    Path,
    Color,
    Chord,
    Restart,
}

/// Hints by row path (element fields: `"bookmarks[].hotkey"`).
pub const HINTS: &[(&str, Hint)] = &[
    // [tui]: the TuiSection consts (tested).
    ("tui.sidebar_width", Hint::Range(TuiSection::SIDEBAR_WIDTH_MIN as f64, TuiSection::SIDEBAR_WIDTH_MAX as f64)),
    ("tui.jobs_rows", Hint::Range(TuiSection::JOBS_ROWS_MIN as f64, TuiSection::JOBS_ROWS_MAX as f64)),
    ("tui.cell_pixels", Hint::Range(TuiSection::CELL_PIXELS_MIN as f64, TuiSection::CELL_PIXELS_MAX as f64)),
    ("tui.preview_ratio", Hint::Range(0.2, 0.6)),
    ("tui.terminal_rows", Hint::Range(TuiSection::TERMINAL_ROWS_MIN as f64, TuiSection::TERMINAL_ROWS_MAX as f64)),
    ("tui.ai_ratio", Hint::Range(0.25, 0.6)),
    ("tui.editor", Hint::Path),
    // The GUI's number fields.
    ("general.double_click_speed_ms", Hint::Range(150.0, 900.0)),
    ("appearance.font_size", Hint::Range(11.0, 17.0)),
    ("appearance.ui_scale", Hint::Range(0.5, 3.0)),
    ("appearance.thumb_disk_cache_mib", Hint::Range(0.0, 4096.0)),
    ("appearance.thumbnail_size", Hint::Range(32.0, 256.0)),
    ("appearance.sidebar_width", Hint::Range(160.0, 420.0)),
    ("fonts.text_gamma_dark", Hint::Range(0.0, 1.5)),
    ("fonts.text_gamma_light", Hint::Range(0.0, 1.5)),
    ("fonts.ui.size_offset", Hint::Range(-4.0, 8.0)),
    ("fonts.ui_semibold.size_offset", Hint::Range(-4.0, 8.0)),
    ("fonts.terminal.size_offset", Hint::Range(-4.0, 8.0)),
    ("fonts.code.size_offset", Hint::Range(-4.0, 8.0)),
    ("fonts.hex.size_offset", Hint::Range(-4.0, 8.0)),
    ("fonts.numbers.size_offset", Hint::Range(-4.0, 8.0)),
    ("sidebar.tree_drag_expand_ms", Hint::Range(0.0, 3000.0)),
    ("layout.terminal_height", Hint::Range(120.0, 800.0)),
    ("workspace.tear_off_distance", Hint::Range(16.0, 200.0)),
    ("workspace.history_graph_limit", Hint::Range(20.0, 2000.0)),
    ("workspace.timeline_days", Hint::Range(1.0, 60.0)),
    ("workspace.timeline_scan_folders", Hint::Range(0.0, 300.0)),
    ("workspace.timeline_max_items", Hint::Range(50.0, 10_000.0)),
    ("behaviour.spring_load_delay_ms", Hint::Range(200.0, 3000.0)),
    ("behaviour.watch_debounce_ms", Hint::Range(50.0, 2000.0)),
    ("behaviour.watch_poll_interval_ms", Hint::Range(500.0, 60_000.0)),
    ("jobs.max_parallel", Hint::Range(1.0, 16.0)),
    ("jobs.history_limit", Hint::Range(10.0, 5000.0)),
    ("jobs.copy_buffer_mib", Hint::Range(1.0, 128.0)),
    ("jobs.unbuffered_threshold_mib", Hint::Range(16.0, 4096.0)),
    ("archive.bomb_ratio", Hint::Range(0.0, 1000.0)),
    ("archive.bomb_floor_mib", Hint::Range(16.0, 65_536.0)),
    ("archive.max_entries", Hint::Range(0.0, 10_000_000.0)),
    ("archive.max_entry_mib", Hint::Range(0.0, 1_048_576.0)),
    ("archive.nested_max_mib", Hint::Range(0.0, 1_048_576.0)),
    ("archive.edit_level", Hint::Range(0.0, 9.0)),
    ("sync.change_debounce_secs", Hint::Range(1.0, 600.0)),
    ("sync.reports_kept", Hint::Range(1.0, 200.0)),
    ("sync.shadow_probe_max", Hint::Range(1.0, 512.0)),
    ("remote.cache_ttl_secs", Hint::Range(0.0, 3600.0)),
    ("remote.poll_secs", Hint::Range(0.0, 3600.0)),
    ("remote.connect_timeout_secs", Hint::Range(1.0, 300.0)),
    ("remote.io_timeout_secs", Hint::Range(5.0, 3600.0)),
    ("remote.preview_cache_mb", Hint::Range(16.0, 65_536.0)),
    ("remote.preview_max_mb", Hint::Range(1.0, 4096.0)),
    ("remote.share_minutes", Hint::Range(0.0, 1440.0)),
    ("remote.share_port", Hint::Range(0.0, 65_535.0)),
    ("terminal.scrollback", Hint::Range(1000.0, 100_000.0)),
    ("terminal.font_size", Hint::Range(8.0, 24.0)),
    ("preview.max_text_kib", Hint::Range(16.0, 16_384.0)),
    ("preview.max_image_mp", Hint::Range(4.0, 400.0)),
    ("preview.sheet_rows", Hint::Range(10.0, 100_000.0)),
    ("preview.max_document_mib", Hint::Range(1.0, 4096.0)),
    ("preview.max_model_mib", Hint::Range(1.0, 4096.0)),
    ("preview.max_audio_mib", Hint::Range(1.0, 16_384.0)),
    ("preview.max_diff_kib", Hint::Range(16.0, 65_536.0)),
    ("preview.thumb_cache_mib", Hint::Range(8.0, 4096.0)),
    ("preview.thumb_idle_trim_secs", Hint::Range(0.0, 86_400.0)),
    ("preview.preview_cache_mib", Hint::Range(0.0, 4096.0)),
    ("search.threads", Hint::Range(0.0, 64.0)),
    ("search.content_max_mib", Hint::Range(1.0, 1024.0)),
    ("git.refresh_secs", Hint::Range(1.0, 120.0)),
    ("editor.font_size", Hint::Range(8.0, 32.0)),
    ("editor.tab_width", Hint::Range(1.0, 16.0)),
    ("editor.max_text_mib", Hint::Range(1.0, 512.0)),
    ("editor.jpeg_quality", Hint::Range(50.0, 100.0)),
    ("editor.max_image_mp", Hint::Range(10.0, 1000.0)),
    ("ai.scope.max_read_kib", Hint::Range(16.0, 4096.0)),
    ("ai.request_timeout_secs", Hint::Range(5.0, 600.0)),
    ("ai.idle_timeout_secs", Hint::Range(10.0, 900.0)),
    ("ai.max_retries", Hint::Range(0.0, 10.0)),
    ("ai.budget_warn_percent", Hint::Range(10.0, 100.0)),
    ("ai.agent.max_steps", Hint::Range(1.0, 200.0)),
    ("ai.agent.token_budget", Hint::Range(0.0, 50_000_000.0)),
    ("ai.agent.time_limit_secs", Hint::Range(10.0, 86_400.0)),
    ("ai.agent.max_auto_reads", Hint::Range(1.0, 1000.0)),
    ("ai.agent.compact_after_kib", Hint::Range(8.0, 4096.0)),
    ("ai.agent.keep_recent_results", Hint::Range(0.0, 100.0)),
    ("ai.agent.listing_entries", Hint::Range(0.0, 2000.0)),
    ("ai.agent.name_excerpt_chars", Hint::Range(0.0, 20_000.0)),
    ("ai.agent.vision_max_px", Hint::Range(0.0, 2048.0)),
    ("ai.agent.name_max_files", Hint::Range(1.0, 500.0)),
    ("ai.mcp.http_port", Hint::Range(1024.0, 65_535.0)),
    ("ai.mcp.approval_timeout_secs", Hint::Range(10.0, 3600.0)),
    ("media.threads", Hint::Range(0.0, 64.0)),
    ("media.volume", Hint::Range(0.0, 100.0)),
    ("media.seek_step_secs", Hint::Range(0.5, 60.0)),
    ("media.max_px", Hint::Range(256.0, 4096.0)),
    ("media.tui_fps", Hint::Range(1.0, 30.0)),
    ("media.poster_secs", Hint::Range(0.0, 3600.0)),
    // Strings that are paths / programs, colours, chords.
    ("general.start_location", Hint::Path),
    ("terminal.shell", Hint::Path),
    ("preview.pdfium_dir", Hint::Path),
    ("media.ffmpeg_dir", Hint::Path),
    ("custom_commands[].command", Hint::Path),
    ("ai.mcp_servers[].command", Hint::Path),
    ("appearance.accent", Hint::Color),
    ("bookmarks[].hotkey", Hint::Chord),
    ("custom_commands[].hotkey", Hint::Chord),
    // Read once at startup (the GUI's descriptions say "after a restart").
    ("performance.gpu_backend", Hint::Restart),
    ("performance.gpu_memory", Hint::Restart),
    ("linux.window_backend", Hint::Restart),
    ("appearance.backdrop", Hint::Restart),
    ("appearance.snap_layouts", Hint::Restart),
    ("appearance.custom_titlebar", Hint::Restart),
    ("workspace.launch", Hint::Restart),
];

/// Keys the editor does not show: the schema version and legacy keys that are only read
/// (moved elsewhere on load, never written).
pub const HIDDEN: &[&str] = &["schema", "appearance.font", "appearance.mono_font"];

/// The editable schema, built on first use from `soyuz_core::config::meta::schema()`.
pub fn build() -> &'static SettingsSchema {
    static SCHEMA: OnceLock<SettingsSchema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let schema = meta::schema();
        let defaults = default_table();
        let mut out = SettingsSchema::default();
        walk_table(schema, &mut out, "", "Config", Some(defaults));
        out.tables.retain(|t| !(t.table.is_empty() && t.rows.is_empty()));
        out
    })
}

/// The element form of the array / map of tables whose elements are struct `name`
/// (`"Bookmark"`, `"AiProfile"`, `"AiMcpServer"`, …).
pub fn element_rows(name: &str) -> Option<&'static TableRows> {
    build().elements.get(name)
}

/// The row at `path`: a table key (`"tui.borders"`), an element field in its schema form
/// (`"bookmarks[].hotkey"`) or with an index (`"bookmarks[2].hotkey"`).
pub fn row(path: &str) -> Option<&'static FieldRow> {
    let norm = normalize(path);
    let s = build();
    s.tables.iter().chain(s.elements.values()).flat_map(|t| t.rows.iter()).find(|r| r.path == norm)
}

/// The table at `table` (`"tui"`, `"search.index"`).
pub fn table(table: &str) -> Option<&'static TableRows> {
    build().tables.iter().find(|t| t.table == table)
}

/// The tables (with their sub-tables) and keys a settings page edits, as groups of rows.
/// A key another page borrows (`general.default_view` on Views) shows on that page only.
pub fn tables_of(section: Section) -> Vec<TableRows> {
    let s = build();
    let borrowed: Vec<&str> = Section::ALL
        .iter()
        .flat_map(|sec| sec.tables().iter().copied())
        .filter(|p| row(p).is_some_and(|r| r.path == *p) && table(p).is_none())
        .collect();
    let mut out: Vec<TableRows> = Vec::new();
    for entry in section.tables() {
        let under = |t: &str| t == *entry || t.starts_with(&format!("{entry}."));
        if s.tables.iter().any(|t| under(&t.table)) {
            for t in s.tables.iter().filter(|t| under(&t.table)) {
                let rows: Vec<FieldRow> =
                    t.rows.iter().filter(|r| !borrowed.contains(&r.path.as_str())).cloned().collect();
                if !rows.is_empty() {
                    out.push(TableRows { rows, ..t.clone() });
                }
            }
        } else if let Some(r) = row(entry) {
            let parent = entry.rsplit_once('.').map_or("", |(p, _)| p);
            match out.iter_mut().find(|t| t.table == parent) {
                Some(t) => t.rows.push(r.clone()),
                None => out.push(TableRows {
                    table: parent.to_string(),
                    doc: table(parent).map(|t| t.doc.clone()).unwrap_or_default(),
                    rows: vec![r.clone()],
                }),
            }
        }
    }
    out
}

/// `"a[3].b"` → `"a[].b"`.
pub fn normalize(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut skip = false;
    for c in path.chars() {
        match c {
            '[' => {
                skip = true;
                out.push('[');
            }
            ']' => {
                skip = false;
                out.push(']');
            }
            _ if skip => {}
            _ => out.push(c),
        }
    }
    out
}

/// `Config::default()` as a TOML table (cached).
pub fn default_table() -> &'static toml::Table {
    static DEFAULTS: OnceLock<toml::Table> = OnceLock::new();
    DEFAULTS.get_or_init(|| to_table(&Config::default()))
}

/// A config as a TOML table (what `config.toml` would hold, every key written).
pub fn to_table(c: &Config) -> toml::Table {
    toml::Table::try_from(c).expect("config serializes to a TOML table")
}

/// A fresh element of the array / map of tables at `table` (`"bookmarks"`,
/// `"ai.profiles"`): an empty table run through the config's defaults.
pub fn element_default(table: &str) -> Option<toml::Table> {
    let FieldKind::Table { keyed, .. } = row(table)?.kind else { return None };
    element_defaults_for(table, keyed)
}

/// The value at a dotted path (`"a.b"`, with `[n]` indexing into arrays: `"a[2].b"`).
pub fn get_in<'a>(t: &'a toml::Table, path: &str) -> Option<&'a toml::Value> {
    let mut segs = segments(path)?.into_iter();
    let (first, idx) = segs.next()?;
    let mut cur = index(t.get(first)?, &idx)?;
    for (k, idx) in segs {
        cur = index(cur.as_table()?.get(k)?, &idx)?;
    }
    Some(cur)
}

/// Mutable [`get_in`].
pub fn get_in_mut<'a>(t: &'a mut toml::Table, path: &str) -> Option<&'a mut toml::Value> {
    let mut segs = segments(path)?.into_iter();
    let (first, idx) = segs.next()?;
    let mut cur = index_mut(t.get_mut(first)?, &idx)?;
    for (k, idx) in segs {
        cur = index_mut(cur.as_table_mut()?.get_mut(k)?, &idx)?;
    }
    Some(cur)
}

/// Set the value at `path` (creating missing tables on the way; array indices must exist).
/// Returns false when the path cannot be reached.
pub fn set_in(t: &mut toml::Table, path: &str, v: toml::Value) -> bool {
    let Some(mut segs) = segments(path) else { return false };
    let Some((last, last_idx)) = segs.pop() else { return false };
    let mut cur = t;
    for (k, idx) in segs {
        let entry = cur.entry(k.to_string()).or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let Some(next) = index_mut(entry, &idx).and_then(toml::Value::as_table_mut) else { return false };
        cur = next;
    }
    if last_idx.is_empty() {
        cur.insert(last.to_string(), v);
        return true;
    }
    match cur.get_mut(last).and_then(|e| index_mut(e, &last_idx)) {
        Some(slot) => {
            *slot = v;
            true
        }
        None => false,
    }
}

/// Remove the key at `path` (no-op when absent). Array elements are not removed here.
pub fn remove_in(t: &mut toml::Table, path: &str) {
    match path.rsplit_once('.') {
        Some((parent, key)) if !key.contains('[') => {
            if let Some(p) = get_in_mut(t, parent).and_then(toml::Value::as_table_mut) {
                p.remove(key);
            }
        }
        None if !path.contains('[') => {
            t.remove(path);
        }
        _ => {}
    }
}

type Segment<'a> = (&'a str, Vec<usize>);

fn segments(path: &str) -> Option<Vec<Segment<'_>>> {
    if path.is_empty() {
        return None;
    }
    path.split('.')
        .map(|seg| {
            let (name, rest) = seg.split_once('[').map_or((seg, ""), |(n, r)| (n, r));
            let mut idx = Vec::new();
            if !rest.is_empty() {
                for part in format!("[{rest}").split('[').skip(1) {
                    idx.push(part.strip_suffix(']')?.parse().ok()?);
                }
            }
            (!name.is_empty()).then_some((name, idx))
        })
        .collect()
}

fn index<'a>(mut v: &'a toml::Value, idx: &[usize]) -> Option<&'a toml::Value> {
    for i in idx {
        v = v.as_array()?.get(*i)?;
    }
    Some(v)
}

fn index_mut<'a>(mut v: &'a mut toml::Value, idx: &[usize]) -> Option<&'a mut toml::Value> {
    for i in idx {
        v = v.as_array_mut()?.get_mut(*i)?;
    }
    Some(v)
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() { name.to_string() } else { format!("{path}.{name}") }
}

/// Add the table of struct `ty` at `path` (and its sub-tables after it, in field order).
fn walk_table(schema: &Schema, out: &mut SettingsSchema, path: &str, ty: &str, value: Option<&toml::Table>) {
    let Some(st) = schema.structs.get(meta::last_segment(ty)) else { return };
    let mut rows = Vec::new();
    let mut nested: Vec<(String, String, String)> = Vec::new();
    for f in &st.fields {
        let fpath = join(path, &f.name);
        if HIDDEN.contains(&fpath.as_str()) {
            continue;
        }
        let fvalue = value.and_then(|t| t.get(&f.name));
        match meta::kind_of(schema, &f.ty) {
            Kind::Table(t) => nested.push((fpath, f.name.clone(), t.to_string())),
            Kind::ArrayOfTables(inner) => {
                rows.push(table_row(&fpath, &f.name, &f.doc, inner, false));
                add_element(schema, out, &fpath, inner, false);
            }
            Kind::MapOfTables(inner) => {
                rows.push(table_row(&fpath, &f.name, &f.doc, inner, true));
                add_element(schema, out, &fpath, inner, true);
            }
            Kind::Value => rows.push(field_row(schema, &fpath, &f.name, &f.doc, &f.ty, fvalue)),
        }
    }
    let doc = if path.is_empty() { String::new() } else { st.doc.clone() };
    out.tables.push(TableRows { table: path.to_string(), doc, rows });
    for (fpath, fname, t) in nested {
        let sub = value.and_then(|v| v.get(&fname)).and_then(toml::Value::as_table);
        walk_table(schema, out, &fpath, &t, sub);
    }
}

/// Register the element form of `[[path]]` / `[path.<name>]`.
fn add_element(schema: &Schema, out: &mut SettingsSchema, path: &str, ty: &str, keyed: bool) {
    let name = meta::last_segment(ty).to_string();
    if out.elements.contains_key(&name) {
        return;
    }
    // Placeholder first, so a self-referencing element cannot recurse forever.
    out.elements.insert(name.clone(), TableRows { table: String::new(), doc: String::new(), rows: Vec::new() });
    let prefix = format!("{path}[]");
    let defaults = element_defaults_for(path, keyed);
    let mut rows = Vec::new();
    element_fields(schema, out, &prefix, "", ty, defaults.as_ref(), &mut rows);
    let doc = schema.structs.get(&name).map(|s| s.doc.clone()).unwrap_or_default();
    out.elements.insert(name, TableRows { table: prefix, doc, rows });
}

/// Default element of the array at `path`, computed without [`row`] (the schema is still
/// being built).
fn element_defaults_for(path: &str, keyed: bool) -> Option<toml::Table> {
    if path.contains("[]") {
        return None;
    }
    let mut t = default_table().clone();
    let empty = toml::Value::Table(toml::Table::new());
    let v = if keyed {
        toml::Value::Table(toml::Table::from_iter([("x".to_string(), empty)]))
    } else {
        toml::Value::Array(vec![empty])
    };
    set_in(&mut t, path, v);
    let c = soyuz_core::config::parse_str(&toml::to_string(&t).ok()?).ok()?.config;
    match get_in(&to_table(&c), path)? {
        toml::Value::Array(a) => a.first()?.as_table().cloned(),
        toml::Value::Table(m) => m.get("x")?.as_table().cloned(),
        _ => None,
    }
}

/// Fields of an element struct, nested plain tables flattened (`sub.key`).
fn element_fields(
    schema: &Schema,
    out: &mut SettingsSchema,
    prefix: &str,
    sub: &str,
    ty: &str,
    value: Option<&toml::Table>,
    rows: &mut Vec<FieldRow>,
) {
    let Some(st) = schema.structs.get(meta::last_segment(ty)) else { return };
    for f in &st.fields {
        let name = join(sub, &f.name);
        let fpath = format!("{prefix}.{name}");
        let fvalue = value.and_then(|t| t.get(&f.name));
        match meta::kind_of(schema, &f.ty) {
            Kind::Table(t) => {
                let t = t.to_string();
                element_fields(schema, out, prefix, &name, &t, fvalue.and_then(toml::Value::as_table), rows);
            }
            Kind::ArrayOfTables(inner) => {
                rows.push(table_row(&fpath, &name, &f.doc, inner, false));
                add_element(schema, out, &fpath, inner, false);
            }
            Kind::MapOfTables(inner) => {
                rows.push(table_row(&fpath, &name, &f.doc, inner, true));
                add_element(schema, out, &fpath, inner, true);
            }
            Kind::Value => rows.push(field_row(schema, &fpath, &name, &f.doc, &f.ty, fvalue)),
        }
    }
}

fn table_row(path: &str, name: &str, doc: &str, inner: &str, keyed: bool) -> FieldRow {
    FieldRow {
        path: path.to_string(),
        name: name.to_string(),
        doc: doc.to_string(),
        kind: FieldKind::Table { element: meta::last_segment(inner).to_string(), keyed },
        restart: hint(path, |h| matches!(h, Hint::Restart)).is_some(),
        secret_ref: false,
    }
}

fn hint(path: &str, pick: impl Fn(&Hint) -> bool) -> Option<Hint> {
    HINTS.iter().filter(|(p, _)| *p == path).map(|(_, h)| *h).find(pick)
}

fn field_row(schema: &Schema, path: &str, name: &str, doc: &str, ty: &str, value: Option<&toml::Value>) -> FieldRow {
    let mut kind = classify(schema, ty, value);
    if let Some(Hint::Range(lo, hi)) = hint(path, |h| matches!(h, Hint::Range(..))) {
        match &mut kind {
            FieldKind::Int { min, max } => {
                *min = Some(lo as i64);
                *max = Some(hi as i64);
            }
            FieldKind::Float { min, max } => {
                *min = Some(lo);
                *max = Some(hi);
            }
            _ => {}
        }
    }
    if kind == FieldKind::Text {
        match hint(path, |h| matches!(h, Hint::Path | Hint::Color | Hint::Chord)) {
            Some(Hint::Path) => kind = FieldKind::Path,
            Some(Hint::Color) => kind = FieldKind::Color,
            Some(Hint::Chord) => kind = FieldKind::Chord,
            _ => {}
        }
    }
    let secret_ref = kind == FieldKind::Text && doc.contains("credential:<name>");
    FieldRow {
        path: path.to_string(),
        name: name.to_string(),
        doc: doc.to_string(),
        kind,
        restart: hint(path, |h| matches!(h, Hint::Restart)).is_some(),
        secret_ref,
    }
}

fn int_bounds(name: &str) -> Option<(Option<i64>, Option<i64>)> {
    Some(match name {
        "u8" => (Some(0), Some(u8::MAX as i64)),
        "u16" => (Some(0), Some(u16::MAX as i64)),
        "u32" => (Some(0), Some(u32::MAX as i64)),
        "u64" | "usize" => (Some(0), None),
        "i8" => (Some(i8::MIN as i64), Some(i8::MAX as i64)),
        "i16" => (Some(i16::MIN as i64), Some(i16::MAX as i64)),
        "i32" => (Some(i32::MIN as i64), Some(i32::MAX as i64)),
        "i64" | "isize" => (None, None),
        _ => return None,
    })
}

fn is_scalar(name: &str) -> bool {
    matches!(name, "String" | "PathBuf" | "str" | "bool" | "f32" | "f64") || int_bounds(name).is_some()
}

fn plain_enum<'a>(schema: &'a Schema, name: &str) -> Option<&'a Vec<(String, String)>> {
    schema.enums.get(name).and_then(|e| e.values.as_ref())
}

/// The editing kind of a Rust type (with the default value for types the schema does not
/// describe).
fn classify(schema: &Schema, ty: &str, value: Option<&toml::Value>) -> FieldKind {
    let ty = ty.trim();
    if ty.starts_with('[') {
        return FieldKind::TextList;
    }
    let name = meta::last_segment(ty);
    let args = meta::generic_args(ty);
    if let Some((min, max)) = int_bounds(name) {
        return FieldKind::Int { min, max };
    }
    match name {
        "Option" if !args.is_empty() => classify(schema, &args[0], value),
        "bool" => FieldKind::Bool,
        "f32" | "f64" => FieldKind::Float { min: None, max: None },
        "String" | "str" => FieldKind::Text,
        "PathBuf" => FieldKind::Path,
        "KeyChord" => FieldKind::Chord,
        "Vec" if !args.is_empty() => {
            let inner = meta::last_segment(&args[0]);
            if let Some(values) = plain_enum(schema, inner) {
                FieldKind::EnumList { values: values.clone() }
            } else if is_scalar(inner) {
                FieldKind::TextList
            } else {
                FieldKind::Opaque
            }
        }
        "BTreeMap" | "HashMap" if args.len() == 2 => {
            if is_scalar(meta::last_segment(&args[1])) {
                FieldKind::Map
            } else {
                FieldKind::Opaque
            }
        }
        "Table" | "Value" => FieldKind::Opaque,
        other => match plain_enum(schema, other) {
            Some(values) => FieldKind::Enum { values: values.clone() },
            None => match value {
                Some(toml::Value::String(_)) => FieldKind::Text,
                Some(toml::Value::Integer(_)) => FieldKind::Int { min: None, max: None },
                Some(toml::Value::Float(_)) => FieldKind::Float { min: None, max: None },
                Some(toml::Value::Boolean(_)) => FieldKind::Bool,
                Some(toml::Value::Array(a)) if a.iter().all(|v| !v.is_table() && !v.is_array()) => FieldKind::TextList,
                _ => FieldKind::Opaque,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_rows() -> Vec<&'static FieldRow> {
        let s = build();
        s.tables.iter().chain(s.elements.values()).flat_map(|t| t.rows.iter()).collect()
    }

    #[test]
    fn every_hint_path_exists_and_fits_its_kind() {
        for (path, h) in HINTS {
            let r = row(path).unwrap_or_else(|| panic!("hint for a missing key: {path}"));
            match h {
                Hint::Range(lo, hi) => {
                    assert!(lo < hi, "{path}");
                    assert!(
                        matches!(r.kind, FieldKind::Int { .. } | FieldKind::Float { .. } | FieldKind::TextList),
                        "{path}: {:?}",
                        r.kind
                    );
                }
                Hint::Path => assert_eq!(r.kind, FieldKind::Path, "{path}"),
                Hint::Color => assert_eq!(r.kind, FieldKind::Color, "{path}"),
                Hint::Chord => assert_eq!(r.kind, FieldKind::Chord, "{path}"),
                Hint::Restart => assert!(r.restart, "{path}"),
            }
        }
        for p in HIDDEN {
            assert!(row(p).is_none(), "hidden key shown: {p}");
            assert!(
                soyuz_core::config::meta::schema()
                    .structs
                    .values()
                    .any(|s| s.fields.iter().any(|f| p.rsplit('.').next() == Some(f.name.as_str())))
            );
        }
    }

    #[test]
    fn tui_ranges_equal_the_consts() {
        let int = |p: &str| match row(p).unwrap().kind {
            FieldKind::Int { min, max } => (min.unwrap(), max.unwrap()),
            ref k => panic!("{p}: {k:?}"),
        };
        let float = |p: &str| match row(p).unwrap().kind {
            FieldKind::Float { min, max } => (min.unwrap(), max.unwrap()),
            ref k => panic!("{p}: {k:?}"),
        };
        type T = TuiSection;
        assert_eq!(int("tui.sidebar_width"), (T::SIDEBAR_WIDTH_MIN as i64, T::SIDEBAR_WIDTH_MAX as i64));
        assert_eq!(int("tui.jobs_rows"), (T::JOBS_ROWS_MIN as i64, T::JOBS_ROWS_MAX as i64));
        assert_eq!(int("tui.terminal_rows"), (T::TERMINAL_ROWS_MIN as i64, T::TERMINAL_ROWS_MAX as i64));
        let (lo, hi) = float("tui.preview_ratio");
        assert_eq!((lo as f32, hi as f32), (T::PREVIEW_RATIO_MIN, T::PREVIEW_RATIO_MAX));
        let (lo, hi) = float("tui.ai_ratio");
        assert_eq!((lo as f32, hi as f32), (T::AI_RATIO_MIN, T::AI_RATIO_MAX));
        assert_eq!(row("tui.cell_pixels").unwrap().kind, FieldKind::TextList);
        assert_eq!(
            hint("tui.cell_pixels", |_| true),
            Some(Hint::Range(T::CELL_PIXELS_MIN as f64, T::CELL_PIXELS_MAX as f64))
        );
        // Every [tui] key is a row of the [tui] table.
        let tui = table("tui").unwrap();
        let keys = to_table(&Config::default())["tui"].as_table().unwrap().clone();
        let mut names: Vec<&str> = tui.rows.iter().map(|r| r.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, keys.keys().map(String::as_str).collect::<Vec<_>>());
        assert!(matches!(&row("tui.shell_mode").unwrap().kind, FieldKind::Enum { values } if values.len() == 2));
    }

    #[test]
    fn every_key_shows_on_exactly_one_page() {
        let s = build();
        let mut seen: BTreeMap<String, Vec<Section>> = BTreeMap::new();
        for sec in Section::ALL {
            for t in tables_of(sec) {
                for r in &t.rows {
                    seen.entry(r.path.clone()).or_default().push(sec);
                }
            }
        }
        for t in &s.tables {
            for r in &t.rows {
                assert_eq!(seen.get(&r.path).map(Vec::len), Some(1), "{}: {:?}", r.path, seen.get(&r.path));
            }
        }
        // Every top-level table of Config is owned (as a whole) by exactly one page.
        let config = &soyuz_core::config::meta::schema().structs["Config"];
        for f in &config.fields {
            if matches!(meta::kind_of(meta::schema(), &f.ty), Kind::Value) {
                continue;
            }
            let owners: Vec<Section> =
                Section::ALL.into_iter().filter(|s| s.tables().contains(&f.name.as_str())).collect();
            assert_eq!(owners.len(), 1, "{}: {owners:?}", f.name);
        }
        for sec in Section::ALL {
            for p in sec.tables().iter().chain(sec.kept()) {
                assert!(row(p).is_some() || table(p).is_some(), "{sec:?}: {p}");
            }
        }
    }

    #[test]
    fn kinds_cover_the_config() {
        let kind = |p: &str| row(p).unwrap_or_else(|| panic!("{p}")).kind.clone();
        assert_eq!(kind("general.show_hidden"), FieldKind::Bool);
        assert!(matches!(kind("tui.borders"), FieldKind::Enum { .. }));
        assert!(matches!(kind("columns.details"), FieldKind::EnumList { .. }));
        assert!(matches!(kind("custom_commands[].context"), FieldKind::EnumList { .. }));
        assert_eq!(kind("columns.widths"), FieldKind::Map);
        assert_eq!(kind("theme.colors"), FieldKind::Map);
        assert_eq!(kind("terminal.args"), FieldKind::TextList);
        assert_eq!(kind("bookmarks[].path"), FieldKind::Path);
        assert_eq!(kind("bookmarks"), FieldKind::Table { element: "Bookmark".into(), keyed: false });
        assert_eq!(kind("ai.profiles"), FieldKind::Table { element: "AiProfile".into(), keyed: false });
        assert_eq!(kind("ai.profiles[].headers"), FieldKind::Map);
        assert_eq!(kind("keys.bindings"), FieldKind::Opaque);
        // The element forms the TUI table editor needs.
        for el in [
            "Bookmark",
            "CustomCommand",
            "AiProfile",
            "AiPrice",
            "AiMcpClient",
            "AiMcpServer",
            "AiFolderRule",
            "AutomationRule",
            "PortalPin",
            "SavedSearchEntry",
        ] {
            assert!(element_rows(el).is_some_and(|t| !t.rows.is_empty()), "{el}");
        }
        // Opaque only where a form cannot represent the value.
        let opaque: Vec<&str> =
            all_rows().into_iter().filter(|r| r.kind == FieldKind::Opaque).map(|r| r.path.as_str()).collect();
        let allowed = ["search.saved[].query", "keys.bindings", "automation.rules[].actions"];
        assert!(opaque.iter().all(|p| allowed.contains(p)), "unexpected opaque rows: {opaque:?}");
    }

    #[test]
    fn credential_references_are_marked() {
        let mut secret: Vec<&str> = all_rows().into_iter().filter(|r| r.secret_ref).map(|r| r.path.as_str()).collect();
        secret.sort();
        assert_eq!(secret, ["ai.mcp_clients[].token", "ai.mcp_servers[].token", "ai.profiles[].key"]);
    }

    #[test]
    fn paths_index_into_arrays() {
        let mut t = default_table().clone();
        assert!(set_in(&mut t, "bookmarks", toml::Value::Array(vec![toml::Value::Table(toml::Table::new())])));
        assert!(set_in(&mut t, "bookmarks[0].name", toml::Value::String("x".into())));
        assert_eq!(get_in(&t, "bookmarks[0].name").and_then(toml::Value::as_str), Some("x"));
        assert!(get_in(&t, "bookmarks[1].name").is_none());
        assert!(!set_in(&mut t, "bookmarks[3].name", toml::Value::Boolean(true)));
        assert_eq!(normalize("ai.profiles[12].key"), "ai.profiles[].key");
        assert!(row("ai.profiles[0].key").is_some_and(|r| r.secret_ref));
        let el = element_default("ai.profiles").unwrap();
        assert!(el.contains_key("kind") && el.contains_key("name"), "{el:?}");
    }
}
