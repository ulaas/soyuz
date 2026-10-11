//! Configuration: schema, loading/saving, key bindings and actions.
//!
//! Hot reload is *not* here (core has no `notify`); see `soyuz-app/src/services/config_watch.rs`.

pub mod actions;
pub mod ai;
pub mod ai_agent;
pub mod automation;
pub mod defaults;
pub mod fonts;
pub mod keys;
pub mod mcp;
pub mod migrate;
pub mod preserve;
pub mod schema;

use std::io;
use std::path::{Path, PathBuf};

pub use actions::{Action, ActionCategory, ParseActionError};
pub use ai::{
    Ai, AiPrice, AiPrivacy, AiProfile, AiProviderKind, AiScope, AiTaskModel, AiTasks, AiToolMode, default_prices,
};
pub use ai_agent::{AiAccess, AiAgent, AiApproval, AiFolderRule, AiRiskCeiling, folder_access};
pub use automation::{Automation, AutomationRule, Cli, RuleAction, RuleConflict, RuleKind, Scripting};
pub use fonts::{FontRole, FontRoleKind, Fonts, HintingStrength};
pub use keys::{
    KeyChord, KeyContext, KeyName, KeyPreset, KeySequence, Keymap, Mods, ParseChordError, SeqMatch, format_sequence,
    parse_sequence,
};
pub use mcp::{AiMcp, AiMcpApprovalUi, AiMcpClient, AiMcpServer, AiRisk};
pub use migrate::{MigrateOutcome, migrate_file};
pub use preserve::to_string_preserving;
pub use schema::*;

use crate::error::{FsError, FsErrorKind};

/// A successfully parsed config plus non-fatal warnings (unknown keys, bad bindings).
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub config: Config,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config: {1}")]
    Io(FsErrorKind, String),
    #[error("config parse error at line {line}, column {col}: {msg}")]
    Parse { line: usize, col: usize, msg: String },
    #[error("config schema {0} is newer than this version of Soyuz supports")]
    Schema(u32),
}

impl ConfigError {
    fn io(op: &'static str, path: &Path, e: &io::Error) -> Self {
        let fe = FsError::from_io(op, Some(path), e);
        ConfigError::Io(fe.kind, fe.to_string())
    }
}

/// Parse a config file. Unknown keys become warnings (collected with `serde_ignored`).
pub fn load(path: &Path) -> Result<Loaded, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::io("load_config", path, &e))?;
    parse_str(&text)
}

/// [`load`], falling back to defaults when the file does not exist. Other errors are
/// returned so the app can show them (and keep its last good config).
pub fn load_or_default(path: &Path) -> Result<Loaded, ConfigError> {
    match load(path) {
        Err(ConfigError::Io(FsErrorKind::NotFound, _)) => {
            Ok(Loaded { config: Config::default(), warnings: Vec::new() })
        }
        other => other,
    }
}

/// Keys of removed settings: still accepted in old config files, without a warning.
/// (`startup_placeholder` / `cpu_first_frame`: the startup placeholder window and the
/// CPU-drawn first frames, removed 2026-10-09.)
pub const REMOVED_KEYS: &[&str] = &["appearance.startup_placeholder", "performance.cpu_first_frame"];

/// Parse config text (see [`load`]).
pub fn parse_str(text: &str) -> Result<Loaded, ConfigError> {
    let mut warnings = Vec::new();
    let de = toml::Deserializer::parse(text).map_err(|e| parse_error(text, &e))?;
    let mut config: Config = serde_ignored::deserialize(de, |path| {
        let path = path.to_string();
        if !REMOVED_KEYS.contains(&path.as_str()) {
            warnings.push(format!("unknown config key: {path}"));
        }
    })
    .map_err(|e| parse_error(text, &e))?;
    let a = &mut config.appearance;
    for note in config.fonts.adopt_legacy(&mut a.font, &mut a.mono_font) {
        log::info!("{note}");
    }
    if config.schema > SCHEMA_VERSION {
        return Err(ConfigError::Schema(config.schema));
    }
    for action in config.keys.bindings.keys() {
        if let Err(e) = action.parse::<Action>() {
            warnings.push(format!("keys.bindings: {e}"));
        }
    }
    Ok(Loaded { config, warnings })
}

fn parse_error(text: &str, e: &toml::de::Error) -> ConfigError {
    let (line, col) = match e.span() {
        Some(span) => line_col(text, span.start),
        None => (0, 0),
    };
    ConfigError::Parse { line, col, msg: e.message().to_string() }
}

