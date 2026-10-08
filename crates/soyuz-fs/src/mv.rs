//! Rename / move.

use std::path::Path;

use serde::{Deserialize, Serialize};
use soyuz_core::FsResult;

use crate::baseline;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RenameMode {
    /// Fail with `AlreadyExists` if the destination exists.
    #[default]
    NoReplace,
    Replace,
    /// Atomically swap (Linux `RENAME_EXCHANGE`, macOS `RENAME_SWAP`); `Unsupported` elsewhere.
    Exchange,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MoveOptions {
    pub mode: RenameMode,
    /// Test hook: always report `NeedsCopy`.
    pub force_cross_device: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveFast {
    Renamed,
    NeedsCopy,
}

/// Same-device rename; `CrossDevice` error otherwise.
pub fn rename(src: &Path, dst: &Path, mode: RenameMode) -> FsResult<()> {
    baseline::rename(src, dst, mode)
}

/// Case-only rename that is safe on case-insensitive volumes (two-step on FAT).
pub fn rename_case_only(src: &Path, new_name: &str) -> FsResult<()> {
    baseline::rename_case_only(src, new_name)
}

/// Try a rename; `Ok(NeedsCopy)` when the move crosses devices (caller copies + deletes).
pub fn try_rename_or_crossdev(src: &Path, dst: &Path, o: &MoveOptions) -> FsResult<MoveFast> {
    baseline::try_rename_or_crossdev(src, dst, o)
}
