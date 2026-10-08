//! Unix mode / ownership and a read-only ACL view.

use std::path::Path;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use soyuz_core::FsResult;

use crate::baseline;

/// `Err(Unsupported)` on Windows (not cfg-gated).
pub fn set_mode(path: &Path, mode: u32) -> FsResult<()> {
    baseline::set_mode(path, mode)
}

/// `Err(Unsupported)` on Windows.
pub fn chown(path: &Path, uid: Option<u32>, gid: Option<u32>) -> FsResult<()> {
    baseline::chown(path, uid, gid)
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct AccessRights: u8 {
        const READ = 1;
        const WRITE = 2;
        const EXECUTE = 4;
        const DELETE = 8;
        const FULL = 16;
    }
}

/// One simplified access-control entry.
#[derive(Clone, Debug)]
pub struct AceLite {
    pub principal: String,
    pub allow: bool,
    pub rights: AccessRights,
    pub inherited: bool,
}

#[derive(Clone, Debug)]
pub struct SecurityInfo {
    pub owner: String,
    pub group: Option<String>,
    pub entries: Vec<AceLite>,
}

/// Read-only ACL view (GetNamedSecurityInfoW / mode+acl on unix). Baseline: `Unsupported`.
pub fn security_info(path: &Path) -> FsResult<SecurityInfo> {
    baseline::security_info(path)
}

/// Effective rights of the current user. Baseline: `Unsupported`.
pub fn effective_access(path: &Path) -> FsResult<AccessRights> {
    baseline::effective_access(path)
}
