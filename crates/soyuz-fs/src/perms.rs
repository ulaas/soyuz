//! Unix mode / ownership and a read-only ACL view.
//!
//! * Windows: `GetNamedSecurityInfoW` + `GetAce` + `LookupAccountSidW` for the DACL view,
//!   `AccessCheck` (impersonation token, `MAXIMUM_ALLOWED`) for the effective rights.
//! * Unix: owner/group/other entries derived from the mode; `faccessat(AT_EACCESS)` for the
//!   effective rights. POSIX ACLs (libacl) are out of scope.

use std::path::Path;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use soyuz_core::{FsError, FsResult};

/// Set the permission bits (`fchmodat`). `Err(Unsupported)` on Windows (not cfg-gated).
pub fn set_mode(path: &Path, mode: u32) -> FsResult<()> {
    imp::set_mode(path, mode)
}

/// Change owner and/or group (`fchownat`). `Err(Unsupported)` on Windows.
pub fn chown(path: &Path, uid: Option<u32>, gid: Option<u32>) -> FsResult<()> {
    imp::chown(path, uid, gid)
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

/// Read-only ACL view (owner, group and simplified entries).
#[track_caller]
pub fn security_info(path: &Path) -> FsResult<SecurityInfo> {
    let _guard = crate::guard::check("security_info");
    imp::security_info(path).map_err(|e| FsError { op: "security_info".into(), ..e })
}

/// Effective rights of the current user on `path`.
#[track_caller]
pub fn effective_access(path: &Path) -> FsResult<AccessRights> {
    let _guard = crate::guard::check("effective_access");
    imp::effective_access(path).map_err(|e| FsError { op: "effective_access".into(), ..e })
}

/// Map a Windows file access mask onto [`AccessRights`].
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn rights_from_mask(mask: u32) -> AccessRights {
    const FILE_READ_DATA: u32 = 0x1;
    const FILE_WRITE_DATA: u32 = 0x2;
    const FILE_APPEND_DATA: u32 = 0x4;
    const FILE_EXECUTE: u32 = 0x20;
    const DELETE: u32 = 0x1_0000;
    const FILE_ALL_ACCESS: u32 = 0x1F_01FF;
    let mut r = AccessRights::empty();
    if mask & FILE_READ_DATA != 0 {
        r |= AccessRights::READ;
    }
    if mask & (FILE_WRITE_DATA | FILE_APPEND_DATA) != 0 {
        r |= AccessRights::WRITE;
    }
    if mask & FILE_EXECUTE != 0 {
        r |= AccessRights::EXECUTE;
    }
    if mask & DELETE != 0 {
        r |= AccessRights::DELETE;
    }
    if mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS {
        r |= AccessRights::FULL;
    }
    r
}

/// Rights encoded in one `rwx` triplet.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn rights_from_triplet(bits: u32) -> AccessRights {
    let mut r = AccessRights::empty();
    if bits & 4 != 0 {
        r |= AccessRights::READ;
    }
    if bits & 2 != 0 {
        r |= AccessRights::WRITE;
    }
    if bits & 1 != 0 {
        r |= AccessRights::EXECUTE;
    }
    r
}

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use soyuz_core::FsResult;

    use super::{AccessRights, AceLite, SecurityInfo, rights_from_mask};
    use crate::err;
    use crate::sys::windows::{fs as wfs, security};

    pub(super) fn set_mode(path: &Path, _mode: u32) -> FsResult<()> {
        Err(err::unsupported("set_mode", path, "Unix permission bits do not exist on Windows; use attributes"))
    }

    pub(super) fn chown(path: &Path, _uid: Option<u32>, _gid: Option<u32>) -> FsResult<()> {
        Err(err::unsupported("chown", path, "changing owners by uid/gid is not supported on Windows"))
    }

    pub(super) fn security_info(path: &Path) -> FsResult<SecurityInfo> {
        let security::Dacl { owner, group, aces } = security::dacl(path).map_err(err::map("security_info", path))?;
        let entries = match aces {
            None => vec![AceLite {
                principal: "Everyone".into(),
                allow: true,
                rights: AccessRights::all(),
                inherited: false,
            }],
            Some(aces) => aces
                .into_iter()
                .map(|a| AceLite {
                    principal: a.principal.to_string(),
                    allow: a.allow,
                    rights: rights_from_mask(security::map_generic(a.mask)),
                    inherited: a.inherited,
                })
                .collect(),
        };
        Ok(SecurityInfo { owner: owner.to_string(), group: group.map(|g| g.to_string()), entries })
    }

    pub(super) fn effective_access(path: &Path) -> FsResult<AccessRights> {
        let mask = security::effective_mask(path).map_err(err::map("effective_access", path))?;
        let mut r = rights_from_mask(mask);
        // The read-only attribute denies writes regardless of the ACL (files only).
        if let Ok(attrs) = wfs::get_attributes(path)
            && attrs & 0x1 != 0
            && attrs & 0x10 == 0
        {
            r.remove(AccessRights::WRITE | AccessRights::FULL);
        }
        Ok(r)
    }
}

