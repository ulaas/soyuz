#![no_main]
//! freedesktop `.trashinfo` files and their percent-encoded paths.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        soyuz_fs::fuzzing::trashinfo(text);
    }
});
