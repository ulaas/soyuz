//! Metadata snapshots.

use std::path::Path;

use serde::{Deserialize, Serialize};
use soyuz_core::{Attributes, FsResult};

use crate::baseline;
use crate::times::TimesSet;

/// Snapshot used by jobs' undo of `SetMeta`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetaSnapshot {
    pub times: TimesSet,
    pub attrs: Attributes,
    pub mode: Option<u32>,
}

/// Capture times / attributes / mode of `path` (symlinks not followed).
pub fn meta_snapshot(path: &Path) -> FsResult<MetaSnapshot> {
    baseline::meta_snapshot(path)
}
