//! advapi32 security: user SID, owner names, read-only DACL view, effective access,
//! SDDL security descriptors.

use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::mem::{size_of, zeroed};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AccessCheck, AclSizeInformation,
    DACL_SECURITY_INFORMATION, DuplicateToken, GENERIC_MAPPING, GROUP_SECURITY_INFORMATION, GetAce, GetAclInformation,
    GetFileSecurityW, GetTokenInformation, LookupAccountSidW, MapGenericMask, OWNER_SECURITY_INFORMATION,
    PRIVILEGE_SET, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE, SecurityImpersonation, TOKEN_DUPLICATE, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
};
use windows_sys::Win32::System::SystemServices::MAXIMUM_ALLOWED;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{Handle, from_wide_nul, last_err, wide_path, wide_str};

/// Memory returned by advapi32 that must be released with `LocalFree`.
struct LocalBox(*mut c_void);

impl Drop for LocalBox {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer was allocated by advapi32 with LocalAlloc and is freed once.
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

fn process_token(access: u32) -> io::Result<Handle> {
    let mut h = null_mut();
    // SAFETY: pseudo-handle of the current process; valid out-pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) } == 0 {
        return Err(last_err());
    }
    Handle::new(h)
}

/// Raw bytes of the current user's SID (from `TokenUser`).
fn current_user_sid_bytes() -> io::Result<Vec<u8>> {
    let token = process_token(TOKEN_QUERY)?;
    let mut len = 0u32;
    // SAFETY: size query with a null buffer.
    unsafe {
        GetTokenInformation(token.raw(), TokenUser, null_mut(), 0, &mut len);
    }
    if len == 0 {
        return Err(last_err());
    }
    // u64 storage keeps the TOKEN_USER header suitably aligned.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: buffer of at least `len` bytes.
    if unsafe { GetTokenInformation(token.raw(), TokenUser, buf.as_mut_ptr().cast(), len, &mut len) } == 0 {
        return Err(last_err());
    }
    // SAFETY: the buffer now starts with a TOKEN_USER whose SID pointer points inside it.
    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    let sid_len = sid_length(sid);
    // SAFETY: `sid` points at a valid SID of `sid_len` bytes inside `buf`.
    Ok(unsafe { std::slice::from_raw_parts(sid as *const u8, sid_len) }.to_vec())
}

/// Byte length of a SID: 8 + 4 * SubAuthorityCount.
fn sid_length(sid: PSID) -> usize {
    // SAFETY: a SID starts with Revision (u8) and SubAuthorityCount (u8).
    let count = unsafe { *(sid as *const u8).add(1) } as usize;
    8 + 4 * count
}

