//! Video poster frames (roadmap F3) through an optional `ffmpeg` executable. Soyuz never
//! bundles or downloads ffmpeg: when `[preview] video_ffmpeg` is on and the program is
//! found (a full path in `ffmpeg_path`, or the name looked up on `PATH`), one frame is
//! decoded to PNG on stdout. The process runs with no console window, is killed when the
//! request goes stale or takes too long, and its output is size-capped. Playback is out
//! of scope (see docs/USER_GUIDE.md).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Give up on ffmpeg after this long.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Largest PNG accepted from ffmpeg.
const MAX_PNG: u64 = 64 * 1024 * 1024;

/// Resolve the configured ffmpeg program to an existing file.
pub fn find_ffmpeg(configured: &str) -> Option<PathBuf> {
    let c = configured.trim();
    let c = if c.is_empty() { "ffmpeg" } else { c };
    let p = Path::new(c);
    if p.components().count() > 1 || p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    let exts: Vec<String> = if cfg!(windows) {
        let has_ext = p.extension().is_some();
        if has_ext { vec![String::new()] } else { vec![".exe".into(), ".com".into()] }
    } else {
        vec![String::new()]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for e in &exts {
            let cand = dir.join(format!("{c}{e}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Seconds to seek to for a representative frame: 10% in, at most 5 s.
pub fn poster_time(duration: Option<f64>) -> f64 {
    match duration {
        Some(d) if d.is_finite() && d > 0.0 => (d * 0.1).min(5.0),
        _ => 1.0,
    }
}

/// Run ffmpeg for one PNG frame of `video` at `at` seconds, scaled to `max_w` wide.
pub fn poster(ffmpeg: &Path, video: &Path, at: f64, max_w: u32, stale: &dyn Fn() -> bool) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss"])
        .arg(format!("{at:.3}"))
        .arg("-i")
        .arg(video)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("scale='min({max_w},iw)':-2"))
        .args(["-f", "image2pipe", "-vcodec", "png", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up for the child.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Can't start ffmpeg ({e})"))?;
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        return Err("ffmpeg gave no output".into());
    };
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.by_ref().take(MAX_PNG).read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
        if stale() || start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if stale() { "stale".into() } else { "ffmpeg took too long".into() });
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let bytes = reader.join().unwrap_or_default();
    if bytes.starts_with(b"\x89PNG") { Ok(bytes) } else { Err("ffmpeg produced no frame".into()) }
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
    fn missing_ffmpeg_is_none() {
        assert_eq!(find_ffmpeg("definitely-not-a-real-program-soyuz"), None);
        assert_eq!(find_ffmpeg("C:/nope/ffmpeg.exe"), None);
    }
}
