//! Authenticode verification of a file (`WinVerifyTrust`, wintrust.dll). Used to check
//! downloaded installers before offering to run them. No UI, no revocation network calls.

use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{TRUST_E_NOSIGNATURE, TRUST_E_PROVIDER_UNKNOWN, TRUST_E_SUBJECT_FORM_UNKNOWN};
use windows_sys::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE,
    WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
};

use super::wide_str;

/// Raw `WinVerifyTrust` outcome: `0` = valid and trusted, `Err(None)` = not signed,
/// `Err(Some(hresult))` = signed but invalid or untrusted.
pub(crate) fn verify(path: &Path) -> Result<(), Option<i32>> {
    let w = wide_str(path.as_os_str());
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: w.as_ptr(),
        hFile: null_mut(),
        pgKnownSubject: null_mut(),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` and `file` are fully initialised, live across both calls, and the
    // path buffer is NUL-terminated; a null HWND means no UI.
    let rc = unsafe { WinVerifyTrust(null_mut(), &mut action, (&mut data as *mut WINTRUST_DATA).cast()) };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: releases the state data allocated by the verify call above.
    unsafe {
        WinVerifyTrust(null_mut(), &mut action, (&mut data as *mut WINTRUST_DATA).cast());
    }
    match rc {
        0 => Ok(()),
        TRUST_E_NOSIGNATURE | TRUST_E_SUBJECT_FORM_UNKNOWN | TRUST_E_PROVIDER_UNKNOWN => Err(None),
        e => Err(Some(e)),
    }
}
