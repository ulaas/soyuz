//! File copy.

use std::path::Path;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use soyuz_core::{DeviceId, FsResult, HashAlgo, OpControl, Overwrite, ProgressSink};

use crate::baseline;

bitflags! {
    /// Metadata to carry over when copying.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct Preserve: u8 {
        const TIMES = 1;
        const ATTRS = 2;
        const XATTR = 4;
        const ACL = 8;
        const OWNER = 16;
        const SPARSE = 32;
    }
}

/// Copy-on-write cloning preference.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Reflink {
    #[default]
    Auto,
    Never,
    Always,
}

/// Options for [`copy_file`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CopyOptions {
    pub overwrite: Overwrite,
    pub preserve: Preserve,
    pub reflink: Reflink,
    /// Files at least this large bypass the page cache (default 256 MiB).
    pub unbuffered_threshold: u64,
    /// Chunk size for progress / checkpoints (default 8 MiB).
    pub chunk: usize,
    pub verify: Option<HashAlgo>,
    pub fsync: bool,
    pub follow_symlinks: bool,
    pub merge_dirs: bool,
    /// Test hook: sleep to cap throughput (honoured by baseline and raw implementations).
    pub throttle_bps: Option<u64>,
}

impl Default for CopyOptions {
    fn default() -> Self {
        CopyOptions {
            overwrite: Overwrite::Never,
            preserve: Preserve::TIMES | Preserve::ATTRS | Preserve::XATTR,
            reflink: Reflink::Auto,
            unbuffered_threshold: 256 * 1024 * 1024,
            chunk: 8 * 1024 * 1024,
            verify: None,
            fsync: false,
            follow_symlinks: false,
            merge_dirs: true,
            throttle_bps: None,
        }
    }
}

/// Why a copy was skipped by the overwrite policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Exists,
    Older,
    Smaller,
    SameSize,
    SameFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyOutcome {
    Copied { bytes: u64, method: CopyMethod },
    Skipped(SkipReason),
}

/// How the bytes were copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMethod {
    Reflink,
    CopyFileEx,
    BlockClone,
    CopyFileRange,
    ReadWrite,
    Clonefile,
    Fcopyfile,
}

/// Progress is reported at least every `chunk` bytes; `ctl.checkpoint()` is honoured
/// between chunks (and inside the CopyFileExW callback on Windows). On cancel the partial
/// destination is removed and `Err(Interrupted)` returned.
///
/// Baseline: `std::fs::copy` + one final progress call (no mid-file cancel), or a chunked
/// read/write loop with progress + cancel when `throttle_bps` is set.
pub fn copy_file(
    src: &Path,
    dst: &Path,
    opts: &CopyOptions,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> FsResult<CopyOutcome> {
    baseline::copy_file(src, dst, opts, ctl, sink)
}

/// Recreate the symlink `src` at `dst` (never follows).
pub fn copy_symlink(src: &Path, dst: &Path, opts: &CopyOptions) -> FsResult<()> {
    baseline::copy_symlink(src, dst, opts)
}

/// Copy directory metadata (times / attributes) after its contents were copied.
pub fn copy_dir_metadata(src: &Path, dst: &Path, preserve: Preserve) -> FsResult<()> {
    baseline::copy_dir_metadata(src, dst, preserve)
}

/// dev+inode / FileIdInfo equality.
pub fn same_file(a: &Path, b: &Path) -> FsResult<bool> {
    baseline::same_file(a, b)
}

/// Device (volume) of `path`.
pub fn device_of(path: &Path) -> FsResult<DeviceId> {
    baseline::device_of(path)
}
