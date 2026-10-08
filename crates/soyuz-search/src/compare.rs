//! Folder compare (M12) — blocking, run on a worker thread.

use std::path::{Path, PathBuf};

use soyuz_core::{FsResult, OpControl};

/// All paths are relative to the compared roots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompareResult {
    pub only_left: Vec<PathBuf>,
    pub only_right: Vec<PathBuf>,
    pub newer_left: Vec<PathBuf>,
    pub newer_right: Vec<PathBuf>,
    pub differ: Vec<PathBuf>,
    pub same: u64,
}

pub fn compare_dirs(a: &Path, b: &Path, by_content: bool, ctl: &OpControl) -> FsResult<CompareResult> {
    let _ = (a, b, by_content, ctl);
    todo!("M12: soyuz_search::compare_dirs")
}
