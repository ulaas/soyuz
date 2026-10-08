//! `soyuz-script`: automation for Soyuz (roadmap F6).
//!
//! * **Scripting** ([`ScriptSet`]): sandboxed [Rhai](https://rhai.rs) scripts in
//!   `scripts/*.rhai` that register custom actions (palette, context menu, key
//!   bindings), column providers, previewers and rule functions. Scripts read through a
//!   capped API and *plan* changes: every write becomes a [`PlannedJob`] the app
//!   submits through the jobs engine (preview / approval when configured, undo).
//! * **Rules** ([`rules`]): matching, action planning and scheduling for
//!   `[[automation.rules]]`; the app drives them from the folder watcher.
//! * **CLI companion** ([`cli`]): `soyuz cp|mv|rm|trash|ls|open|reveal|jobs`, sent to the
//!   running window over a local named pipe or run on a private headless engine.
#![deny(unsafe_code)]

pub mod api;
pub mod cli;
pub mod rules;
pub mod sandbox;
pub mod script;

use std::path::PathBuf;

pub use sandbox::{Limits, ScriptError};
pub use script::{ActionCtx, ActionDef, ColumnDef, MenuContext, Preview, PreviewerDef, Script, ScriptSet};

/// A change a script asked for; the host submits it as a job.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedJob {
    pub kind: soyuz_jobs::JobKind,
    /// Human summary ("Move “a.pdf” to “PDF”").
    pub summary: String,
}

/// A UI request of a script.
#[derive(Clone, Debug, PartialEq)]
pub enum UiOp {
    Navigate(PathBuf),
    /// Select these items (they share a folder; the pane navigates there).
    Select(Vec<PathBuf>),
    Toast(String),
    CopyText(String),
    /// Add a color tag (name or 1..8) to the paths.
    Tag(Vec<PathBuf>, String),
}

/// Everything one script call produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub jobs: Vec<PlannedJob>,
    pub ui: Vec<UiOp>,
    /// `print` / `debug` output.
    pub log: Vec<String>,
}

impl Outcome {
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty() && self.ui.is_empty()
    }
}

/// Definitions collected while a script loads (`register_*` calls).
#[derive(Clone, Default)]
pub(crate) struct Registry {
    pub actions: Vec<ActionDef>,
    pub columns: Vec<ColumnDef>,
    pub previewers: Vec<PreviewerDef>,
}

/// Directory of the scripts (`<config dir>/scripts`).
pub fn scripts_dir() -> PathBuf {
    soyuz_core::config::config_dir().join("scripts")
}

#[cfg(test)]
mod tests;
