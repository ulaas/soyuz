//! Metadata snapshots.

use std::path::Path;

use serde::{Deserialize, Serialize};
use soyuz_core::{Attributes, FsError, FsResult};

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
    let e = crate::list::stat(path, false).map_err(|e| FsError { op: "meta_snapshot".into(), ..e })?;
    Ok(MetaSnapshot {
        times: TimesSet { modified: e.modified, created: e.created, accessed: e.accessed },
        attrs: e.attrs,
        mode: e.mode,
    })
}
