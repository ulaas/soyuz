//! Registry access (advapi32): reads for file associations, and HKCU-only writes for the
//! opt-in "default file manager" registration (`crate::integration`).

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_CREATED_NEW_KEY,
    REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_ANY, RRF_RT_REG_DWORD, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW, RegGetValueW,
    RegLoadMUIStringW, RegOpenKeyExW, RegQueryInfoKeyW, RegSetValueExW,
};

use super::{from_wide_nul, wide_str};

/// Registry hive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Hive {
    ClassesRoot,
    CurrentUser,
    LocalMachine,
}

fn hkey(h: Hive) -> HKEY {
    match h {
        Hive::ClassesRoot => HKEY_CLASSES_ROOT,
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
    }
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: key opened by RegOpenKeyExW, closed once.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

fn open(h: Hive, subkey: &str) -> Option<Key> {
    let w = wide_str(subkey);
    let mut out = null_mut();
    // SAFETY: predefined root key, NUL-terminated subkey, valid out-pointer.
    let rc = unsafe { RegOpenKeyExW(hkey(h), w.as_ptr(), 0, KEY_READ, &mut out) };
    (rc == 0).then_some(Key(out))
}

/// String value (`REG_SZ` / `REG_EXPAND_SZ`, environment variables expanded); `None` for the
/// default value when `value` is `None`.
pub(crate) fn get_string(h: Hive, subkey: &str, value: Option<&str>) -> Option<String> {
    let k = wide_str(subkey);
    let v = value.map(wide_str);
    let vptr = v.as_ref().map_or(null(), |v| v.as_ptr());
    let mut buf = vec![0u16; 260];
    loop {
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: predefined root, NUL-terminated names, output buffer of `bytes` bytes.
        let rc = unsafe {
            RegGetValueW(
                hkey(h),
                k.as_ptr(),
                vptr,
                RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ,
                null_mut(),
                buf.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if rc == ERROR_MORE_DATA && buf.len() < 1 << 20 {
            buf.resize((bytes as usize).div_ceil(2) + 1, 0);
            continue;
        }
        if rc != 0 {
            return None;
        }
        return Some(from_wide_nul(&buf).to_string_lossy().into_owned());
    }
}

/// `REG_DWORD` value.
pub(crate) fn get_dword(h: Hive, subkey: &str, value: &str) -> Option<u32> {
    let k = wide_str(subkey);
    let v = wide_str(value);
    let mut out = 0u32;
    let mut bytes = 4u32;
    // SAFETY: predefined root, NUL-terminated names, a 4-byte output buffer.
    let rc = unsafe {
        RegGetValueW(
            hkey(h),
            k.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut out as *mut u32).cast(),
            &mut bytes,
        )
    };
    (rc == 0).then_some(out)
}

/// Whether a value of any type exists.
pub(crate) fn has_value(h: Hive, subkey: &str, value: &str) -> bool {
    let k = wide_str(subkey);
    let v = wide_str(value);
    // SAFETY: predefined root, NUL-terminated names; size-only query (null data).
    let rc = unsafe { RegGetValueW(hkey(h), k.as_ptr(), v.as_ptr(), RRF_RT_ANY, null_mut(), null_mut(), null_mut()) };
    rc == 0
}

/// Names of the direct subkeys.
pub(crate) fn subkeys(h: Hive, subkey: &str) -> Vec<String> {
    let Some(key) = open(h, subkey) else { return Vec::new() };
    let mut out = Vec::new();
    let mut name = [0u16; 256];
    for i in 0.. {
        let mut len = name.len() as u32;
        // SAFETY: open key; output buffer of `len` characters.
        let rc =
            unsafe { RegEnumKeyExW(key.0, i, name.as_mut_ptr(), &mut len, null(), null_mut(), null_mut(), null_mut()) };
        if rc != 0 {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    out
}

/// Names of the values of a key (the default value is skipped).
pub(crate) fn value_names(h: Hive, subkey: &str) -> Vec<String> {
    let Some(key) = open(h, subkey) else { return Vec::new() };
    let mut out = Vec::new();
    let mut name = vec![0u16; 16_384];
    for i in 0.. {
        let mut len = name.len() as u32;
        // SAFETY: open key; output name buffer of `len` characters; type/data not requested.
        let rc =
            unsafe { RegEnumValueW(key.0, i, name.as_mut_ptr(), &mut len, null(), null_mut(), null_mut(), null_mut()) };
        if rc != 0 {
            break;
        }
        if len > 0 {
            out.push(String::from_utf16_lossy(&name[..len as usize]));
        }
    }
    out
}

/// A value resolved through `RegLoadMUIStringW` (handles `@dll,-id` indirect strings).
pub(crate) fn mui_string(h: Hive, subkey: &str, value: &str) -> Option<String> {
    let key = open(h, subkey)?;
    let v = wide_str(value);
    let mut buf = vec![0u16; 512];
    let mut needed = 0u32;
    // SAFETY: open key, NUL-terminated value name, output buffer of the byte size we pass.
    let rc = unsafe {
        RegLoadMUIStringW(key.0, v.as_ptr(), buf.as_mut_ptr(), (buf.len() * 2) as u32, &mut needed, 0, null())
    };
    if rc != 0 {
        return None;
    }
    let s = from_wide_nul(&buf).to_string_lossy().into_owned();
    (!s.is_empty()).then_some(s)
}

// ---- HKCU writes ---------------------------------------------------------------------
//
// Writes are limited to HKEY_CURRENT_USER on purpose: the only writer is the per-user,
// opt-in default-file-manager registration, which must never need admin rights.

fn win_err(rc: u32) -> std::io::Error {
    std::io::Error::from_raw_os_error(rc as i32)
}

/// Whether `HKCU\subkey` exists.
pub(crate) fn hkcu_key_exists(subkey: &str) -> bool {
    open(Hive::CurrentUser, subkey).is_some()
}

/// Create (or open) `HKCU\subkey`; returns whether it was newly created.
pub(crate) fn hkcu_create_key(subkey: &str) -> std::io::Result<bool> {
    let w = wide_str(subkey);
    let mut out = null_mut();
    let mut disposition = 0u32;
    // SAFETY: predefined root, NUL-terminated subkey, valid out-pointers; no class, default security.
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            null(),
            &mut out,
            &mut disposition,
        )
    };
    if rc != 0 {
        return Err(win_err(rc));
    }
    drop(Key(out));
    Ok(disposition == REG_CREATED_NEW_KEY)
}

/// Set a `REG_SZ` value (`None` = the key's default value) under `HKCU\subkey`, creating the key.
pub(crate) fn hkcu_set_string(subkey: &str, value: Option<&str>, data: &str) -> std::io::Result<()> {
    hkcu_create_key(subkey)?;
    let k = wide_str(subkey);
    let mut key = null_mut();
    // SAFETY: predefined root, NUL-terminated subkey, valid out-pointer.
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, KEY_WRITE, &mut key) };
    if rc != 0 {
        return Err(win_err(rc));
    }
    let key = Key(key);
    let name = value.map(wide_str);
    let data = wide_str(data);
    // SAFETY: open key, NUL-terminated (or null = default) name, data buffer of the byte length passed.
    let rc = unsafe {
        RegSetValueExW(
            key.0,
            name.as_ref().map_or(null(), |n| n.as_ptr()),
            0,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    if rc != 0 { Err(win_err(rc)) } else { Ok(()) }
}

/// Delete a value (`None` = default value) under `HKCU\subkey`. Missing key/value is not an error.
pub(crate) fn hkcu_delete_value(subkey: &str, value: Option<&str>) -> std::io::Result<()> {
    let k = wide_str(subkey);
    let mut key = null_mut();
    // SAFETY: predefined root, NUL-terminated subkey, valid out-pointer.
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, KEY_WRITE, &mut key) };
    if rc == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    if rc != 0 {
        return Err(win_err(rc));
    }
    let key = Key(key);
    let name = value.map(wide_str);
    // SAFETY: open key, NUL-terminated (or null = default) value name.
    let rc = unsafe { RegDeleteValueW(key.0, name.as_ref().map_or(null(), |n| n.as_ptr())) };
    if rc == 0 || rc == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(win_err(rc)) }
}

