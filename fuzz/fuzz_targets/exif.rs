#![no_main]
//! EXIF in JPEG / TIFF files (batch-rename tokens, properties).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(e) = soyuz_app::rename::exif::parse(data) {
        let _ = e.fields();
    }
});
