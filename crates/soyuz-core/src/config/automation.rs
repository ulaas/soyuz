//! Scripting, rule-based automations and the CLI companion (roadmap F6):
//!
//! * `[scripting]` — Rhai scripts in `scripts/*.rhai` next to the config (custom actions,
//!   column providers, previewers, context-menu entries): sandbox limits and approval.
//! * `[automation]` + `[[automation.rules]]` — "when a file lands in Downloads matching
//!   `*.pdf` → move to Documents/PDF and tag blue", driven by the folder watcher while
//!   the app runs; every change is a job (undoable).
//! * `[cli]` — `soyuz cp|mv|rm|trash|…` submits jobs to the running window over a local
//!   named pipe.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// `[scripting]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scripting {
    /// Load `scripts/*.rhai` (off = no script actions, columns or previewers).
    pub enabled: bool,
    /// Rhai operations one script call may run before it is stopped.
    pub max_operations: u64,
    /// Wall-clock limit of an action / rule script call, in milliseconds.
    pub timeout_ms: u64,
    /// Wall-clock limit of one column value or previewer call, in milliseconds.
    pub cell_timeout_ms: u64,
    /// Scripts may read files (`read_text`, `list`, `stat`, `search`). Off = they only
    /// see the paths they are given.
    pub allow_read: bool,
    /// Largest `read_text` result, in KiB (longer files are cut).
    pub max_read_kib: u64,
    /// Most entries `list` / `search` return.
    pub max_list_entries: usize,
    /// Show the planned jobs of a script action for approval before they run. Off = they
    /// are submitted directly (still visible in the activity panel and undoable).
    pub approve_writes: bool,
    /// Script file names (`tidy.rhai`) that are not loaded.
    pub disabled: Vec<String>,
}

impl Default for Scripting {
    fn default() -> Self {
        Scripting {
            enabled: true,
            max_operations: 5_000_000,
            timeout_ms: 3_000,
            cell_timeout_ms: 250,
            allow_read: true,
            max_read_kib: 1024,
            max_list_entries: 10_000,
            approve_writes: true,
            disabled: Vec::new(),
        }
    }
}

/// `[automation]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Automation {
    /// Run the rules while Soyuz is open.
    pub enabled: bool,
    /// Global dry run: rules only log what they would do.
    pub dry_run: bool,
    /// Run-log entries kept per rule (in memory).
    pub log_entries: usize,
    pub rules: Vec<AutomationRule>,
}

impl Default for Automation {
    fn default() -> Self {
        Automation { enabled: true, dry_run: false, log_entries: 200, rules: Vec::new() }
    }
}

/// Which entries a rule considers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    #[default]
    Files,
    Folders,
    Any,
}

impl RuleKind {
    pub fn label(self) -> &'static str {
        match self {
            RuleKind::Files => "Files",
            RuleKind::Folders => "Folders",
            RuleKind::Any => "Files and folders",
        }
    }
}

/// What a rule does when the target name already exists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RuleConflict {
    /// Keep both (the new item gets a numbered name).
    #[default]
    KeepBoth,
    Skip,
    /// Replace the existing item (it goes to the undo history like any overwrite).
    Overwrite,
    /// Ask in the activity panel.
    Ask,
}

impl RuleConflict {
    pub fn label(self) -> &'static str {
        match self {
            RuleConflict::KeepBoth => "Keep both",
            RuleConflict::Skip => "Skip",
            RuleConflict::Overwrite => "Replace",
            RuleConflict::Ask => "Ask me",
        }
    }
}

/// One step of a rule, applied in order to each matching item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RuleAction {
    /// Move into this folder (created when missing).
    Move { to: PathBuf },
    /// Copy into this folder; later steps keep acting on the original.
    Copy { to: PathBuf },
    /// Rename with a pattern: `{name}` `{stem}` `{ext}` `{date}` `{time}` `{mdate}`
    /// `{parent}` (e.g. `"{mdate}-{name}"`).
    Rename { pattern: String },
    /// Add a color tag (`red` … `gray`).
    Tag { tag: String },
    /// Move to the trash.
    Trash,
    /// Run the `rule(path, ctx)` function of `scripts/<script>.rhai` (its file
    /// changes become jobs like everything else).
    Script { script: String },
}

