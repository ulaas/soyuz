//! Creating files and directories, atomic writes.

use std::path::Path;

use soyuz_core::FsResult;

use crate::baseline;

pub fn create_dir(path: &Path) -> FsResult<()> {
    baseline::create_dir(path)
}

pub fn create_dir_all(path: &Path) -> FsResult<()> {
    baseline::create_dir_all(path)
}

/// Create an empty file; `exclusive` fails with `AlreadyExists` if it exists.
pub fn create_file(path: &Path, exclusive: bool) -> FsResult<()> {
    baseline::create_file(path, exclusive)
}

/// Temp file + rename in the same directory; used for config / session files.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> FsResult<()> {
    baseline::write_atomic(path, bytes)
}
