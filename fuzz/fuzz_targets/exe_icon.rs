#![no_main]
//! Program icons read from the resources of `.exe` / `.scr` files found in folders.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| soyuz_fs::fuzzing::exe_icon(data));
