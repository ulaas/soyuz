//! Video thumbnails (roadmap B3): the first frame through an `ffmpeg` binary found on
//! `PATH` (spawned off-thread with a timeout, no console window), and the duration from
//! our own container parser (`preview_media`) for the placeholder when ffmpeg is
//! missing or fails.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use egui::ColorImage;

/// Longest a single ffmpeg run may take.
const TIMEOUT: Duration = Duration::from_secs(8);
/// Largest PNG accepted from ffmpeg.
const MAX_OUTPUT: usize = 16 * 1024 * 1024;

static FFMPEG: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Video extensions we try to thumbnail.
pub fn is_video(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return false };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "flv" | "mpg" | "mpeg" | "3gp" | "m2ts"
    )
}

/// Search `PATH` for ffmpeg on a worker thread (call once at startup).
pub fn probe_in_background() {
    if FFMPEG.get().is_some() {
        return;
    }
    let _ = std::thread::Builder::new().name("soyuz-ffmpeg-probe".into()).spawn(|| {
        let _ = FFMPEG.set(find_on_path(std::env::var_os("PATH").as_deref(), ffmpeg_names()));
    });
}

/// The ffmpeg binary, once [`probe_in_background`] has found it (`None` before / when
/// missing).
pub fn ffmpeg_path() -> Option<PathBuf> {
    FFMPEG.get().cloned().flatten()
}

fn ffmpeg_names() -> &'static [&'static str] {
    if cfg!(windows) { &["ffmpeg.exe"] } else { &["ffmpeg"] }
}

/// First `names` entry found in a `PATH`-style list.
pub fn find_on_path(path: Option<&std::ffi::OsStr>, names: &[&str]) -> Option<PathBuf> {
    let path = path?;
    std::env::split_paths(path)
        .filter(|d| !d.as_os_str().is_empty())
        .find_map(|d| names.iter().map(|n| d.join(n)).find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file())))
}

/// The ffmpeg arguments for one frame at `seek` seconds, scaled to fit `px`.
pub fn ffmpeg_args(input: &Path, seek: f32, px: u32) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = ["-hide_banner", "-loglevel", "error", "-nostdin"].map(Into::into).into();
    if seek > 0.0 {
        a.push("-ss".into());
        a.push(format!("{seek:.2}").into());
    }
    a.push("-i".into());
    a.push(input.as_os_str().to_owned());
    let filter = format!("scale='min({px},iw)':'min({px},ih)':force_original_aspect_ratio=decrease");
    for s in ["-frames:v", "1", "-an", "-sn", "-vf", &filter, "-f", "image2pipe", "-vcodec", "png", "-"] {
        a.push(s.into());
    }
    a
}

/// One frame of `path` (at ~1 s, falling back to the very first frame), fitted to `px`.
pub fn frame(path: &Path, px: u32) -> Option<ColorImage> {
    let ff = ffmpeg_path()?;
    [1.0, 0.0].into_iter().find_map(|seek| run(&ff, &ffmpeg_args(path, seek, px)))
}

fn run(ff: &Path, args: &[std::ffi::OsString]) -> Option<ColorImage> {
    let mut cmd = Command::new(ff);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut out).take(MAX_OUTPUT as u64).read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let bytes = reader.join().ok()?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
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
        assert!(!is_video(Path::new("x.mp3")));
        assert!(!is_video(Path::new("mp4")));
    }

    #[test]
    fn path_search() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("bin");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("ffmpeg.exe"), b"").unwrap();
        let path = std::env::join_paths([dir.path().to_path_buf(), sub.clone()]).unwrap();
        assert_eq!(find_on_path(Some(&path), &["ffmpeg.exe"]), Some(sub.join("ffmpeg.exe")));
        assert_eq!(find_on_path(Some(&path), &["nope.exe"]), None);
        assert_eq!(find_on_path(None, &["ffmpeg.exe"]), None);
    }

    #[test]
    fn args_seek_and_scale() {
        let a = ffmpeg_args(Path::new("v.mp4"), 1.0, 256);
        let s: Vec<String> = a.iter().map(|x| x.to_string_lossy().into_owned()).collect();
        let i = s.iter().position(|x| x == "-i").unwrap();
        assert_eq!(s[i + 1], "v.mp4");
        assert!(s[..i].contains(&"-ss".to_string()), "input seeking (fast) comes before -i");
        assert!(s.iter().any(|x| x.contains("min(256,iw)")));
        assert_eq!(s.last().map(String::as_str), Some("-"));
        let b = ffmpeg_args(Path::new("v.mp4"), 0.0, 64);
        assert!(!b.iter().any(|x| x == "-ss"));
    }
}