/// Delete `HKCU\subkey` with all its subkeys and values. Missing key is not an error.
pub(crate) fn hkcu_delete_tree(subkey: &str) -> std::io::Result<()> {
    if subkey.trim_matches('\\').is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "refusing to delete the HKCU root"));
    }
    let k = wide_str(subkey);
    // SAFETY: predefined root, NUL-terminated non-empty subkey.
    let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    if rc != 0 && rc != windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND {
        return Err(win_err(rc));
    }
    // RegDeleteTreeW empties the key; remove the key itself too.
    // SAFETY: as above.
    let rc = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, k.as_ptr()) };
    if rc == 0 || rc == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(win_err(rc)) }
}

/// Delete `HKCU\subkey` only when it has no subkeys and no values (incl. the default value).
/// Returns whether it was deleted.
pub(crate) fn hkcu_delete_key_if_empty(subkey: &str) -> std::io::Result<bool> {
    let Some(key) = open(Hive::CurrentUser, subkey) else { return Ok(false) };
    let (mut nsub, mut nval) = (0u32, 0u32);
    // SAFETY: open key; only the subkey and value counts are requested.
    let rc = unsafe {
        RegQueryInfoKeyW(
            key.0,
            null_mut(),
            null_mut(),
            null(),
            &mut nsub,
            null_mut(),
            null_mut(),
            &mut nval,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
        )
    };
    drop(key);
    if rc != 0 {
        return Err(win_err(rc));
    }
    if nsub > 0 || nval > 0 {
        return Ok(false);
    }
    let k = wide_str(subkey);
    // SAFETY: predefined root, NUL-terminated subkey.
    let rc = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, k.as_ptr()) };
    if rc != 0 { Err(win_err(rc)) } else { Ok(true) }
}
