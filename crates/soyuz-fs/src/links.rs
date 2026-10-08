//! Symbolic links, hard links and junctions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soyuz_core::FsResult;

use crate::baseline;

/// Windows needs to know whether a symlink points at a file or a directory.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    File,
    Dir,
}

pub fn symlink(target: &Path, link: &Path, kind: LinkKind) -> FsResult<()> {
    baseline::symlink(target, link, kind)
}

pub fn hardlink(src: &Path, dst: &Path) -> FsResult<()> {
    baseline::hardlink(src, dst)
}

/// NTFS junction. `Err(Unsupported)` on non-Windows (not cfg-gated, so callers compile
/// everywhere). Baseline: `Unsupported` everywhere.
pub fn junction(target: &Path, link: &Path) -> FsResult<()> {
    baseline::junction(target, link)
}

/// The raw link target (may be relative).
pub fn read_link(path: &Path) -> FsResult<PathBuf> {
    baseline::read_link(path)
}

/// The link target made absolute (relative targets resolved against the link's parent),
/// normalized but not canonicalized.
pub fn resolve_link(path: &Path) -> FsResult<PathBuf> {
    baseline::resolve_link(path)
}
