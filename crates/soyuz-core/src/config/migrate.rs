//! Versioned `config.toml` migrations.
//!
//! The schema version lives in the top-level `schema` key (`version` is accepted as an
//! alias; a file without either is version 1, the first released schema). When Soyuz
//! starts with a file older than [`SCHEMA_VERSION`], [`migrate_file`] copies it to
//! `config.toml.bak-v<N>` (N = the old version, never overwriting an existing backup) and
//! rewrites it through every [`Migration`] step in order, editing the document with
//! `toml_edit` so comments and layout survive. A file *newer* than this build is left alone
//! (loading reports [`ConfigError::Schema`]).
//!
//! Adding a migration: bump [`SCHEMA_VERSION`], append a `Migration { from: old, .. }` to
//! [`MIGRATIONS`] that turns an `old` document into an `old + 1` one, and add a test.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, value};

use super::{ConfigError, SCHEMA_VERSION};

/// One step: turns a document of schema `from` into schema `from + 1`. The framework
/// updates the version key itself afterwards.
#[derive(Clone, Copy)]
pub struct Migration {
    pub from: u32,
    pub describe: &'static str,
    pub apply: fn(&mut DocumentMut) -> Result<(), String>,
}

impl std::fmt::Debug for Migration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration").field("from", &self.from).field("describe", &self.describe).finish()
    }
}

/// The registered migrations (schema 1 is the first released schema).
pub const MIGRATIONS: &[Migration] = &[Migration {
    from: 1,
    describe: "sidebar: portal section, new section order, disk tree as its own view",
    apply: sidebar_portal_v2,
}];

/// The schema 1 default `[sidebar] sections`.
const SIDEBAR_V1_DEFAULT: [&str; 8] = ["favorites", "smart", "devices", "cloud", "tags", "network", "trash", "tree"];
/// The schema 2 default `[sidebar] sections`.
const SIDEBAR_V2_DEFAULT: [&str; 8] = ["favorites", "devices", "portal", "tags", "trash", "network", "cloud", "smart"];

/// v1 → v2: a `[sidebar] sections` list equal to the old default becomes the new default
/// (and `mode = "both"`, the old default, becomes `"places"`: the disk tree moved to its
/// own full-height view). A customized list gets `portal` after `devices` (or at the end)
/// and keeps the tree: a missing `mode` meant `both` in schema 1, so it is written out.
fn sidebar_portal_v2(doc: &mut DocumentMut) -> Result<(), String> {
    let Some(sidebar) = doc.get_mut("sidebar").and_then(|s| s.as_table_like_mut()) else { return Ok(()) };
    let Some(arr) = sidebar.get("sections").and_then(|i| i.as_array()) else { return Ok(()) };
    let names: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
    if names.len() != arr.len() {
        return Err("`[sidebar] sections` must be a list of strings".into());
    }
    let mode = sidebar.get("mode").and_then(|m| m.as_str()).map(str::to_string);
    if names.iter().map(String::as_str).eq(SIDEBAR_V1_DEFAULT) {
        if let Some(a) = sidebar.get_mut("sections").and_then(|i| i.as_array_mut()) {
            a.clear();
            a.extend(SIDEBAR_V2_DEFAULT);
        }
        if mode.as_deref().is_none_or(|m| m == "both") {
            sidebar.insert("mode", value("places"));
        }
        return Ok(());
    }
    if !names.iter().any(|n| n == "portal") {
        let at = names.iter().position(|n| n == "devices").map_or(names.len(), |i| i + 1);
        if let Some(a) = sidebar.get_mut("sections").and_then(|i| i.as_array_mut()) {
            a.insert(at, "portal");
            a.fmt();
        }
    }
    if mode.is_none() {
        sidebar.insert("mode", value("both"));
    }
    Ok(())
}

/// Result of a migration run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrateOutcome {
    /// The file is missing or already current (or newer: loading reports that).
    UpToDate,
    /// Migrated from `from` to `to`; the original was saved as `backup`.
    Migrated { from: u32, to: u32, backup: PathBuf },
}