/// `S-1-5-21-...` string of a SID.
pub(crate) fn sid_string(sid: PSID) -> io::Result<String> {
    let mut out = null_mut();
    // SAFETY: valid SID; out-pointer receives a LocalAlloc'd string.
    if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 {
        return Err(last_err());
    }
    let guard = LocalBox(out.cast());
    let mut len = 0;
    // SAFETY: NUL-terminated string allocated by ConvertSidToStringSidW.
    while unsafe { *out.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` UTF-16 units are readable.
    let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(out, len) });
    drop(guard);
    Ok(s)
}

/// SID string of the user running this process (cached).
pub(crate) fn current_user_sid() -> io::Result<String> {
    static SID: OnceLock<Result<String, String>> = OnceLock::new();
    SID.get_or_init(|| {
        current_user_sid_bytes().and_then(|mut b| sid_string(b.as_mut_ptr().cast())).map_err(|e| e.to_string())
    })
    .clone()
    .map_err(io::Error::other)
}

static NAMES: OnceLock<Mutex<HashMap<String, Arc<str>>>> = OnceLock::new();

/// `DOMAIN\name` of a SID (cached by SID string; the SID string when unresolvable).
pub(crate) fn sid_name(sid: PSID) -> Arc<str> {
    // A failed conversion must not be cached (every unconvertible SID would share the
    // `""` slot and get the first one's name); resolve it uncached instead.
    let Ok(key) = sid_string(sid) else {
        return lookup_sid(sid).unwrap_or_else(|| "(unknown SID)".to_string()).into();
    };
    let cache = NAMES.get_or_init(Default::default);
    if let Some(n) = cache.lock().get(&key) {
        return n.clone();
    }
    let name: Arc<str> = lookup_sid(sid).unwrap_or_else(|| key.clone()).into();
    cache.lock().insert(key, name.clone());
    name
}

fn lookup_sid(sid: PSID) -> Option<String> {
    let mut name = vec![0u16; 256];
    let mut domain = vec![0u16; 256];
    for _ in 0..2 {
        let (mut nl, mut dl) = (name.len() as u32, domain.len() as u32);
        let mut use_: SID_NAME_USE = 0;
        // SAFETY: valid SID; buffers with the lengths we pass.
        let ok = unsafe {
            LookupAccountSidW(null(), sid, name.as_mut_ptr(), &mut nl, domain.as_mut_ptr(), &mut dl, &mut use_)
        };
        if ok != 0 {
            let n = from_wide_nul(&name).to_string_lossy().into_owned();
            let d = from_wide_nul(&domain).to_string_lossy().into_owned();
            return Some(if d.is_empty() { n } else { format!("{d}\\{n}") });
        }
        if last_err().raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return None;
        }
        name.resize(nl as usize + 1, 0);
        domain.resize(dl as usize + 1, 0);
    }
    None
}

/// Security descriptor of a file plus pointers into it.
struct NamedSd {
    _sd: LocalBox,
    owner: PSID,
    group: PSID,
    dacl: *mut ACL,
}

fn named_security(path: &Path, what: u32) -> io::Result<NamedSd> {
    let call = |w: &[u16]| -> (u32, NamedSd) {
        let (mut owner, mut group, mut dacl, mut sd) = (null_mut(), null_mut(), null_mut(), null_mut());
        // SAFETY: NUL-terminated path; out-pointers are valid; the descriptor is freed by
        // the LocalBox guard.
        let rc = unsafe {
            GetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT,
                what,
                &mut owner,
                &mut group,
                &mut dacl,
                null_mut(),
                &mut sd,
            )
        };
        (rc, NamedSd { _sd: LocalBox(sd), owner, group, dacl })
    };
    let (rc, sd) = call(&wide_path(path));
    if rc == 0 {
        return Ok(sd);
    }
    // Some redirectors reject verbatim paths; retry with the plain form.
    let plain = crate::path::strip_verbatim(&crate::path::normalize(path));
    let (rc2, sd2) = call(&wide_str(plain.as_os_str()));
    if rc2 == 0 { Ok(sd2) } else { Err(io::Error::from_raw_os_error(rc as i32)) }
}

/// Owner (and primary group) names of `path`.
pub(crate) fn owner_group(path: &Path) -> io::Result<(Arc<str>, Option<Arc<str>>)> {
    let sd = named_security(path, OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION)?;
    if sd.owner.is_null() {
        return Err(io::Error::other("no owner in security descriptor"));
    }
    let group = (!sd.group.is_null()).then(|| sid_name(sd.group));
    Ok((sid_name(sd.owner), group))
}

/// One simplified ACE: `(principal, allow, access mask, inherited)`.
pub(crate) struct RawAce {
    pub principal: Arc<str>,
    pub allow: bool,
    pub mask: u32,
    pub inherited: bool,
}

/// Owner, group and the allow/deny ACEs of a file's DACL.
pub(crate) struct Dacl {
    pub owner: Arc<str>,
    pub group: Option<Arc<str>>,
    /// `None` = NULL DACL (everyone has full access).
    pub aces: Option<Vec<RawAce>>,
}

pub(crate) fn dacl(path: &Path) -> io::Result<Dacl> {
    let sd = named_security(path, OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION)?;
    let owner = if sd.owner.is_null() { Arc::from("") } else { sid_name(sd.owner) };
    let group = (!sd.group.is_null()).then(|| sid_name(sd.group));
    if sd.dacl.is_null() {
        return Ok(Dacl { owner, group, aces: None });
    }
    // SAFETY: plain-old-data struct.
    let mut info: ACL_SIZE_INFORMATION = unsafe { zeroed() };
    // SAFETY: valid ACL pointer from GetNamedSecurityInfoW; out-buffer of the size we pass.
    let ok = unsafe {
        GetAclInformation(sd.dacl, (&raw mut info).cast(), size_of::<ACL_SIZE_INFORMATION>() as u32, AclSizeInformation)
    };
    if ok == 0 {
        return Err(last_err());
    }
    let mut out = Vec::new();
    for i in 0..info.AceCount {
        let mut ace: *mut c_void = null_mut();
        // SAFETY: index < AceCount of a valid ACL.
        if unsafe { GetAce(sd.dacl, i, &mut ace) } == 0 {
            continue;
        }
        // SAFETY: every ACE starts with an ACE_HEADER.
        let header = unsafe { std::ptr::read_unaligned(ace as *const ACE_HEADER) };
        let allow = match header.AceType {
            0 => true,  // ACCESS_ALLOWED_ACE_TYPE
            1 => false, // ACCESS_DENIED_ACE_TYPE
            _ => continue,
        };
        // SAFETY: allowed/denied ACEs share the ACCESS_ALLOWED_ACE layout; the SID starts at
        // `SidStart` inside the ACE.
        let (mask, sid) = unsafe {
            let a = ace as *const ACCESS_ALLOWED_ACE;
            (std::ptr::read_unaligned(&raw const (*a).Mask), (&raw const (*a).SidStart) as PSID)
        };
        out.push(RawAce { principal: sid_name(sid), allow, mask, inherited: header.AceFlags & 0x10 != 0 });
    }
    Ok(Dacl { owner, group, aces: Some(out) })
}

const FILE_MAPPING: GENERIC_MAPPING = GENERIC_MAPPING {
    GenericRead: FILE_GENERIC_READ,
    GenericWrite: FILE_GENERIC_WRITE,
    GenericExecute: FILE_GENERIC_EXECUTE,
    GenericAll: FILE_ALL_ACCESS,
};

/// Map generic bits (`GENERIC_READ` ...) of an ACE mask onto file-specific rights.
pub(crate) fn map_generic(mut mask: u32) -> u32 {
    if mask & (GENERIC_ALL | GENERIC_READ | GENERIC_WRITE | GENERIC_EXECUTE) != 0 {
        // SAFETY: valid in/out mask and mapping table.
        unsafe { MapGenericMask(&mut mask, &FILE_MAPPING) };
    }
    mask
}

/// Access mask granted to the current user on `path` (`AccessCheck` with an impersonation
/// copy of the process token and `MAXIMUM_ALLOWED`).
pub(crate) fn effective_mask(path: &Path) -> io::Result<u32> {
    let w = wide_path(path);
    let what = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut needed = 0u32;
    // SAFETY: size query with a null buffer.
    unsafe {
        GetFileSecurityW(w.as_ptr(), what, null_mut(), 0, &mut needed);
    }
    if needed == 0 {
        return Err(last_err());
    }
    let mut sd = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: buffer of at least `needed` bytes.
    if unsafe { GetFileSecurityW(w.as_ptr(), what, sd.as_mut_ptr().cast(), needed, &mut needed) } == 0 {
        return Err(last_err());
    }
    let token = process_token(TOKEN_QUERY | TOKEN_DUPLICATE)?;
    let mut imp = null_mut();
    // SAFETY: valid primary token; out-pointer.
    if unsafe { DuplicateToken(token.raw(), SecurityImpersonation, &mut imp) } == 0 {
        return Err(last_err());
    }
    let imp = Handle::new(imp)?;
    // SAFETY: plain-old-data struct.
    let mut privs: PRIVILEGE_SET = unsafe { zeroed() };
    let mut privs_len = size_of::<PRIVILEGE_SET>() as u32;
    let (mut granted, mut status) = (0u32, 0);
    // SAFETY: self-relative descriptor in `sd`; impersonation token; valid out-pointers.
    let ok = unsafe {
        AccessCheck(
            sd.as_mut_ptr() as PSECURITY_DESCRIPTOR,
            imp.raw(),
            MAXIMUM_ALLOWED,
            &FILE_MAPPING,
            &mut privs,
            &mut privs_len,
            &mut granted,
            &mut status,
        )
    };
    if ok == 0 {
        return Err(last_err());
    }
    Ok(if status != 0 { granted } else { 0 })
}

/// A security descriptor built from SDDL (freed on drop).
pub(crate) struct Sddl(LocalBox);

impl Sddl {
    pub(crate) fn new(sddl: &str) -> io::Result<Sddl> {
        let w = wide_str(sddl);
        let mut sd = null_mut();
        // SAFETY: NUL-terminated SDDL string; out-pointer receives a LocalAlloc'd descriptor.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut())
        };
        if ok == 0 {
            return Err(last_err());
        }
        Ok(Sddl(LocalBox(sd)))
    }

    pub(crate) fn ptr(&self) -> *mut c_void {
        self.0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_sid_and_owner() {
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-"), "{sid}");
        let d = tempfile::tempdir().unwrap();
        let (owner, _group) = owner_group(d.path()).unwrap();
        assert!(!owner.is_empty());
        let mask = effective_mask(d.path()).unwrap();
        assert_ne!(mask & 1, 0, "temp dir readable: {mask:#x}");
        let acl = dacl(d.path()).unwrap();
        assert!(acl.aces.is_some_and(|a| !a.is_empty()));
        assert_eq!(map_generic(GENERIC_READ) & FILE_GENERIC_READ, FILE_GENERIC_READ);
        let sd = Sddl::new(&format!("D:PAI(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)")).unwrap();
        assert!(!sd.ptr().is_null());
    }
}
