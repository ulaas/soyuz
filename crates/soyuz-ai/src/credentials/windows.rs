//! Windows Credential Manager through raw advapi32 calls (generic credentials, persisted
//! per user on this machine). The only `unsafe` in `soyuz-ai`.

use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
};

use crate::error::{AiError, AiResult};

/// Largest blob `CredWriteW` accepts (`CRED_MAX_CREDENTIAL_BLOB_SIZE`, 5 * 512 bytes).
const MAX_BLOB: usize = 5 * 512;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error(op: &str) -> AiError {
    // SAFETY: GetLastError has no preconditions.
    let code = unsafe { GetLastError() };
    AiError::Credential(format!("{op} failed (Win32 error {code})"))
}

pub fn store(target: &str, secret: &str) -> AiResult<()> {
    let mut blob = secret.as_bytes().to_vec();
    if blob.len() > MAX_BLOB {
        return Err(AiError::Credential(format!("secret too long ({} bytes, max {MAX_BLOB})", blob.len())));
    }
    let mut target_w = wide(target);
    let mut user_w = wide("soyuz");
    let mut comment_w = wide("Soyuz AI provider key");
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target_w.as_mut_ptr(),
        Comment: comment_w.as_mut_ptr(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: user_w.as_mut_ptr(),
        ..CREDENTIALW::default()
    };
    // SAFETY: every pointer in `cred` points into a live, NUL-terminated (strings) or
    // correctly sized (blob) buffer owned by this frame; CredWriteW copies the data.
    let ok = unsafe { CredWriteW(&cred, 0) };
    blob.fill(0);
    if ok == 0 { Err(last_error("CredWriteW")) } else { Ok(()) }
}

pub fn load(target: &str) -> AiResult<Option<String>> {
    let target_w = wide(target);
    let mut out: *mut CREDENTIALW = std::ptr::null_mut();
    // SAFETY: `target_w` is NUL-terminated and outlives the call; `out` receives a buffer
    // allocated by the system that we release with CredFree below.
    let ok = unsafe { CredReadW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0, &mut out) };
    if ok == 0 {
        // SAFETY: no preconditions.
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            return Ok(None);
        }
        return Err(AiError::Credential(format!("CredReadW failed (Win32 error {code})")));
    }
    // SAFETY: CredReadW succeeded, so `out` points to a valid CREDENTIALW whose blob
    // pointer / size describe readable memory until CredFree.
    let bytes = unsafe {
        let c = &*out;
        if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
        }
    };
    // SAFETY: `out` was allocated by CredReadW and is freed exactly once.
    unsafe { CredFree(out.cast()) };
    String::from_utf8(bytes).map(Some).map_err(|_| AiError::Credential("stored secret is not UTF-8".into()))
}

pub fn delete(target: &str) -> AiResult<()> {
    let target_w = wide(target);
    // SAFETY: `target_w` is NUL-terminated and outlives the call.
    let ok = unsafe { CredDeleteW(target_w.as_ptr(), CRED_TYPE_GENERIC, 0) };
    if ok == 0 {
        // SAFETY: no preconditions.
        let code = unsafe { GetLastError() };
        if code != ERROR_NOT_FOUND {
            return Err(AiError::Credential(format!("CredDeleteW failed (Win32 error {code})")));
        }
    }
    Ok(())
}
