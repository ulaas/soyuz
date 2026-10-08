//! The user's Windows display language (kernel32 `GetUserDefaultUILanguage`).

use windows_sys::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};

/// BCP-47 name of the user's UI language (`"tr-TR"`), `None` if the lookup fails.
pub(crate) fn ui_language() -> Option<String> {
    // SAFETY: no arguments, returns a LANGID by value.
    let langid = unsafe { GetUserDefaultUILanguage() };
    // LOCALE_NAME_MAX_LENGTH = 85 UTF-16 units including the NUL.
    let mut buf = [0u16; 85];
    // SAFETY: `buf` is a valid writable buffer of the length we pass; a LANGID is a
    // valid LCID with the default sort order.
    let n = unsafe { LCIDToLocaleName(u32::from(langid), buf.as_mut_ptr(), buf.len() as i32, 0) };
    if n <= 1 {
        return None;
    }
    let name = super::from_wide_nul(&buf).to_string_lossy().into_owned();
    (!name.is_empty()).then_some(name)
}
