//! CoreFoundation helpers: URLs, strings and the global preferences domain
//! (`CFPreferencesCopyAppValue(key, kCFPreferencesAnyApplication)`, i.e. NSGlobalDomain).
//!
//! Untested on macOS: compile-checked from Linux only.

use std::ffi::c_void;
use std::path::Path;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, TCFType};
use core_foundation::number::CFNumber;
use core_foundation::propertylist::{CFPropertyList, CFPropertyListRef};
use core_foundation::string::{CFString, CFStringRef, kCFStringEncodingUTF8};
use core_foundation::url::{CFURL, CFURLCreateWithBytes};

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFPreferencesAnyApplication: CFStringRef;
    fn CFPreferencesCopyAppValue(key: CFStringRef, application_id: CFStringRef) -> CFPropertyListRef;
    fn CFPreferencesAppSynchronize(application_id: CFStringRef) -> u8;
}

/// A `file://` URL for a local path (`None` for an empty path or interior NUL).
pub(crate) fn file_url(path: &Path) -> Option<CFURL> {
    if path.as_os_str().is_empty() {
        return None;
    }
    let is_dir = std::fs::metadata(path).is_ok_and(|m| m.is_dir());
    CFURL::from_path(path, is_dir)
}

/// A URL parsed from a string (`https://…`, `mailto:…`, `file:///…`); `None` when
/// CoreFoundation rejects it.
pub(crate) fn string_url(s: &str) -> Option<CFURL> {
    let len = isize::try_from(s.len()).ok()?;
    // SAFETY: the bytes are valid for `len` bytes for the duration of the call; the
    // returned URL follows the create rule and is wrapped exactly once.
    unsafe {
        let r = CFURLCreateWithBytes(std::ptr::null(), s.as_ptr(), len, kCFStringEncodingUTF8, std::ptr::null());
        (!r.is_null()).then(|| CFURL::wrap_under_create_rule(r))
    }
}

/// Take ownership of a `CFArray` returned under the create rule and collect its elements
/// as `CFType`s (each retained).
///
/// # Safety
/// `arr` must be null or a valid `CFArrayRef` the caller owns (create / copy rule).
pub(crate) unsafe fn take_array(arr: CFArrayRef) -> Vec<CFType> {
    if arr.is_null() {
        return Vec::new();
    }
    // SAFETY: the caller owns `arr` (create rule); wrapping releases it on drop.
    let arr: CFArray<*const c_void> = unsafe { CFArray::wrap_under_create_rule(arr) };
    arr.get_all_values()
        .into_iter()
        .filter(|p| !p.is_null())
        // SAFETY: every element of a CFArray is a valid CF object kept alive by the array
        // while we retain it (get rule).
        .map(|p| unsafe { CFType::wrap_under_get_rule(p) })
        .collect()
}

/// A value of the global preferences domain. `sync` first re-reads the domain from
/// `cfprefsd` (so changes made by System Settings while we run are seen).
pub(crate) fn global_pref(key: &str, sync: bool) -> Option<CFPropertyList> {
    let k = CFString::new(key);
    // SAFETY: `kCFPreferencesAnyApplication` is an immutable framework constant; `k` is a
    // valid CFString for the call; the result follows the copy rule and is wrapped once.
    unsafe {
        if sync {
            CFPreferencesAppSynchronize(kCFPreferencesAnyApplication);
        }
        let v = CFPreferencesCopyAppValue(k.as_concrete_TypeRef(), kCFPreferencesAnyApplication);
        (!v.is_null()).then(|| CFPropertyList::wrap_under_create_rule(v))
    }
}

/// A string value of the global domain.
pub(crate) fn global_string(key: &str, sync: bool) -> Option<String> {
    global_pref(key, sync)?.downcast_into::<CFString>().map(|s| s.to_string())
}

/// An integer value of the global domain (stored as a number or as a numeric string).
pub(crate) fn global_int(key: &str, sync: bool) -> Option<i64> {
    let v = global_pref(key, sync)?;
    if let Some(n) = v.downcast::<CFNumber>() {
        return n.to_i64();
    }
    v.downcast_into::<CFString>()?.to_string().trim().parse().ok()
}

/// The strings of an array value of the global domain (non-strings are skipped).
pub(crate) fn global_strings(key: &str, sync: bool) -> Vec<String> {
    let Some(arr) = global_pref(key, sync).and_then(|v| v.downcast_into::<CFArray>()) else {
        return Vec::new();
    };
    arr.get_all_values()
        .into_iter()
        .filter(|p| !p.is_null())
        // SAFETY: elements of a CFArray are valid CF objects while the array is alive; the
        // get rule retains them for the duration of the wrapper.
        .map(|p| unsafe { CFType::wrap_under_get_rule(p) })
        .filter_map(|t| t.downcast::<CFString>().map(|s| s.to_string()))
        .collect()
}
