#![no_main]
//! Key / mouse / focus encoding for every key, modifier and terminal mode; bracketed paste
//! and URL detection on arbitrary text.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    soyuz_term::fuzzing::encode_input(data);
    soyuz_term::fuzzing::paste_and_urls(&String::from_utf8_lossy(data));
});