/// 1-based line/column of a byte offset.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
    (line, col)
}

/// Serialize `config` as pretty TOML and write it atomically (temp file + rename).
pub fn save_pretty(path: &Path, config: &Config) -> Result<(), ConfigError> {
    let text = to_pretty_string(config)?;
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| ConfigError::io("save_config", dir, &e))?;
    }
    let tmp = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| ConfigError::io("save_config", &tmp, &e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ConfigError::io("save_config", path, &e)
    })
}

/// Pretty TOML text for `config`.
pub fn to_pretty_string(config: &Config) -> Result<String, ConfigError> {
    toml::to_string_pretty(config).map_err(|e| ConfigError::Parse { line: 0, col: 0, msg: e.to_string() })
}

/// `<exe_dir>/portable-data/config.toml` when a `portable` marker file sits next to the
/// executable (portable mode), else the platform config dir (`ProjectDirs("", "", "soyuz")`).
pub fn default_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Name of the portable-mode marker file next to the executable.
pub const PORTABLE_MARKER: &str = "portable";
/// Data directory used in portable mode, next to the executable.
pub const PORTABLE_DATA_DIR: &str = "portable-data";

/// Portable-mode data directory for an executable living in `exe_dir`: a `portable` marker
/// *file* selects `<exe_dir>/portable-data`; a `portable` *directory* (older layouts) is used
/// as the data directory itself. `None` when there is no marker.
pub fn portable_dir(exe_dir: &Path) -> Option<PathBuf> {
    let marker = exe_dir.join(PORTABLE_MARKER);
    if marker.is_file() {
        Some(exe_dir.join(PORTABLE_DATA_DIR))
    } else if marker.is_dir() {
        Some(marker)
    } else {
        None
    }
}

/// Directory holding `config.toml`, `session.toml`, `views.toml`, `history.jsonl`, `themes/`.
pub fn config_dir() -> PathBuf {
    if let Some(dir) =
        std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).and_then(|d| portable_dir(&d))
    {
        return dir;
    }
    match directories::ProjectDirs::from("", "", "soyuz") {
        Some(d) => d.config_dir().to_path_buf(),
        None => PathBuf::from("."),
    }
}

/// Directory for large machine-local data (the search index): the portable data dir in
/// portable mode, else the platform's local (non-roaming) data dir.
pub fn local_data_dir() -> PathBuf {
    if let Some(dir) =
        std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).and_then(|d| portable_dir(&d))
    {
        return dir;
    }
    match directories::ProjectDirs::from("", "", "soyuz") {
        Some(d) => d.data_local_dir().to_path_buf(),
        None => config_dir(),
    }
}

