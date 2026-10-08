//! Read-only registry access (advapi32) for file associations.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_ANY, RRF_RT_REG_DWORD,
    RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegGetValueW, RegLoadMUIStringW,
    RegOpenKeyExW,
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

/// Every `REG_SZ` / `REG_EXPAND_SZ` value of a key as `(name, data)` in one enumeration
/// (unexpanded; the default value and other types are skipped).
pub(crate) fn string_values(h: Hive, subkey: &str) -> Vec<(String, String)> {
    use windows_sys::Win32::System::Registry::{REG_EXPAND_SZ, REG_SZ};
    let Some(key) = open(h, subkey) else { return Vec::new() };
    let mut out = Vec::new();
    let mut name = vec![0u16; 16_384];
    let mut data = vec![0u16; 1024];
    let mut i = 0u32;
    loop {
        let mut name_len = name.len() as u32;
        let mut data_bytes = (data.len() * 2) as u32;
        let mut ty = 0u32;
        // SAFETY: open key; name buffer of `name_len` characters, data buffer of
        // `data_bytes` bytes, valid type out-pointer.
        let rc = unsafe {
            RegEnumValueW(
                key.0,
                i,
                name.as_mut_ptr(),
                &mut name_len,
                null(),
                &mut ty,
                data.as_mut_ptr().cast(),
                &mut data_bytes,
            )
        };
        if rc == ERROR_MORE_DATA && data.len() < 1 << 16 {
            data.resize((data_bytes as usize).div_ceil(2) + 1, 0);
            continue;
        }
        if rc != 0 {
            break;
        }
        i += 1;
        if name_len > 0 && (ty == REG_SZ || ty == REG_EXPAND_SZ) {
            let chars = (data_bytes as usize / 2).min(data.len());
            let value = from_wide_nul(&data[..chars]).to_string_lossy().into_owned();
            out.push((String::from_utf16_lossy(&name[..name_len as usize]), value));
        }
    }
    out
}
