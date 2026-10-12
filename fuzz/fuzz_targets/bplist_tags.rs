#![no_main]
//! macOS Finder tags (`com.apple.metadata:_kMDItemUserTags`, a binary plist).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(tags) = soyuz_appcore::services::tags::parse_finder_tags(data) {
        let _ = soyuz_appcore::services::tags::finder_mask(&tags);
    }
});
