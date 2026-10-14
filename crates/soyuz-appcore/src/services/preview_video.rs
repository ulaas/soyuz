//! Video poster frames through the FFmpeg libraries (`soyuz_media::grab_frame`, when
//! `services::media` has loaded them): one keyframe near the poster time, rotated and
//! fitted into `[media] max_px`. Soyuz never spawns an `ffmpeg` program: without the
//! libraries a video shows its container metadata (`preview_media`) and a card that says
//! where to get them. Playback arrives with the player (Phase V, wave 3).

use std::path::Path;

use soyuz_media::{FrameRequest, MediaError, Runtime, Source};

/// Seconds to seek to for a representative frame: 10% in, at most 5 s.
pub fn poster_time(duration: Option<f64>) -> f64 {
    match duration {
        Some(d) if d.is_finite() && d > 0.0 => (d * 0.1).min(5.0),
        _ => 1.0,
    }
}

/// Where the poster frame is taken: `[media] poster_secs` clamped to the duration, or
/// [`poster_time`] when it is 0.
pub fn poster_at(poster_secs: f32, duration: Option<f64>) -> f64 {
    if poster_secs > 0.0 && poster_secs.is_finite() {
        let secs = f64::from(poster_secs);
        match duration {
            Some(d) if d.is_finite() && d > 0.0 => secs.min(d),
            _ => secs,
        }
    } else {
        poster_time(duration)
    }
}

/// One keyframe of `video` near `at` seconds, fitted into `max_px`, through the loaded
/// libraries. `Err("stale")` when the request was abandoned.
pub fn poster(
    rt: &Runtime,
    video: &Path,
    at: f64,
    max_px: u32,
    stale: &dyn Fn() -> bool,
) -> Result<image::RgbaImage, String> {
    let src = Source::File(video.to_path_buf());
    match soyuz_media::grab_frame(rt, &src, FrameRequest::near(at, max_px), stale) {
        Ok(img) => Ok(img),
        Err(MediaError::Stale) => Err("stale".into()),
        // The first keyframe is always worth a try when the seek target has none.
        Err(e) if at > 0.0 => match soyuz_media::grab_frame(rt, &src, FrameRequest::near(0.0, max_px), stale) {
            Ok(img) => Ok(img),
            Err(MediaError::Stale) => Err("stale".into()),
            Err(_) => Err(tr!("services-video-no-frame", error = e.to_string())),
        },
        Err(e) => Err(tr!("services-video-no-frame", error = e.to_string())),
    }
}

/// Parse a "1:02:03" / "4:05" / "12.5 s" duration fact back into seconds.
pub fn parse_duration(s: &str) -> Option<f64> {
    let s = s.trim().trim_start_matches('≈').trim();
    if let Some(v) = s.strip_suffix('s') {
        return v.trim().parse().ok();
    }
    let mut total = 0f64;
    for part in s.split(':') {
        total = total * 60.0 + part.trim().parse::<f64>().ok()?;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poster_time_and_durations() {
        assert_eq!(poster_time(Some(20.0)), 2.0);
        assert_eq!(poster_time(Some(3600.0)), 5.0);
        assert_eq!(poster_time(None), 1.0);
        assert_eq!(parse_duration("1:02:03"), Some(3723.0));
        assert_eq!(parse_duration("4:05"), Some(245.0));
        assert_eq!(parse_duration("≈ 0:10"), Some(10.0));
        assert_eq!(parse_duration("12.5 s"), Some(12.5));
        assert_eq!(parse_duration("n/a"), None);
    }

    #[test]
    fn poster_secs_semantics() {
        // 0 = today's fraction logic.
        assert_eq!(poster_at(0.0, Some(20.0)), 2.0);
        assert_eq!(poster_at(0.0, None), 1.0);
        // A value: that many seconds, clamped to the duration.
        assert_eq!(poster_at(30.0, Some(20.0)), 20.0);
        assert_eq!(poster_at(3.0, Some(20.0)), 3.0);
        assert_eq!(poster_at(3.0, None), 3.0);
        assert_eq!(poster_at(f32::NAN, Some(20.0)), 2.0);
        assert_eq!(poster_at(-1.0, Some(20.0)), 2.0);
    }
}
