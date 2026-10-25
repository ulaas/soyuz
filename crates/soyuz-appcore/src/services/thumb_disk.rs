//! On-disk thumbnail cache: decoded thumbnails are written as small PNGs to
//! `<config dir>/thumbs/<hash>.png`, keyed by kind + path + size + mtime + pixel bucket,
//! so they survive restarts and a changed file never shows a stale picture. The folder
//! is kept under `[appearance] thumb_disk_cache_mib`: a hit refreshes the file's mtime and
//! eviction removes the least recently used files first.
//!
//! The OS cache (Explorer's `thumbcache_*.db`, the freedesktop `~/.cache/thumbnails`)
//! is read separately, read-only, by `thumbs::os_cached` (`soyuz_fs::thumbcache`); hits
//! from it are copied in here.
//!
//! Every function here does file I/O and must only run on worker threads.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Bump to invalidate every cached file (format or rendering changes).
const FORMAT_VERSION: u32 = 1;
/// Evict down to this share of the cap.
const LOW_WATER: f64 = 0.85;

static CAP: AtomicU64 = AtomicU64::new(256 * 1024 * 1024);
/// Bytes on disk; `u64::MAX` until the first scan.
static USED: AtomicU64 = AtomicU64::new(u64::MAX);
static DIR: OnceLock<PathBuf> = OnceLock::new();

fn dir() -> &'static Path {
    DIR.get_or_init(|| soyuz_core::config::config_dir().join("thumbs"))
}

/// `[appearance] thumb_disk_cache_mib` (0 disables the cache).
pub fn set_cap_bytes(bytes: u64) {
    CAP.store(bytes, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    CAP.load(Ordering::Relaxed) > 0
}

/// What a cached thumbnail shows (part of the key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Image,
    Video,
    Folder,
    /// First page of a PDF (pdfium).
    Pdf,
    /// The icon in a program's resources (`soyuz_fs::exe_icon`).
    ProgramIcon,
}

/// Stable 64-bit FNV-1a (the std hasher is not stable across Rust releases).
fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// The cache key of a thumbnail. Paths compare case-insensitively on Windows.
pub fn cache_key(kind: Kind, path: &Path, size: u64, mtime: Option<SystemTime>, px: u32) -> u64 {
    cache_key_with_source(kind, path, size, mtime, px, "")
}

/// [`cache_key`] with a `source` tag of what made the picture (the FFmpeg build for
/// video frames, `services::media::source_tag`), so a picture from one decoder is never
/// served for another; the empty tag is [`cache_key`].
pub fn cache_key_with_source(
    kind: Kind,
    path: &Path,
    size: u64,
    mtime: Option<SystemTime>,
    px: u32,
    source: &str,
) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325;
    h = fnv(&FORMAT_VERSION.to_le_bytes(), h);
    h = fnv(&[kind as u8], h);
    if !source.is_empty() {
        h = fnv(source.as_bytes(), h);
        h = fnv(&[0], h);
    }
    let p = soyuz_fs::strip_verbatim(path).to_string_lossy().into_owned();
    let p = if cfg!(windows) { p.to_lowercase().replace('/', "\\") } else { p };
    h = fnv(p.as_bytes(), h);
    h = fnv(&size.to_le_bytes(), h);
    let (secs, nanos) = mtime
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or((u64::MAX, 0), |d| (d.as_secs(), d.subsec_nanos()));
    h = fnv(&secs.to_le_bytes(), h);
    h = fnv(&nanos.to_le_bytes(), h);
    fnv(&px.to_le_bytes(), h)
}

fn file_of(key: u64) -> PathBuf {
    dir().join(format!("{key:016x}.png"))
}

/// A cached thumbnail; refreshes its mtime (LRU order).
pub fn get(key: u64) -> Option<image::RgbaImage> {
    if !enabled() {
        return None;
    }
    let path = file_of(key);
    let bytes = std::fs::read(&path).ok()?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
    if let Ok(f) = std::fs::File::options().write(true).open(&path) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some(img)
}

/// Store a thumbnail, evicting old ones when over the cap.
pub fn put(key: u64, img: &image::RgbaImage) {
    if !enabled() {
        return;
    }
    let (w, h) = img.dimensions();
    let mut png = Vec::new();
    let enc = image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    );
    if image::ImageEncoder::write_image(enc, img.as_raw(), w, h, image::ExtendedColorType::Rgba8).is_err() {
        return;
    }
    let _ = std::fs::create_dir_all(dir());
    let path = file_of(key);
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, &png).is_err() || std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if USED.load(Ordering::Relaxed) == u64::MAX {
        USED.store(scan().iter().map(|e| e.1).sum(), Ordering::Relaxed);
    } else {
        USED.fetch_add(png.len() as u64, Ordering::Relaxed);
    }
    let cap = CAP.load(Ordering::Relaxed);
    if USED.load(Ordering::Relaxed) > cap {
        let entries = scan();
        let doomed = plan_eviction(&entries, cap);
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        for (name, size) in doomed {
            if std::fs::remove_file(dir().join(&name)).is_ok() {
                total = total.saturating_sub(size);
            }
        }
        USED.store(total, Ordering::Relaxed);
    }
}

