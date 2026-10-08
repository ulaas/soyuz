//! Volumes, free space and the injectable [`FsProbe`].

use std::path::{Path, PathBuf};

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use soyuz_core::{DeviceId, FsResult};

use crate::baseline;
use crate::trash::TrashSupport;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeKind {
    Fixed,
    Removable,
    Network,
    Optical,
    Ram,
    Unknown,
}

bitflags! {
    /// Filesystem capabilities.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct FsCaps: u16 {
        const SPARSE = 1;
        const REFLINK = 2;
        const HARDLINK = 4;
        const SYMLINK = 8;
        const ADS = 16;
        const CASE_SENSITIVE = 32;
        const COMPRESSION = 64;
        const TRASH = 128;
    }
}

#[derive(Clone, Debug)]
pub struct VolumeInfo {
    pub mount: PathBuf,
    pub label: String,
    /// Filesystem name (`"NTFS"`, `"ext4"`, ...); empty when unknown.
    pub fs: String,
    pub kind: VolumeKind,
    /// Bytes; 0 when unknown (baseline).
    pub total: u64,
    pub free: u64,
    pub read_only: bool,
    pub device: DeviceId,
    pub supports: FsCaps,
}

/// Mounted volumes (drives on Windows). Baseline: drive letters / `/` + home, no sizes.
pub fn volumes() -> Vec<VolumeInfo> {
    baseline::volumes()
}

/// `(total, free)` bytes of the volume containing `path`. Baseline: `Unsupported`.
pub fn space(path: &Path) -> FsResult<(u64, u64)> {
    baseline::space(path)
}

/// Mount point / drive root containing `path`.
pub fn volume_root(path: &Path) -> FsResult<PathBuf> {
    baseline::volume_root(path)
}

/// Injectable probe so jobs can fake free space / device ids in tests.
pub trait FsProbe: Send + Sync {
    fn space(&self, p: &Path) -> FsResult<(u64, u64)>;
    fn device_of(&self, p: &Path) -> FsResult<DeviceId>;
    fn trash_support(&self, p: &Path) -> TrashSupport;
}

/// The real filesystem.
#[derive(Clone, Copy, Debug, Default)]
pub struct RealProbe;

impl FsProbe for RealProbe {
    fn space(&self, p: &Path) -> FsResult<(u64, u64)> {
        space(p)
    }

    fn device_of(&self, p: &Path) -> FsResult<DeviceId> {
        crate::copy::device_of(p)
    }

    fn trash_support(&self, p: &Path) -> TrashSupport {
        crate::trash::trash_support(p)
    }
}
