#![no_main]
//! Archive format detection and the zip / tar.* / 7z readers (headers, entry lists, the
//! first bytes of each entry). The first byte picks the extension hint.
use libfuzzer_sys::fuzz_target;

const EXTS: &[&str] = &["zip", "7z", "tar", "tar.gz", "tgz", "tar.bz2", "tar.xz", "tar.zst", "gz", "bin"];

fuzz_target!(|data: &[u8]| {
    let Some((&pick, body)) = data.split_first() else { return };
    soyuz_archive::fuzzing::tar_header(body);
    soyuz_archive::fuzzing::detect_and_list(body, EXTS[pick as usize % EXTS.len()]);
});
