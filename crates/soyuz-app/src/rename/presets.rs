//! Batch rename presets: a few built-ins plus user presets stored in
//! `rename_presets.toml` next to `config.toml` (same pattern as saved searches).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{CaseMode, Counter, Position, Rule, RuleEntry, Spec, Target};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(flatten)]
    pub spec: Spec,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    preset: Vec<Preset>,
}

/// `<config dir>/rename_presets.toml`.
pub fn default_path() -> PathBuf {
    soyuz_core::config::config_dir().join("rename_presets.toml")
}

/// Load user presets (a missing file yields none; a broken one is backed up to `.bak`,
/// logged, and protected from being overwritten this session — see `store_file`).
pub fn load(path: &Path) -> Vec<Preset> {
    crate::services::store_file::load_toml::<File>(path).value().preset
}

/// Save user presets atomically (refused when the file was unreadable at load).
pub fn save(path: &Path, presets: &[Preset]) -> Result<(), String> {
    let text = toml::to_string_pretty(&File { preset: presets.to_vec() }).map_err(|e| e.to_string())?;
    crate::services::store_file::save_sync(path, text.as_bytes())
}

/// Presets that ship with the app.
pub fn builtin() -> Vec<Preset> {
    let p = |name: &str, target: Target, counter: Counter, rules: Vec<Rule>| Preset {
        name: name.into(),
        spec: Spec { target, counter, rules: rules.into_iter().map(RuleEntry::new).collect() },
    };
    vec![
        p(
            "Photos by capture date",
            Target::Stem,
            Counter { start: 1, step: 1, pad: 3, per_folder: false },
            vec![Rule::Template { pattern: "{exif:%Y-%m-%d} {n}".into() }],
        ),
        p(
            "Folder name + number",
            Target::Stem,
            Counter::default(),
            vec![Rule::Template { pattern: "{parent} {n}".into() }],
        ),
        p(
            "Spaces to underscores",
            Target::Stem,
            Counter::default(),
            vec![
                Rule::Trim { collapse: true },
                Rule::Replace {
                    find: " ".into(),
                    with: "_".into(),
                    regex: false,
                    case_sensitive: true,
                    first_only: false,
                },
            ],
        ),
        p("Lower-case extensions", Target::Extension, Counter::default(), vec![Rule::Case { mode: CaseMode::Lower }]),
        p(
            "Prefix with modified date",
            Target::Stem,
            Counter::default(),
            vec![Rule::Insert { text: "{date:%Y%m%d}_".into(), at: Position::Start }],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("rename_presets.toml");
        assert!(load(&path).is_empty());
        let presets = builtin();
        save(&path, &presets).unwrap();
        assert_eq!(load(&path), presets);
        std::fs::write(&path, "not = [valid").unwrap();
        assert!(load(&path).is_empty());
    }
}
