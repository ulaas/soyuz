//! Unix mode / ownership and a read-only ACL view.
//!
//! * Windows: `GetNamedSecurityInfoW` + `GetAce` + `LookupAccountSidW` for the DACL view,
//!   `AccessCheck` (impersonation token, `MAXIMUM_ALLOWED`) for the effective rights.
//! * Unix: owner/group/other entries derived from the mode; `faccessat(AT_EACCESS)` for the
//!   effective rights. On Linux the POSIX ACL xattrs (`system.posix_acl_access`, and
//!   `system.posix_acl_default` on folders) are parsed by hand (no libacl) into the same
//!   view: named users and groups, the mask (applied to their rights) and the default
//!   entries new children inherit. The view is read-only; ACLs are carried over by copies.

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

/// One entry of a Linux POSIX ACL (`struct posix_acl_xattr_entry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PosixAce {
    pub tag: u16,
    pub perm: u16,
    pub id: u32,
}

pub(crate) const ACL_USER_OBJ: u16 = 0x01;
pub(crate) const ACL_USER: u16 = 0x02;
pub(crate) const ACL_GROUP_OBJ: u16 = 0x04;
pub(crate) const ACL_GROUP: u16 = 0x08;
pub(crate) const ACL_MASK: u16 = 0x10;
pub(crate) const ACL_OTHER: u16 = 0x20;
const POSIX_ACL_XATTR_VERSION: u32 = 2;

/// Parse the little-endian `system.posix_acl_*` xattr value: a `u32` version (2) followed
/// by 8-byte entries `(u16 tag, u16 perm, u32 id)`. `None` for anything malformed (the
/// bytes come from the filesystem and are not trusted).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn parse_posix_acl(b: &[u8]) -> Option<Vec<PosixAce>> {
    if b.len() < 4 || !(b.len() - 4).is_multiple_of(8) || b.len() > 4 + 8 * 4096 {
        return None;
    }
    if u32::from_le_bytes(b[..4].try_into().ok()?) != POSIX_ACL_XATTR_VERSION {
        return None;
    }
    let mut out = Vec::with_capacity((b.len() - 4) / 8);
    for c in b[4..].as_chunks::<8>().0 {
        let tag = u16::from_le_bytes([c[0], c[1]]);
        if !matches!(tag, ACL_USER_OBJ | ACL_USER | ACL_GROUP_OBJ | ACL_GROUP | ACL_MASK | ACL_OTHER) {
            return None;
        }
        out.push(PosixAce {
            tag,
            perm: u16::from_le_bytes([c[2], c[3]]) & 7,
            id: u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
        });
    }
    Some(out)
}

/// Encode entries as a `system.posix_acl_*` xattr value (tests and copies).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn encode_posix_acl(entries: &[PosixAce]) -> Vec<u8> {
    let mut out = POSIX_ACL_XATTR_VERSION.to_le_bytes().to_vec();
    for e in entries {
        out.extend_from_slice(&e.tag.to_le_bytes());
        out.extend_from_slice(&e.perm.to_le_bytes());
        out.extend_from_slice(&e.id.to_le_bytes());
    }
    out
}

