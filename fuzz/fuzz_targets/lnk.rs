#![no_main]
//! Windows shell link (`.lnk`) files found in folders and on the desktop.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| soyuz_fs::fuzzing::lnk(data));