/// Schema version of a parsed document (`schema`, alias `version`; missing = 1).
pub fn doc_version(doc: &DocumentMut) -> Result<u32, ConfigError> {
    let item = doc.get("schema").or_else(|| doc.get("version"));
    match item {
        None => Ok(1),
        Some(i) => i.as_integer().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| ConfigError::Parse {
            line: 0,
            col: 0,
            msg: "`schema` must be a positive integer".into(),
        }),
    }
}

/// Migrate config text with an explicit migration table up to `target`. `Ok(None)` when no
/// change is needed (already at or past `target`).
pub fn migrate_text_with(
    text: &str,
    migrations: &[Migration],
    target: u32,
) -> Result<Option<(String, u32)>, ConfigError> {
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        let (line, col) = e.span().map(|s| super::line_col(text, s.start)).unwrap_or((0, 0));
        ConfigError::Parse { line, col, msg: e.message().to_string() }
    })?;
    let from = doc_version(&doc)?;
    if from >= target {
        return Ok(None);
    }
    let mut v = from;
    while v < target {
        let step = migrations.iter().find(|m| m.from == v).ok_or_else(|| ConfigError::Parse {
            line: 0,
            col: 0,
            msg: format!("no migration from config schema {v}"),
        })?;
        (step.apply)(&mut doc).map_err(|msg| ConfigError::Parse {
            line: 0,
            col: 0,
            msg: format!("migration {v} → {} ({}) failed: {msg}", v + 1, step.describe),
        })?;
        v += 1;
    }
    if doc.contains_key("version") && !doc.contains_key("schema") {
        doc.remove("version");
    }
    doc["schema"] = value(i64::from(v));
    Ok(Some((doc.to_string(), from)))
}

