#![no_main]
//! `$Recycle.Bin\$I…` records (written by any program, read when listing the trash).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| soyuz_fs::fuzzing::recycle_i(data));
