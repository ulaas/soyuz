#![no_main]
//! `.desktop` launchers, `Exec=` lines, shared-mime-info `globs2` and `mimeapps.list`.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        soyuz_fs::fuzzing::desktop(text);
    }
});