/// Build the effective key map: preset + `[keys.bindings]` overrides. Returns warnings
/// for invalid override entries.
pub fn keymap(config: &Config) -> (Keymap, Vec<String>) {
    let mut km = defaults::preset(config.keys.preset);
    let mut warnings = km.apply_overrides(&config.keys.bindings, config.keys.cmd_is_ctrl);
    // Bookmark and custom-command hotkeys (global; they take the chord from other actions).
    let extra =
        config
            .bookmarks
            .iter()
            .take(actions::BOOKMARK_SLOTS as usize)
            .enumerate()
            .map(|(i, b)| (format!("bookmarks[{i}].hotkey"), &b.hotkey, Action::OpenBookmark(i as u8 + 1)))
            .chain(config.custom_commands.iter().filter(|c| !c.name.trim().is_empty()).map(|c| {
                (format!("custom_commands.{}.hotkey", c.name), &c.hotkey, Action::CustomCommand(c.name.clone()))
            }));
    for (what, hotkey, action) in extra {
        if hotkey.trim().is_empty() {
            continue;
        }
        match parse_sequence(hotkey) {
            Ok(seq) if seq.len() == 1 => {
                km.bind_stealing(KeyContext::Global, action, seq[0].normalized(config.keys.cmd_is_ctrl))
            }
            Ok(seq) => km.bind_sequence(
                KeyContext::Global,
                action,
                seq.into_iter().map(|c| c.normalized(config.keys.cmd_is_ctrl)).collect(),
            ),
            Err(e) => warnings.push(format!("{what}: {e}")),
        }
    }
    (km, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columns::Column;
    use crate::size::SizeStyle;

    #[test]
    fn defaults_round_trip() {
        let c = Config::default();
        let s = to_pretty_string(&c).unwrap();
        let back = parse_str(&s).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        assert_eq!(back.config, c);
    }

    #[test]
    fn empty_and_partial_files() {
        let l = parse_str("").unwrap();
        assert_eq!(l.config, Config::default());
        let l = parse_str("[general]\nshow_hidden = true\nsize_style = \"decimal\"\n").unwrap();
        assert!(l.config.general.show_hidden);
        assert_eq!(l.config.general.size_style, SizeStyle::Decimal);
        assert!(l.config.general.confirm_delete, "untouched keys keep defaults");
        assert_eq!(l.config.columns.details[0], Column::Name);
    }

    #[test]
    fn unknown_keys_warn() {
        let l = parse_str("bogus = 1\n[general]\nshow_hiddn = true\n[nope]\nx = 2\n").unwrap();
        assert_eq!(l.warnings.len(), 3, "{:?}", l.warnings);
        assert!(l.warnings.iter().any(|w| w.contains("general.show_hiddn")), "{:?}", l.warnings);
    }

    #[test]
    fn removed_keys_are_ignored_silently() {
        let text = "[appearance]\nstartup_placeholder = true\n[performance]\ncpu_first_frame = false\n";
        let l = parse_str(text).unwrap();
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        assert_eq!(l.config, Config::default());
    }

    #[test]
    fn parse_error_location() {
        let err = parse_str("[general]\nshow_hidden = maybe\n").unwrap_err();
        match err {
            ConfigError::Parse { line, col, .. } => {
                assert_eq!(line, 2);
                assert!(col > 1);
            }
            other => panic!("{other:?}"),
        }
        let err = parse_str("[general]\nshow_hidden = \"yes\"\n").unwrap_err();
        assert!(matches!(err, ConfigError::Parse { line: 2, .. }), "{err:?}");
    }

    #[test]
    fn newer_schema_rejected() {
        assert_eq!(parse_str("schema = 99").unwrap_err(), ConfigError::Schema(99));
    }

    #[test]
    fn save_and_load_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("config.toml");
        let mut c = Config::default();
        c.general.show_hidden = true;
        c.keys.bindings.insert("copy".into(), vec!["Ctrl+Insert".into()]);
        c.bookmarks.push(Bookmark {
            name: "P".into(),
            path: "/p".into(),
            hotkey: "Ctrl+1".into(),
            icon: "code".into(),
        });
        save_pretty(&path, &c).unwrap();
        let l = load(&path).unwrap();
        assert_eq!(l.config, c);
        assert!(l.warnings.is_empty());
        let missing = load_or_default(&dir.path().join("missing.toml")).unwrap();
        assert_eq!(missing.config, Config::default());
        assert!(matches!(load(&dir.path().join("missing.toml")), Err(ConfigError::Io(FsErrorKind::NotFound, _))));
    }

    #[test]
    fn legacy_font_keys_migrate_into_roles() {
        let old = "[appearance]
font = \"Fira Sans\"   # mine
mono_font = \"JetBrains Mono\"
font_size = 14.0
";
        let l = parse_str(old).unwrap();
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        let c = &l.config;
        assert_eq!(c.fonts.ui.family, "Fira Sans");
        assert_eq!(c.fonts.terminal.family, fonts::Fonts::DEFAULT_MONO, "old default mono = new default");
        assert!(c.appearance.font.is_empty() && c.appearance.mono_font.is_empty());
        let text = to_string_preserving(old, c).unwrap();
        assert!(!text.contains("mono_font") && !text.contains("font = \"Fira"), "{text}");
        assert!(text.contains("font_size = 14.0"));
        let back = parse_str(&text).unwrap().config;
        assert_eq!(back.fonts, c.fonts);
        assert!(!to_pretty_string(&Config::default()).unwrap().contains("mono_font"));
    }

    #[test]
    fn example_config_parses_cleanly() {
        let text = include_str!("../../../../docs/config.example.toml");
        let l = parse_str(text).unwrap();
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        assert_eq!(l.config.bookmarks.len(), 1);
        assert_eq!(l.config.custom_commands.len(), 1);
        assert_eq!(l.config.search.saved.len(), 1);
        assert_eq!(l.config.automation.rules.len(), 1);
        assert_eq!(l.config.automation.rules[0].actions.len(), 2);
        let (km, w) = keymap(&l.config);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(km.lookup(KeyContext::FileList, "F5".parse().unwrap()), Some(&Action::CopyToOtherPane));
    }

    #[test]
    fn portable_mode_marker() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(portable_dir(dir.path()), None, "no marker: platform config dir");
        std::fs::write(dir.path().join(PORTABLE_MARKER), b"").unwrap();
        let data = portable_dir(dir.path()).unwrap();
        assert_eq!(data, dir.path().join(PORTABLE_DATA_DIR));
        // The data dir lives next to the marker file (they no longer collide).
        save_pretty(&data.join("config.toml"), &Config::default()).unwrap();
        assert!(data.join("config.toml").is_file());
        assert!(dir.path().join(PORTABLE_MARKER).is_file());
        assert_eq!(load(&data.join("config.toml")).unwrap().config, Config::default());

        let legacy = tempfile::tempdir().unwrap();
        std::fs::create_dir(legacy.path().join(PORTABLE_MARKER)).unwrap();
        assert_eq!(portable_dir(legacy.path()), Some(legacy.path().join(PORTABLE_MARKER)));
    }

    #[test]
    fn bookmark_and_command_hotkeys() {
        let mut c = Config::default();
        c.bookmarks.push(Bookmark {
            name: "P".into(),
            path: "/p".into(),
            hotkey: "Ctrl+1".into(),
            icon: String::new(),
        });
        c.bookmarks.push(Bookmark {
            name: "Q".into(),
            path: "/q".into(),
            hotkey: "Nope+1".into(),
            icon: String::new(),
        });
        c.custom_commands.push(CustomCommand { name: "Edit".into(), hotkey: "Ctrl+K E".into(), ..Default::default() });
        let (km, w) = keymap(&c);
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(km.lookup(KeyContext::FileList, "Ctrl+1".parse().unwrap()), Some(&Action::OpenBookmark(1)));
        assert!(km.chords_for(&Action::SelectTab(1)).is_empty(), "the bookmark took Ctrl+1");
        let seq = parse_sequence("Ctrl+K E").unwrap();
        assert_eq!(
            km.sequence_match(KeyContext::FileList, &seq),
            SeqMatch::Exact(Action::CustomCommand("Edit".into()))
        );
    }

    #[test]
    fn default_path_is_toml() {
        assert!(default_path().ends_with("config.toml"));
    }

    #[test]
    fn tag_names_back_compat_and_round_trip() {
        // A config written before `[tags]` existed: every tag keeps its color name.
        let old = parse_str(
            "[general]
show_hidden = true
",
        )
        .unwrap()
        .config;
        assert_eq!(old.tags, TagsSection::default());
        assert!((0..8).all(|i| old.tags.custom_name(i).is_none()));
        // Fewer entries than tags: the rest are defaults.
        let short = parse_str(
            "[tags]
names = [\"Work\", \"\"]
",
        )
        .unwrap();
        assert!(short.warnings.is_empty(), "{:?}", short.warnings);
        assert_eq!(short.config.tags.custom_name(0).as_deref(), Some("Work"));
        assert_eq!(short.config.tags.custom_name(1), None);
        assert_eq!(short.config.tags.custom_name(7), None);
        // Round trip through both writers.
        let mut c = Config::default();
        c.tags.names[2] = "Urgent".into();
        let text = to_pretty_string(&c).unwrap();
        assert_eq!(parse_str(&text).unwrap().config.tags, c.tags);
        let merged = to_string_preserving(
            "[general]
show_hidden = true # mine
",
            &c,
        )
        .unwrap();
        assert!(merged.contains("# mine") && merged.contains("Urgent"), "{merged}");
        assert_eq!(parse_str(&merged).unwrap().config.tags, c.tags);
        // Defaults are not written into a user file.
        let untouched = to_string_preserving(
            "[general]
show_hidden = true
",
            &old,
        )
        .unwrap();
        assert!(!untouched.contains("[tags]"), "{untouched}");
    }

    #[test]
    fn tag_name_cleaning() {
        assert_eq!(clean_tag_name("  Work  "), "Work");
        assert_eq!(clean_tag_name("   "), "");
        assert_eq!(
            clean_tag_name(
                "a
b	c"
            ),
            "a b c"
        );
        let long = "x".repeat(100);
        assert_eq!(clean_tag_name(&long).chars().count(), TAG_NAME_MAX_CHARS);
        // Cut at characters, not bytes; no trailing space after the cut.
        let turkish = "ğ".repeat(39) + " çok uzun";
        let cut = clean_tag_name(&turkish);
        assert_eq!(cut, "ğ".repeat(39));
        let t = TagsSection { names: vec!["  ".into(), " Home ".into()], ..Default::default() };
        assert_eq!(t.custom_name(0), None);
        assert_eq!(t.custom_name(1).as_deref(), Some("Home"));
    }
}
