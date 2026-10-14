//! Video thumbnails: one keyframe through the FFmpeg libraries (`soyuz_media::grab_frame`,
//! once `services::media` has loaded them), and the duration from our own container
//! parser (`preview_media`) for the label, which is all a video gets without the
//! libraries. Every decode registers the file in the crash sentinel first
//! (`media::sentinel`); a file whose decode crashed the last run is skipped.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use soyuz_media::{FrameRequest, Source};

use super::media;
use super::preview_video::poster_at;

/// Video extensions we try to thumbnail (the FFmpeg-only containers once the libraries
/// are loaded; `file_class::VIDEO_EXT`).
pub fn is_video(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return false };
    crate::file_class::is_video_ext(&ext.to_ascii_lowercase(), media::runtime().is_some())
}

/// `[media] poster_secs` (read by the workers).
static POSTER_SECS: AtomicU32 = AtomicU32::new(0);

/// Where thumbnails and posters are taken from (`[media] poster_secs`; 0 = a tenth of the
/// duration, at most 5 s).
pub fn set_poster_secs(secs: f32) {
    POSTER_SECS.store(secs.max(0.0).to_bits(), Ordering::Relaxed);
}

pub fn poster_secs() -> f32 {
    f32::from_bits(POSTER_SECS.load(Ordering::Relaxed))
}

/// One keyframe of `path` near the poster time (falling back to the very first frame),
/// fitted into `px`. `None` without the libraries, when the file is skipped after a crash
/// or when nothing decodes.
pub fn frame(path: &Path, px: u32) -> Option<image::RgbaImage> {
    let rt = media::runtime()?;
    let _inflight = media::sentinel().begin(path)?;
    let src = Source::File(path.to_path_buf());
    let at = poster_at(poster_secs(), duration_secs(path));
    let mut times = vec![at];
    if at > 0.0 {
        times.push(0.0);
    }
    times.into_iter().find_map(|t| match soyuz_media::grab_frame(&rt, &src, FrameRequest::near(t, px), &|| false) {
        Ok(img) => Some(img),
        Err(e) => {
            log::debug!("no thumbnail frame of {} at {t:.2} s: {e}", path.display());
            None
        }
    })
}

/// The duration in seconds from the container headers (pure Rust, no decoding).
pub fn duration_secs(path: &Path) -> Option<f64> {
    duration_label(path).and_then(|l| super::preview_video::parse_duration(&l))
}

/// `3:25`-style duration of a video, from the container headers.
pub fn duration_label(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let fields = super::preview_media::probe(&mut f, len, &ext);
    fields.into_iter().find(|(k, _)| k == "Duration").map(|(_, v)| v.trim_start_matches("≈ ").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_extensions() {
        assert!(is_video(Path::new("a/clip.MP4")));
        assert!(is_video(Path::new("x.mkv")));
        assert!(is_video(Path::new("x.flv")));
        assert!(!is_video(Path::new("x.mp3")));
        assert!(!is_video(Path::new("mp4")));
        assert!(!is_video(Path::new("app.ts")));
    }

    #[test]
    fn poster_secs_round_trip() {
        set_poster_secs(2.5);
        assert_eq!(poster_secs(), 2.5);
        set_poster_secs(-1.0);
        assert_eq!(poster_secs(), 0.0);
    }

    #[test]
    fn no_frame_without_the_libraries_or_for_junk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v.mp4");
        std::fs::write(&p, b"\0\0\0\x18ftypisom\0\0\0\0isomiso2").unwrap();
        // Either no runtime (None at once) or a runtime that cannot decode 24 junk bytes.
        assert!(frame(&p, 64).is_none());
        assert_eq!(duration_secs(&p), None);
    }
}
