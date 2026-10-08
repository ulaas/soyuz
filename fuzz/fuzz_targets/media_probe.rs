#![no_main]
//! Audio / video metadata for the preview pane (MP4/MOV boxes, WAV, AVI, FLAC, Ogg, MP3
//! frames + ID3). The first byte picks the extension hint.
use libfuzzer_sys::fuzz_target;

const EXTS: &[&str] = &["mp4", "mov", "m4a", "wav", "avi", "flac", "ogg", "opus", "mp3", "mkv", "bin"];

fuzz_target!(|data: &[u8]| {
    let Some((&pick, body)) = data.split_first() else { return };
    let ext = EXTS[pick as usize % EXTS.len()];
    let mut cur = std::io::Cursor::new(body);
    let _ = soyuz_app::services::preview_media::probe(&mut cur, body.len() as u64, ext);
    let _ = soyuz_app::services::preview_media::id3_text(body);
});