/// `(file name, bytes, mtime)` of every cached file.
fn scan() -> Vec<(String, u64, SystemTime)> {
    let Ok(rd) = std::fs::read_dir(dir()) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".png") {
                return None;
            }
            let m = e.metadata().ok()?;
            Some((name, m.len(), m.modified().unwrap_or(UNIX_EPOCH)))
        })
        .collect()
}

/// Which files to delete (oldest first) so the total drops to [`LOW_WATER`] × `cap`.
pub fn plan_eviction(entries: &[(String, u64, SystemTime)], cap: u64) -> Vec<(String, u64)> {
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    if total <= cap {
        return Vec::new();
    }
    let target = (cap as f64 * LOW_WATER) as u64;
    let mut by_age: Vec<&(String, u64, SystemTime)> = entries.iter().collect();
    by_age.sort_by_key(|e| e.2);
    let mut out = Vec::new();
    for (name, size, _) in by_age {
        if total <= target {
            break;
        }
        total = total.saturating_sub(*size);
        out.push((name.clone(), *size));
    }
    out
}

/// Delete every cached thumbnail (Settings → Clear), on a worker thread.
pub fn clear() {
    let _ = std::thread::Builder::new().name("soyuz-thumbs-clear".into()).spawn(|| {
        for (name, _, _) in scan() {
            let _ = std::fs::remove_file(dir().join(name));
        }
        USED.store(0, Ordering::Relaxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn keys_change_with_every_input() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let base = cache_key(Kind::Image, Path::new("/a/b.png"), 10, Some(t), 128);
        assert_eq!(base, cache_key(Kind::Image, Path::new("/a/b.png"), 10, Some(t), 128), "deterministic");
        assert_ne!(base, cache_key(Kind::Video, Path::new("/a/b.png"), 10, Some(t), 128));
        let video = cache_key(Kind::Video, Path::new("/a/b.mp4"), 10, Some(t), 128);
        assert_eq!(video, cache_key_with_source(Kind::Video, Path::new("/a/b.mp4"), 10, Some(t), 128, ""));
        let lib = cache_key_with_source(Kind::Video, Path::new("/a/b.mp4"), 10, Some(t), 128, "ffmpeg-63");
        assert_ne!(video, lib, "a frame from the libraries never serves an older cached one");
        assert_ne!(lib, cache_key_with_source(Kind::Video, Path::new("/a/b.mp4"), 10, Some(t), 128, "ffmpeg-62"));
        assert_ne!(base, cache_key(Kind::Image, Path::new("/a/c.png"), 10, Some(t), 128));
        assert_ne!(base, cache_key(Kind::Image, Path::new("/a/b.png"), 11, Some(t), 128));
        assert_ne!(base, cache_key(Kind::Image, Path::new("/a/b.png"), 10, Some(t + Duration::from_micros(1)), 128));
        assert_ne!(base, cache_key(Kind::Image, Path::new("/a/b.png"), 10, None, 128));
        assert_ne!(base, cache_key(Kind::Image, Path::new("/a/b.png"), 10, Some(t), 256));
        // Known value: the key must stay stable across releases (cached files are reused).
        assert_eq!(fnv(b"a", 0xCBF2_9CE4_8422_2325), 0xAF63_DC4C_8601_EC8C);
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_ignore_case_and_prefix() {
        let k = |p: &str| cache_key(Kind::Image, Path::new(p), 1, None, 64);
        assert_eq!(k(r"C:\Pics\A.png"), k(r"c:\pics\a.PNG"));
        assert_eq!(k(r"\\?\C:\Pics\A.png"), k(r"C:\Pics\A.png"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_paths_are_case_sensitive() {
        let k = |p: &str| cache_key(Kind::Image, Path::new(p), 1, None, 64);
        assert_ne!(k("/pics/A.png"), k("/pics/a.png"));
        assert_eq!(k("/pics/A.png"), k("/pics/A.png"));
    }

    #[test]
    fn eviction_removes_oldest_down_to_low_water() {
        let t = |s: u64| UNIX_EPOCH + Duration::from_secs(s);
        let entries: Vec<(String, u64, SystemTime)> =
            (0..10).map(|i| (format!("{i}.png"), 100, t(1000 - i * 10))).collect();
        assert!(plan_eviction(&entries, 1000).is_empty(), "at the cap: nothing to do");
        let doomed = plan_eviction(&entries, 900);
        // 1000 bytes → ≤ 765: the three oldest (9, 8, 7) go.
        let names: Vec<&str> = doomed.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["9.png", "8.png", "7.png"]);
        assert_eq!(plan_eviction(&entries, 0).len(), 10);
    }
}
