//! "Apply to all" memory for prompts.
//!
//! Each job has a *preset* built from its [`JobOptions`](crate::JobOptions) (pre-answered
//! conflict resolution / error action / `CopyOptions::overwrite`) and a *memory* of the
//! answers the user gave with "apply to all". The memory is per job, or engine-wide when
//! `EngineConfig::reset_apply_all_between_jobs` is `false`.

use soyuz_core::{ConflictResolution, ErrorAction, Overwrite};

/// Remembered answers. `None` = ask.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PolicyMemory {
    pub conflict: Option<ConflictResolution>,
    /// Only `Skip` or `Abort` are ever stored (`SkipAll` is normalized to `Skip`).
    pub error: Option<ErrorAction>,
    /// `true` = clear the read-only flag and continue.
    pub read_only: Option<bool>,
    /// `true` = delete permanently, `false` = skip.
    pub trash_unsupported: Option<bool>,
}

impl PolicyMemory {
    /// Preset from job options. `errors: Some(Retry)` is not stored here (the job context
    /// handles it as "retry a few times automatically, then ask").
    pub fn preset(conflict: Option<ConflictResolution>, errors: Option<ErrorAction>, overwrite: Overwrite) -> Self {
        PolicyMemory {
            conflict: conflict.or_else(|| overwrite_to_resolution(overwrite)).map(normalize_conflict_for_all),
            error: errors.and_then(remembered_error),
            read_only: None,
            trash_unsupported: None,
        }
    }
}

/// `CopyOptions::overwrite` expressed as a pre-answered conflict resolution.
pub(crate) fn overwrite_to_resolution(o: Overwrite) -> Option<ConflictResolution> {
    match o {
        Overwrite::Never => None,
        Overwrite::Always => Some(ConflictResolution::Overwrite),
        Overwrite::IfNewer => Some(ConflictResolution::OverwriteIfNewer),
        Overwrite::IfLarger => Some(ConflictResolution::OverwriteIfLarger),
        Overwrite::IfDifferentSize => Some(ConflictResolution::OverwriteIfDifferent),
    }
}

/// The value stored for an "apply to all" error answer, if any.
pub(crate) fn remembered_error(a: ErrorAction) -> Option<ErrorAction> {
    match a {
        ErrorAction::Skip | ErrorAction::SkipAll => Some(ErrorAction::Skip),
        ErrorAction::Abort => Some(ErrorAction::Abort),
        ErrorAction::Retry => None,
    }
}

/// A concrete `Rename(name)` cannot apply to every conflict; it becomes `KeepBoth`.
pub(crate) fn normalize_conflict_for_all(r: ConflictResolution) -> ConflictResolution {
    match r {
        ConflictResolution::Rename(_) => ConflictResolution::KeepBoth,
        other => other,
    }
}

/// Overwrite policy handed to `soyuz_fs::copy_file` for a resolved conflict.
pub(crate) fn resolution_to_overwrite(r: &ConflictResolution) -> Option<Overwrite> {
    match r {
        ConflictResolution::Overwrite => Some(Overwrite::Always),
        ConflictResolution::OverwriteIfNewer => Some(Overwrite::IfNewer),
        ConflictResolution::OverwriteIfLarger => Some(Overwrite::IfLarger),
        ConflictResolution::OverwriteIfDifferent => Some(Overwrite::IfDifferentSize),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets() {
        let p = PolicyMemory::preset(None, Some(ErrorAction::SkipAll), Overwrite::IfNewer);
        assert_eq!(p.conflict, Some(ConflictResolution::OverwriteIfNewer));
        assert_eq!(p.error, Some(ErrorAction::Skip));
        let p = PolicyMemory::preset(
            Some(ConflictResolution::Rename("x".into())),
            Some(ErrorAction::Retry),
            Overwrite::Always,
        );
        assert_eq!(p.conflict, Some(ConflictResolution::KeepBoth));
        assert_eq!(p.error, None);
        assert_eq!(resolution_to_overwrite(&ConflictResolution::Skip), None);
        assert_eq!(resolution_to_overwrite(&ConflictResolution::OverwriteIfLarger), Some(Overwrite::IfLarger));
    }
}
