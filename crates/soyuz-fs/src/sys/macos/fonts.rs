//! CoreText's list of available font files (`CTFontManagerCopyAvailableFontURLs`).
//!
//! Untested on macOS: compile-checked from Linux only.

use std::path::PathBuf;

use core_foundation::array::CFArrayRef;
use core_foundation::url::CFURL;

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontManagerCopyAvailableFontURLs() -> CFArrayRef;
}

/// Every font file CoreText can use (system, `/Library/Fonts`, `~/Library/Fonts`,
/// activated fonts), de-duplicated, in CoreText's order.
pub(crate) fn available_font_files() -> Vec<PathBuf> {
    // SAFETY: no arguments; the array follows the copy rule (`take_array` releases it).
    let items = unsafe { super::cf::take_array(CTFontManagerCopyAvailableFontURLs()) };
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter_map(|t| t.downcast::<CFURL>())
        .filter_map(|u| u.to_path())
        .filter(|p| seen.insert(p.clone()))
        .collect()
}