#[cfg(unix)]
mod imp {
    use std::path::Path;

    use rustix::fs::{Access, AtFlags, CWD, Mode};
    use soyuz_core::FsResult;

    use super::{AccessRights, AceLite, SecurityInfo, rights_from_triplet};
    use crate::err;
    use crate::sys::unix as us;

    pub(super) fn set_mode(path: &Path, mode: u32) -> FsResult<()> {
        rustix::fs::chmodat(CWD, path, Mode::from_raw_mode((mode & 0o7777) as _), AtFlags::empty())
            .map_err(|e| err::io("set_mode", path, e.into()))
    }

    pub(super) fn chown(path: &Path, uid: Option<u32>, gid: Option<u32>) -> FsResult<()> {
        rustix::fs::chownat(
            CWD,
            path,
            uid.map(rustix::process::Uid::from_raw),
            gid.map(rustix::process::Gid::from_raw),
            AtFlags::empty(),
        )
        .map_err(|e| err::io("chown", path, e.into()))
    }

    pub(super) fn security_info(path: &Path) -> FsResult<SecurityInfo> {
        let st = us::ustat_at(CWD, path.as_os_str(), true).map_err(err::map("security_info", path))?;
        let owner = us::user_name(st.uid).map(|s| s.to_string()).unwrap_or_else(|| st.uid.to_string());
        let group = us::group_name(st.gid).map(|s| s.to_string());
        let entry = |principal: String, bits: u32| AceLite {
            principal,
            allow: true,
            rights: rights_from_triplet(bits),
            inherited: false,
        };
        let entries = vec![
            entry(format!("{owner} (owner)"), (st.mode >> 6) & 7),
            entry(format!("{} (group)", group.clone().unwrap_or_else(|| st.gid.to_string())), (st.mode >> 3) & 7),
            entry("everyone else".into(), st.mode & 7),
        ];
        Ok(SecurityInfo { owner, group, entries })
    }

    pub(super) fn effective_access(path: &Path) -> FsResult<AccessRights> {
        us::ustat_at(CWD, path.as_os_str(), true).map_err(err::map("effective_access", path))?;
        let ok = |a: Access| rustix::fs::accessat(CWD, path, a, AtFlags::EACCESS).is_ok();
        let mut r = AccessRights::empty();
        if ok(Access::READ_OK) {
            r |= AccessRights::READ;
        }
        if ok(Access::WRITE_OK) {
            r |= AccessRights::WRITE;
        }
        if ok(Access::EXEC_OK) {
            r |= AccessRights::EXECUTE;
        }
        // Deleting needs write + search permission on the parent directory.
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        if rustix::fs::accessat(CWD, parent, Access::WRITE_OK | Access::EXEC_OK, AtFlags::EACCESS).is_ok() {
            r |= AccessRights::DELETE;
        }
        if r.contains(AccessRights::READ | AccessRights::WRITE | AccessRights::DELETE) {
            r |= AccessRights::FULL;
        }
        Ok(r)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::path::Path;

    use soyuz_core::{FsError, FsResult};

    use super::{AccessRights, SecurityInfo};

    pub(super) fn set_mode(path: &Path, _mode: u32) -> FsResult<()> {
        Err(FsError::unsupported("set_mode", Some(path)))
    }
    pub(super) fn chown(path: &Path, _u: Option<u32>, _g: Option<u32>) -> FsResult<()> {
        Err(FsError::unsupported("chown", Some(path)))
    }
    pub(super) fn security_info(path: &Path) -> FsResult<SecurityInfo> {
        Err(FsError::unsupported("security_info", Some(path)))
    }
    pub(super) fn effective_access(path: &Path) -> FsResult<AccessRights> {
        Err(FsError::unsupported("effective_access", Some(path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_and_triplets() {
        assert_eq!(rights_from_mask(0x1F01FF), AccessRights::all());
        assert_eq!(rights_from_mask(0x120089), AccessRights::READ);
        assert_eq!(rights_from_triplet(0o7), AccessRights::READ | AccessRights::WRITE | AccessRights::EXECUTE);
        assert_eq!(rights_from_triplet(0o4), AccessRights::READ);
    }

    #[test]
    fn security_view_of_temp_dir() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        let info = security_info(&f).unwrap();
        assert!(!info.owner.is_empty());
        assert!(!info.entries.is_empty());
        let r = effective_access(&f).unwrap();
        assert!(r.contains(AccessRights::READ | AccessRights::WRITE), "{r:?}");
        assert_eq!(effective_access(&d.path().join("missing")).unwrap_err().kind, soyuz_core::FsErrorKind::NotFound);
    }
}