/// The simplified view of one POSIX ACL. `owner` / `group` are display names; `user` /
/// `group_of` name a uid / gid. Named entries and the owning group are limited by the
/// mask (their effective rights); `default` marks the inheritable entries of a folder.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn acl_entries(
    acl: &[PosixAce],
    owner: &str,
    group: &str,
    default: bool,
    user: &dyn Fn(u32) -> String,
    group_of: &dyn Fn(u32) -> String,
) -> Vec<AceLite> {
    let mask = acl.iter().find(|e| e.tag == ACL_MASK).map(|e| u32::from(e.perm));
    let prefix = if default { "default: " } else { "" };
    let mut out = Vec::new();
    // Canonical order: owner, named users, owning group, named groups, mask, others.
    for tag in [ACL_USER_OBJ, ACL_USER, ACL_GROUP_OBJ, ACL_GROUP, ACL_MASK, ACL_OTHER] {
        for e in acl.iter().filter(|e| e.tag == tag) {
            let bits = u32::from(e.perm);
            let (principal, eff) = match tag {
                ACL_USER_OBJ => (format!("{owner} (owner)"), bits),
                ACL_USER => (format!("{} (user)", user(e.id)), bits & mask.unwrap_or(7)),
                ACL_GROUP_OBJ => (format!("{group} (group)"), bits & mask.unwrap_or(7)),
                ACL_GROUP => (format!("{} (group)", group_of(e.id)), bits & mask.unwrap_or(7)),
                ACL_MASK => ("mask (limit for users and groups)".to_string(), bits),
                _ => ("everyone else".to_string(), bits),
            };
            out.push(AceLite {
                principal: format!("{prefix}{principal}"),
                allow: true,
                rights: rights_from_triplet(eff),
                inherited: false,
            });
        }
    }
    out
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
        #[cfg(target_os = "linux")]
        let entries = {
            let group_name = group.clone().unwrap_or_else(|| st.gid.to_string());
            let user = |id: u32| us::user_name(id).map(|s| s.to_string()).unwrap_or_else(|| id.to_string());
            let grp = |id: u32| us::group_name(id).map(|s| s.to_string()).unwrap_or_else(|| id.to_string());
            let read = |name: &str| xattr::get(path, name).ok().flatten().and_then(|v| super::parse_posix_acl(&v));
            let mut out = match read(crate::xattr::ACL_ACCESS) {
                Some(acl) if !acl.is_empty() => super::acl_entries(&acl, &owner, &group_name, false, &user, &grp),
                _ => entries,
            };
            if st.mode & 0o170000 == 0o040000
                && let Some(def) = read(crate::xattr::ACL_DEFAULT)
            {
                out.extend(super::acl_entries(&def, &owner, &group_name, true, &user, &grp));
            }
            out
        };
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
    fn posix_acl_parsing_and_view() {
        let acl = [
            PosixAce { tag: ACL_USER_OBJ, perm: 7, id: u32::MAX },
            PosixAce { tag: ACL_USER, perm: 7, id: 1234 },
            PosixAce { tag: ACL_GROUP_OBJ, perm: 5, id: u32::MAX },
            PosixAce { tag: ACL_GROUP, perm: 6, id: 99 },
            PosixAce { tag: ACL_MASK, perm: 4, id: u32::MAX },
            PosixAce { tag: ACL_OTHER, perm: 0, id: u32::MAX },
        ];
        let bytes = encode_posix_acl(&acl);
        assert_eq!(bytes.len(), 4 + 6 * 8);
        assert_eq!(parse_posix_acl(&bytes).unwrap(), acl);
        // Malformed: wrong version, truncated entry, unknown tag.
        let mut bad = bytes.clone();
        bad[0] = 1;
        assert!(parse_posix_acl(&bad).is_none());
        assert!(parse_posix_acl(&bytes[..bytes.len() - 3]).is_none());
        let mut bad = bytes.clone();
        bad[4] = 0x40;
        assert!(parse_posix_acl(&bad).is_none());
        assert!(parse_posix_acl(&[]).is_none());

        let user = |id: u32| if id == 1234 { "alice".to_string() } else { id.to_string() };
        let group = |id: u32| format!("g{id}");
        let v = acl_entries(&acl, "me", "staff", false, &user, &group);
        let names: Vec<&str> = v.iter().map(|e| e.principal.as_str()).collect();
        assert_eq!(
            names,
            [
                "me (owner)",
                "alice (user)",
                "staff (group)",
                "g99 (group)",
                "mask (limit for users and groups)",
                "everyone else"
            ]
        );
        // The mask (r--) limits named entries and the owning group, not the owner.
        assert_eq!(v[0].rights, AccessRights::READ | AccessRights::WRITE | AccessRights::EXECUTE);
        assert_eq!(v[1].rights, AccessRights::READ);
        assert_eq!(v[2].rights, AccessRights::READ);
        assert_eq!(v[3].rights, AccessRights::READ);
        assert_eq!(v[5].rights, AccessRights::empty());
        let d = acl_entries(&acl[..1], "me", "staff", true, &user, &group);
        assert_eq!(d[0].principal, "default: me (owner)");
    }

    /// A real ACL set through the xattr shows up in the security view (skipped where the
    /// temp filesystem has no POSIX ACL support).
    #[cfg(target_os = "linux")]
    #[test]
    fn security_view_reads_a_real_acl() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f.txt");
        std::fs::write(&f, b"x").unwrap();
        let me = crate::sys::unix::euid();
        let acl = [
            PosixAce { tag: ACL_USER_OBJ, perm: 6, id: u32::MAX },
            PosixAce { tag: ACL_USER, perm: 6, id: me },
            PosixAce { tag: ACL_GROUP_OBJ, perm: 4, id: u32::MAX },
            PosixAce { tag: ACL_MASK, perm: 6, id: u32::MAX },
            PosixAce { tag: ACL_OTHER, perm: 0, id: u32::MAX },
        ];
        if let Err(e) = xattr::set(&f, crate::xattr::ACL_ACCESS, &encode_posix_acl(&acl)) {
            eprintln!("no POSIX ACLs on the temp filesystem: {e}");
            return;
        }
        let info = security_info(&f).unwrap();
        assert_eq!(info.entries.len(), 5, "{:?}", info.entries);
        assert!(info.entries[1].principal.ends_with("(user)"), "{:?}", info.entries);
        assert_eq!(info.entries[1].rights, AccessRights::READ | AccessRights::WRITE);
        // Folders also list their default (inherited) entries.
        let sub = d.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        xattr::set(&sub, crate::xattr::ACL_DEFAULT, &encode_posix_acl(&acl)).unwrap();
        let info = security_info(&sub).unwrap();
        assert!(info.entries.iter().any(|e| e.principal.starts_with("default: ")), "{:?}", info.entries);
        // Copies carry the ACL with Preserve::ACL alone.
        let o = crate::CopyOptions { preserve: crate::Preserve::ACL, ..crate::CopyOptions::default() };
        let dst = d.path().join("copy.txt");
        crate::copy_file(&f, &dst, &o, &soyuz_core::OpControl::new(), &mut ()).unwrap();
        let got = xattr::get(&dst, crate::xattr::ACL_ACCESS).unwrap().map(|v| parse_posix_acl(&v).unwrap());
        assert_eq!(got.as_deref(), Some(&acl[..]));
        let dsub = d.path().join("sub2");
        std::fs::create_dir(&dsub).unwrap();
        crate::copy_dir_metadata(&sub, &dsub, crate::Preserve::ACL).unwrap();
        assert!(xattr::get(&dsub, crate::xattr::ACL_DEFAULT).unwrap().is_some());
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
