#![no_main]
//! Terminal output: escape sequences through the VT parser, titles used for cwd
//! detection.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    soyuz_term::fuzzing::feed_escapes(data);
    let _ = soyuz_term::fuzzing::cwd_from_title(&String::from_utf8_lossy(data));
});