/// Backup path for a file of schema `version`: `<name>.bak-v<version>` (a numbered suffix
/// is added when that name is taken, so an older backup is never overwritten).
pub fn backup_path(path: &Path, version: u32) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "config.toml".into());
    let first = path.with_file_name(format!("{name}.bak-v{version}"));
    if !first.exists() {
        return first;
    }
    (2..1000).map(|n| path.with_file_name(format!("{name}.bak-v{version}.{n}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// [`migrate_file`] with an explicit migration table and target (tests, future tooling).
pub fn migrate_file_with(path: &Path, migrations: &[Migration], target: u32) -> Result<MigrateOutcome, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(MigrateOutcome::UpToDate),
        Err(e) => return Err(ConfigError::io("migrate_config", path, &e)),
    };
    let Some((new_text, from)) = migrate_text_with(&text, migrations, target)? else {
        return Ok(MigrateOutcome::UpToDate);
    };
    let backup = backup_path(path, from);
    std::fs::write(&backup, &text).map_err(|e| ConfigError::io("backup_config", &backup, &e))?;
    let tmp = path.with_extension(format!("toml.migrate-{}", std::process::id()));
    std::fs::write(&tmp, new_text).map_err(|e| ConfigError::io("migrate_config", &tmp, &e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ConfigError::io("migrate_config", path, &e)
    })?;
    Ok(MigrateOutcome::Migrated { from, to: target, backup })
}

/// Bring `path` up to [`SCHEMA_VERSION`] with the registered [`MIGRATIONS`] (backup first).
pub fn migrate_file(path: &Path) -> Result<MigrateOutcome, ConfigError> {
    migrate_file_with(path, MIGRATIONS, SCHEMA_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename_key(doc: &mut DocumentMut) -> Result<(), String> {
        // v1 → v2: `[general] old_name` became `[general] new_name`.
        let general = doc.get_mut("general").and_then(|g| g.as_table_like_mut()).ok_or("no [general]")?;
        if let Some(v) = general.remove("old_name") {
            general.insert("new_name", v);
        }
        Ok(())
    }

    fn add_section(doc: &mut DocumentMut) -> Result<(), String> {
        let mut t = toml_edit::Table::new();
        t.insert("enabled", value(true));
        doc.insert("extra", toml_edit::Item::Table(t));
        Ok(())
    }

    fn failing(_: &mut DocumentMut) -> Result<(), String> {
        Err("boom".into())
    }

    const TABLE: &[Migration] = &[
        Migration { from: 1, describe: "rename old_name", apply: rename_key },
        Migration { from: 2, describe: "add [extra]", apply: add_section },
    ];

    #[test]
    fn registered_table_is_consistent() {
        // Every version below the current one has exactly one step.
        let versions: Vec<u32> = (1..=SCHEMA_VERSION).collect();
        for &v in &versions[..versions.len() - 1] {
            assert_eq!(MIGRATIONS.iter().filter(|m| m.from == v).count(), 1, "migration from {v}");
        }
        assert!(MIGRATIONS.iter().all(|m| m.from < SCHEMA_VERSION));
        let current = format!("schema = {SCHEMA_VERSION}\n");
        assert_eq!(migrate_text_with(&current, MIGRATIONS, SCHEMA_VERSION).unwrap(), None);
    }

    #[test]
    fn version_key_and_alias() {
        assert_eq!(doc_version(&"".parse().unwrap()).unwrap(), 1);
        assert_eq!(doc_version(&"schema = 3".parse().unwrap()).unwrap(), 3);
        assert_eq!(doc_version(&"version = 2".parse().unwrap()).unwrap(), 2);
        assert!(doc_version(&"schema = \"x\"".parse().unwrap()).is_err());
        assert!(doc_version(&"schema = -1".parse().unwrap()).is_err());
    }

    #[test]
    fn chained_migrations_keep_comments() {
        let text = "# my settings\nschema = 1\n\n[general]\n# keep me\nother = 1\nold_name = 5\n";
        let (out, from) = migrate_text_with(text, TABLE, 3).unwrap().unwrap();
        assert_eq!(from, 1);
        assert!(out.contains("# my settings") && out.contains("# keep me"), "{out}");
        assert!(out.contains("new_name = 5") && !out.contains("old_name"), "{out}");
        assert!(out.contains("schema = 3"), "{out}");
        assert!(out.contains("[extra]") && out.contains("enabled = true"), "{out}");
        // Partial: only up to 2.
        let (out2, _) = migrate_text_with(text, TABLE, 2).unwrap().unwrap();
        assert!(out2.contains("schema = 2") && !out2.contains("[extra]"), "{out2}");
        // Already there / newer: nothing to do.
        assert_eq!(migrate_text_with("schema = 3", TABLE, 3).unwrap(), None);
        assert_eq!(migrate_text_with("schema = 9", TABLE, 3).unwrap(), None);
    }

    #[test]
    fn missing_step_and_failing_step_are_errors() {
        assert!(migrate_text_with("schema = 1", &TABLE[1..], 3).is_err());
        let bad = [Migration { from: 1, describe: "fails", apply: failing }];
        let e = migrate_text_with("schema = 1", &bad, 2).unwrap_err();
        assert!(e.to_string().contains("boom"), "{e}");
        assert!(migrate_text_with("[[[", TABLE, 3).is_err());
    }

    #[test]
    fn version_alias_is_replaced_by_schema() {
        let (out, from) = migrate_text_with("version = 2\n", TABLE, 3).unwrap().unwrap();
        assert_eq!(from, 2);
        assert!(out.contains("schema = 3") && !out.contains("version"), "{out}");
    }

    #[test]
    fn file_migration_writes_backup_once_per_version() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        assert_eq!(migrate_file_with(&p, TABLE, 3).unwrap(), MigrateOutcome::UpToDate, "missing file");
        let original = "schema = 1\n[general]\nold_name = 1\n";
        std::fs::write(&p, original).unwrap();
        let out = migrate_file_with(&p, TABLE, 3).unwrap();
        let MigrateOutcome::Migrated { from: 1, to: 3, backup } = out else { panic!("{out:?}") };
        assert_eq!(backup, d.path().join("config.toml.bak-v1"));
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        assert!(std::fs::read_to_string(&p).unwrap().contains("schema = 3"));
        assert_eq!(migrate_file_with(&p, TABLE, 3).unwrap(), MigrateOutcome::UpToDate, "idempotent");
        // A second v1 file never overwrites the first backup.
        std::fs::write(&p, original).unwrap();
        let MigrateOutcome::Migrated { backup: b2, .. } = migrate_file_with(&p, TABLE, 3).unwrap() else { panic!() };
        assert_eq!(b2, d.path().join("config.toml.bak-v1.2"));
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        // A broken file is not touched.
        std::fs::write(&p, "schema = 1\n[[[").unwrap();
        assert!(migrate_file_with(&p, TABLE, 3).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "schema = 1\n[[[");
        // The migrated result loads as a config (unknown keys are only warnings).
        assert_eq!(migrate_file(&d.path().join("absent.toml")).unwrap(), MigrateOutcome::UpToDate);
    }

    #[test]
    fn loader_accepts_version_alias() {
        let l = super::super::parse_str("version = 1\n").unwrap();
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        assert_eq!(l.config.schema, 1);
    }

    fn migrate_sidebar(text: &str) -> super::super::Sidebar {
        let (out, from) = migrate_text_with(text, MIGRATIONS, 2).unwrap().unwrap();
        assert_eq!(from, 1);
        assert!(out.contains("schema = 2"), "{out}");
        // Running the migration again is a no-op (the schema key says it is done).
        assert_eq!(migrate_text_with(&out, MIGRATIONS, 2).unwrap(), None);
        let l = super::super::parse_str(&out).unwrap();
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        l.config.sidebar
    }

    #[test]
    fn sidebar_old_default_becomes_new_default() {
        use super::super::{Sidebar, SidebarMode};
        let text = "[sidebar]\nsections = [\"favorites\", \"smart\", \"devices\", \"cloud\", \"tags\", \"network\", \"trash\", \"tree\"]  # mine\nmode = \"both\"\ntree_follow = false\n";
        let s = migrate_sidebar(text);
        assert_eq!(s.sections, Sidebar::default().sections);
        assert_eq!(s.mode, SidebarMode::Places);
        assert!(!s.tree_follow, "other keys are kept");
        let (out, _) = migrate_text_with(text, MIGRATIONS, 2).unwrap().unwrap();
        assert!(out.contains("# mine"), "{out}");
        // An explicit non-default mode is kept.
        let s = migrate_sidebar(&text.replace("\"both\"", "\"tree\""));
        assert_eq!(s.mode, SidebarMode::Tree);
        // The default list with no mode key: the tree moves to its own view.
        let s = migrate_sidebar(&text.replace("mode = \"both\"\n", ""));
        assert_eq!(s.mode, SidebarMode::Places);
    }

    #[test]
    fn sidebar_custom_list_gets_portal_once() {
        use super::super::SidebarMode;
        use super::super::SidebarSection::*;
        let s = migrate_sidebar("[sidebar]\nsections = [\"devices\", \"favorites\", \"tree\"]\n");
        assert_eq!(s.sections, vec![Devices, Portal, Favorites, Tree]);
        assert_eq!(s.mode, SidebarMode::Both, "a missing mode meant both in schema 1");
        let s = migrate_sidebar("[sidebar]\nsections = [\"favorites\", \"tags\"]\nmode = \"tree\"\n");
        assert_eq!(s.sections, vec![Favorites, Tags, Portal]);
        assert_eq!(s.mode, SidebarMode::Tree);
        // Already listed: untouched.
        let s = migrate_sidebar("[sidebar]\nsections = [\"portal\", \"favorites\"]\nmode = \"places\"\n");
        assert_eq!(s.sections, vec![Portal, Favorites]);
        assert_eq!(s.mode, SidebarMode::Places);
        // The step itself is idempotent too.
        let mut doc: DocumentMut = "[sidebar]\nsections = [\"devices\"]\n".parse().unwrap();
        sidebar_portal_v2(&mut doc).unwrap();
        sidebar_portal_v2(&mut doc).unwrap();
        assert_eq!(doc["sidebar"]["sections"].as_array().unwrap().len(), 2, "{doc}");
    }

    #[test]
    fn sidebar_without_sections_is_untouched() {
        let (out, _) = migrate_text_with("[general]\nshow_hidden = true\n", MIGRATIONS, 2).unwrap().unwrap();
        assert!(!out.contains("[sidebar]"), "{out}");
        let (out, _) = migrate_text_with("[sidebar]\ntree_follow = false\n", MIGRATIONS, 2).unwrap().unwrap();
        assert!(!out.contains("sections") && !out.contains("mode"), "{out}");
        assert!(migrate_text_with("[sidebar]\nsections = [1]\n", MIGRATIONS, 2).is_err());
    }
}
