//! Undo journal records.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use soyuz_fs::{MetaSnapshot, TrashItem};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoRecord {
    Moved { from: PathBuf, to: PathBuf },
    Copied { dst: PathBuf },
    Renamed { from: PathBuf, to: PathBuf },
    Trashed(TrashItem),
    Created(PathBuf),
    Extracted { dst: PathBuf },
    MetaChanged { path: PathBuf, before: MetaSnapshot },
    Irreversible(PathBuf),
}
