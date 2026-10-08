//! Shared prompt vocabulary spoken by jobs, archive and app.

use serde::{Deserialize, Serialize};

/// Pre-answered overwrite policy for a single copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Overwrite {
    #[default]
    Never,
    Always,
    IfNewer,
    IfLarger,
    IfDifferentSize,
}

/// Answer to a name conflict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    Overwrite,
    OverwriteIfNewer,
    OverwriteIfLarger,
    OverwriteIfDifferent,
    Skip,
    /// Auto-rename via [`unique_name`](crate::unique_name).
    KeepBoth,
    Rename(String),
    MergeDir,
    Cancel,
}

/// Answer to an error prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorAction {
    Retry,
    Skip,
    SkipAll,
    Abort,
}