impl RuleAction {
    pub fn label(&self) -> &'static str {
        match self {
            RuleAction::Move { .. } => "Move to",
            RuleAction::Copy { .. } => "Copy to",
            RuleAction::Rename { .. } => "Rename",
            RuleAction::Tag { .. } => "Tag",
            RuleAction::Trash => "Move to trash",
            RuleAction::Script { .. } => "Run script",
        }
    }
}

/// `[[automation.rules]]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomationRule {
    /// Rule name (shown in the run log and job titles).
    pub name: String,
    /// Run this rule.
    pub enabled: bool,
    /// Folder to watch.
    pub watch: PathBuf,
    /// Include subfolders.
    pub recursive: bool,
    /// Name globs separated by `;` (`"*.pdf;*.epub"`, empty = any name).
    pub glob: String,
    /// Optional regular expression on the name (case-insensitive).
    pub regex: String,
    /// Match files, folders or both.
    pub kind: RuleKind,
    /// Lower size bound in KiB (0 = no bound).
    pub min_size_kib: u64,
    /// Upper size bound in KiB (0 = no bound).
    pub max_size_kib: u64,
    /// Lower age bound by modification time, in minutes (0 = no bound).
    pub min_age_minutes: u64,
    /// Upper age bound in minutes (0 = no bound).
    pub max_age_minutes: u64,
    /// Wait this long after the last change before acting (lets downloads finish).
    pub delay_secs: u64,
    /// Also act on matching items already in the folder when the rule starts.
    pub apply_to_existing: bool,
    /// Only log what would happen.
    pub dry_run: bool,
    /// Name conflicts at the target.
    pub conflict: RuleConflict,
    /// Steps run on each match, in order.
    pub actions: Vec<RuleAction>,
}

impl Default for AutomationRule {
    fn default() -> Self {
        AutomationRule {
            name: String::new(),
            enabled: true,
            watch: PathBuf::new(),
            recursive: false,
            glob: String::new(),
            regex: String::new(),
            kind: RuleKind::Files,
            min_size_kib: 0,
            max_size_kib: 0,
            min_age_minutes: 0,
            max_age_minutes: 0,
            delay_secs: 3,
            apply_to_existing: false,
            dry_run: false,
            conflict: RuleConflict::KeepBoth,
            actions: Vec::new(),
        }
    }
}

/// `[cli]`: the `soyuz cp|mv|rm|…` companion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cli {
    /// The window accepts jobs from `soyuz cp|mv|rm|trash|open|reveal|jobs` over a local
    /// named pipe (current user only). Off = the CLI always runs its own engine.
    pub server_enabled: bool,
}

impl Default for Cli {
    fn default() -> Self {
        Cli { server_enabled: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_parse_from_toml() {
        let text = r#"
[automation]
dry_run = true
[[automation.rules]]
name = "PDFs"
watch = "C:/Users/me/Downloads"
glob = "*.pdf"
actions = [{ action = "move", to = "C:/Docs/PDF" }, { action = "tag", tag = "blue" }, { action = "trash" }]
"#;
        let l = crate::config::parse_str(text).unwrap();
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        let a = &l.config.automation;
        assert!(a.dry_run && a.enabled);
        let r = &a.rules[0];
        assert_eq!(r.delay_secs, 3);
        assert_eq!(r.actions.len(), 3);
        assert_eq!(r.actions[0], RuleAction::Move { to: "C:/Docs/PDF".into() });
        assert_eq!(r.actions[2], RuleAction::Trash);
        assert!(l.config.cli.server_enabled);
        assert!(l.config.scripting.approve_writes);
    }
}
