//! LaunchServices (CoreServices) and AppKit's `NSWorkspace` for opening files, the
//! "Open with" list, Finder reveal and alias resolution. No `open` / `osascript`
//! processes are spawned.
//!
//! Untested on macOS: compile-checked from Linux only.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::TCFType;
use core_foundation::bundle::CFBundle;
use core_foundation::data::CFData;
use core_foundation::error::{CFError, CFErrorRef};
use core_foundation::string::CFString;
use core_foundation::url::{
    CFURL, CFURLCreateBookmarkDataFromFile, CFURLCreateByResolvingBookmarkData, CFURLRef,
    kCFURLBookmarkResolutionWithoutMountingMask, kCFURLBookmarkResolutionWithoutUIMask,
};

/// `OSStatus` of a failed LaunchServices call.
pub(crate) type OsStatus = i32;

/// `kLSRolesAll`.
const LS_ROLES_ALL: u32 = 0xFFFF_FFFF;
/// `kLSLaunchDefaults`.
const LS_LAUNCH_DEFAULTS: u32 = 0x0000_0001;
/// `kLSLaunchAsync`: do not wait for the application to finish launching.
const LS_LAUNCH_ASYNC: u32 = 0x0001_0000;

/// `LSLaunchURLSpec`; the header wraps it in `#pragma pack(push, 2)`.
#[repr(C, packed(2))]
struct LsLaunchUrlSpec {
    app_url: CFURLRef,
    item_urls: CFArrayRef,
    pass_thru_params: *const c_void,
    launch_flags: u32,
    async_ref_con: *const c_void,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn LSOpenFromURLSpec(spec: *const LsLaunchUrlSpec, out_launched: *mut CFURLRef) -> OsStatus;
    fn LSCopyApplicationURLsForURL(url: CFURLRef, roles: u32) -> CFArrayRef;
    fn LSCopyDefaultApplicationURLForURL(url: CFURLRef, roles: u32, err: *mut CFErrorRef) -> CFURLRef;
}

/// Open `items` with `app` (an application bundle URL), or each with its default
/// application when `app` is `None`. One LaunchServices call for all items.
pub(crate) fn open_urls(items: &[CFURL], app: Option<&CFURL>) -> Result<(), OsStatus> {
    let arr = CFArray::<CFURL>::from_CFTypes(items);
    let spec = LsLaunchUrlSpec {
        app_url: app.map_or(std::ptr::null(), |a| a.as_concrete_TypeRef()),
        item_urls: if items.is_empty() { std::ptr::null() } else { arr.as_concrete_TypeRef() },
        pass_thru_params: std::ptr::null(),
        launch_flags: LS_LAUNCH_DEFAULTS | LS_LAUNCH_ASYNC,
        async_ref_con: std::ptr::null(),
    };
    // SAFETY: `spec` points to live CF objects (`arr`, `app`) for the duration of the
    // call; a null `out_launched` is allowed.
    let st = unsafe { LSOpenFromURLSpec(&spec, std::ptr::null_mut()) };
    if st == 0 { Ok(()) } else { Err(st) }
}

/// Every application LaunchServices knows for `url` (all roles), as bundle paths.
pub(crate) fn apps_for_url(url: &CFURL) -> Vec<PathBuf> {
    // SAFETY: `url` is a valid CFURL; the array follows the copy rule (`take_array`).
    let items = unsafe { super::cf::take_array(LSCopyApplicationURLsForURL(url.as_concrete_TypeRef(), LS_ROLES_ALL)) };
    items.into_iter().filter_map(|t| t.downcast::<CFURL>()).filter_map(|u| u.to_path()).collect()
}

/// The default application for `url` (all roles), as a bundle path.
pub(crate) fn default_app_for_url(url: &CFURL) -> Option<PathBuf> {
    let mut err: CFErrorRef = std::ptr::null_mut();
    // SAFETY: `url` is valid; the returned URL and the error follow the create rule and
    // are wrapped (released) exactly once.
    unsafe {
        let r = LSCopyDefaultApplicationURLForURL(url.as_concrete_TypeRef(), LS_ROLES_ALL, &mut err);
        if !err.is_null() {
            drop(CFError::wrap_under_create_rule(err));
        }
        if r.is_null() {
            return None;
        }
        CFURL::wrap_under_create_rule(r).to_path()
    }
}

/// Bundle facts for the "Open with" list.
pub(crate) struct BundleInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    /// `Contents/Resources/<CFBundleIconFile>[.icns]`, when that file exists.
    pub icon: Option<PathBuf>,
}

/// `CFBundleIdentifier`, `CFBundleDisplayName` / `CFBundleName` and the icon file of an
/// application bundle (from its `Info.plist`; not localized).
pub(crate) fn bundle_info(app: &Path) -> BundleInfo {
    let mut info = BundleInfo { id: None, name: None, icon: None };
    let Some(bundle) = CFURL::from_path(app, true).and_then(CFBundle::new) else { return info };
    let dict = bundle.info_dictionary();
    let get = |k: &str| {
        dict.find(CFString::new(k))
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    };
    info.id = get("CFBundleIdentifier");
    info.name = get("CFBundleDisplayName").or_else(|| get("CFBundleName"));
    if let Some(icon) = get("CFBundleIconFile") {
        let res = app.join("Contents").join("Resources");
        let mut p = res.join(&icon);
        if p.extension().is_none() {
            p.set_extension("icns");
        }
        info.icon = p.is_file().then_some(p);
    }
    info
}

/// The target of a Finder alias file (bookmark data in the file), resolved without UI and
/// without mounting volumes. `None` when `path` is not an alias or cannot be resolved.
pub(crate) fn resolve_alias(path: &Path) -> Option<PathBuf> {
    let url = CFURL::from_path(path, false)?;
    let mut err: CFErrorRef = std::ptr::null_mut();
    // SAFETY: `url` is valid; the data and the error follow the create rule and are
    // wrapped exactly once.
    let data = unsafe {
        let d = CFURLCreateBookmarkDataFromFile(std::ptr::null(), url.as_concrete_TypeRef(), &mut err);
        if !err.is_null() {
            drop(CFError::wrap_under_create_rule(err));
            err = std::ptr::null_mut();
        }
        if d.is_null() {
            return None;
        }
        CFData::wrap_under_create_rule(d)
    };
    let mut stale: u8 = 0;
    // SAFETY: `data` is valid bookmark data; null base URL / property array are allowed;
    // the result and error follow the create rule and are wrapped exactly once.
    unsafe {
        let r = CFURLCreateByResolvingBookmarkData(
            std::ptr::null(),
            data.as_concrete_TypeRef(),
            kCFURLBookmarkResolutionWithoutUIMask | kCFURLBookmarkResolutionWithoutMountingMask,
            std::ptr::null(),
            std::ptr::null(),
            &mut stale,
            &mut err,
        );
        if !err.is_null() {
            drop(CFError::wrap_under_create_rule(err));
        }
        if r.is_null() {
            return None;
        }
        CFURL::wrap_under_create_rule(r).to_path()
    }
}

/// Select `paths` in Finder (`-[NSWorkspace activateFileViewerSelectingURLs:]`).
pub(crate) fn reveal(paths: &[PathBuf]) -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSArray, NSURL};
    objc2::rc::autoreleasepool(|_| {
        let urls: Vec<_> = paths.iter().filter_map(NSURL::from_file_path).collect();
        if urls.is_empty() {
            return false;
        }
        NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&urls));
        true
    })
}
