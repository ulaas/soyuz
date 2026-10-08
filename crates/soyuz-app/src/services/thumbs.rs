//! Thumbnail pipeline (PLAN §3): worker threads decode + downscale images and upload the
//! texture themselves (`egui::Context` is `Send + Sync`), so the UI thread only looks
//! textures up. Requests are served newest-first and dropped once their rows have been
//! scrolled away for a while, so fast scrolling never builds a backlog. An LRU keeps the
//! decoded textures within a byte budget (`[preview] thumb_cache_mib`, see
//! [`set_budget_bytes`]); [`memory`] reports the current use for the About dialog.
//!
//! Besides pictures, workers make video thumbnails (ffmpeg, `thumb_video`) with a
//! duration label, PDF first pages (pdfium, when the library is found; otherwise PDFs
//! keep their icon) and folder mosaics (`thumb_folder`); every decoded result also goes
//! through the on-disk cache (`thumb_disk`) so it survives restarts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::SystemTime;

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use parking_lot::{Condvar, Mutex};

/// Default texture budget (bytes) until the config is applied.
pub const DEFAULT_BUDGET: u64 = 128 * 1024 * 1024;
/// Textures used within this many frames are never evicted (a budget smaller than one
/// screen of thumbnails must not thrash).
const KEEP_FRAMES: u64 = 2;

static BUDGET: AtomicU64 = AtomicU64::new(DEFAULT_BUDGET);
static USED_BYTES: AtomicU64 = AtomicU64::new(0);
static READY: AtomicU64 = AtomicU64::new(0);

/// Set the texture memory budget (`[preview] thumb_cache_mib`).
pub fn set_budget_bytes(bytes: u64) {
    BUDGET.store(bytes.max(1024 * 1024), Ordering::Relaxed);
}

/// `(bytes, textures)` of decoded thumbnails currently cached.
pub fn memory() -> (u64, u64) {
    (USED_BYTES.load(Ordering::Relaxed), READY.load(Ordering::Relaxed))
}
/// Requests not re-requested within this many frames are dropped.
const STALE_FRAMES: u64 = 20;
/// Files larger than this are not thumbnailed.
const MAX_FILE_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub path: PathBuf,
    pub mtime: Option<SystemTime>,
    pub size: u64,
    /// Size bucket (longest edge in pixels).
    pub px: u32,
}

use super::thumb_disk::Kind;

#[derive(Clone)]
enum Slot {
    Pending {
        wanted: u64,
    },
    Ready {
        tex: TextureHandle,
        used: u64,
        bytes: u64,
        label: Option<String>,
    },
    /// No picture; videos may still have a duration label for the placeholder.
    Failed {
        label: Option<String>,
    },
}

/// `[appearance] video_thumbnails` (read by the workers).
static VIDEO_FRAMES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turn ffmpeg frame extraction on / off (`[appearance] video_thumbnails`).
pub fn set_video_frames(on: bool) {
    VIDEO_FRAMES.store(on, Ordering::Relaxed);
}

/// `[appearance] pdf_thumbnails` and `[preview] pdfium_dir` (read by the workers).
static PDF_PAGES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static PDFIUM_DIR: Mutex<String> = Mutex::new(String::new());

/// PDF first-page thumbnails on / off and where pdfium is looked for.
pub fn set_pdf(on: bool, pdfium_dir: &str) {
    PDF_PAGES.store(on && cfg!(feature = "pdf"), Ordering::Relaxed);
    let mut d = PDFIUM_DIR.lock();
    if *d != pdfium_dir {
        *d = pdfium_dir.to_string();
    }
}

/// `[appearance] os_thumbnail_cache` (read by the workers).
static OS_CACHE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Use Explorer's thumbnail cache (`thumbcache_*.db`) first (Windows).
pub fn set_os_cache(on: bool) {
    OS_CACHE.store(on && cfg!(windows), Ordering::Relaxed);
}

/// A PDF that gets a first-page thumbnail (when pdfium turns out to be available).
pub fn is_pdf_thumbnailable(path: &Path) -> bool {
    PDF_PAGES.load(Ordering::Relaxed) && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

struct Shared {
    queue: Mutex<Vec<(ThumbKey, Kind)>>,
    cv: Condvar,
    slots: Mutex<HashMap<ThumbKey, Slot>>,
    frame: AtomicU64,
}

pub struct Thumbs {
    shared: Arc<Shared>,
    pub enabled: bool,
}

/// Extensions we can decode.
pub fn is_thumbnailable(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return false };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff"
    )
}

/// Pixel bucket for a requested on-screen size (shared across nearby zoom levels).
pub fn bucket(points: f32, pixels_per_point: f32) -> u32 {
    let px = (points * pixels_per_point).ceil() as u32;
    match px {
        0..=64 => 64,
        65..=128 => 128,
        129..=256 => 256,
        _ => 512,
    }
}

impl Thumbs {
    pub fn new(ctx: Context, threads: usize) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            cv: Condvar::new(),
            slots: Mutex::new(HashMap::new()),
            frame: AtomicU64::new(0),
        });
        for i in 0..threads.max(1) {
            let shared = shared.clone();
            let ctx = ctx.clone();
            let spawned = thread::Builder::new().name(format!("soyuz-thumbs-{i}")).spawn(move || worker(&shared, &ctx));
            if let Err(e) = spawned {
                log::error!("cannot spawn thumbnail thread: {e}");
            }
        }
        Thumbs { shared, enabled: true }
    }

    /// Call once per frame before any [`Thumbs::get`].
    pub fn begin_frame(&self) {
        self.shared.frame.fetch_add(1, Ordering::Relaxed);
    }

    /// The texture for `key` if ready; otherwise requests it (newest requests first).
    /// Pictures and videos (by extension).
    pub fn get(&self, key: &ThumbKey) -> Option<TextureHandle> {
        self.get_labeled(key).0
    }

    /// Like [`Self::get`], plus the label to show on it (a video's duration). A video
    /// without a frame (no ffmpeg) returns `(None, Some(duration))`.
    pub fn get_labeled(&self, key: &ThumbKey) -> (Option<TextureHandle>, Option<String>) {
        let kind = if super::thumb_video::is_video(&key.path) {
            Kind::Video
        } else if is_pdf_thumbnailable(&key.path) {
            Kind::Pdf
        } else {
            Kind::Image
        };
        self.request(key, kind)
    }

    /// A 2x2 mosaic of the pictures inside the folder `key.path` (`key.mtime` = the
    /// folder's mtime, so adding files refreshes it).
    pub fn get_folder(&self, key: &ThumbKey) -> Option<TextureHandle> {
        self.request(key, Kind::Folder).0
    }

    fn request(&self, key: &ThumbKey, kind: Kind) -> (Option<TextureHandle>, Option<String>) {
        if !self.enabled {
            return (None, None);
        }
        let frame = self.shared.frame.load(Ordering::Relaxed);
        let mut slots = self.shared.slots.lock();
        match slots.get_mut(key) {
            Some(Slot::Ready { tex, used, label, .. }) => {
                *used = frame;
                return (Some(tex.clone()), label.clone());
            }
            Some(Slot::Failed { label }) => return (None, label.clone()),
            Some(Slot::Pending { wanted }) => {
                *wanted = frame;
                return (None, None);
            }
            None => {}
        }
        slots.insert(key.clone(), Slot::Pending { wanted: frame });
        drop(slots);
        self.shared.queue.lock().push((key.clone(), kind));
        self.shared.cv.notify_one();
        (None, None)
    }

    /// Number of ready textures (for diagnostics/tests).
    pub fn ready_count(&self) -> usize {
        self.shared.slots.lock().values().filter(|s| matches!(s, Slot::Ready { .. })).count()
    }
}

fn worker(shared: &Shared, ctx: &Context) {
    loop {
        let (key, kind) = {
            let mut q = shared.queue.lock();
            loop {
                if let Some(k) = q.pop() {
                    break k;
                }
                shared.cv.wait(&mut q);
            }
        };
        let frame = shared.frame.load(Ordering::Relaxed);
        let still_wanted = match shared.slots.lock().get(&key) {
            Some(Slot::Pending { wanted }) => frame.saturating_sub(*wanted) <= STALE_FRAMES,
            _ => false,
        };
        if !still_wanted {
            // Forget it so a later request starts fresh.
            let mut slots = shared.slots.lock();
            if matches!(slots.get(&key), Some(Slot::Pending { .. })) {
                slots.remove(&key);
            }
            continue;
        }
        // Image decoders can panic on hostile input; never lose the worker over it.
        let (decoded, label) =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| produce(&key, kind))).unwrap_or((None, None));
        let result = decoded.map(|img| {
            let name = format!("thumb:{}:{}", key.path.display(), key.px);
            ctx.load_texture(name, img, TextureOptions::LINEAR)
        });
        let frame = shared.frame.load(Ordering::Relaxed);
        {
            let mut slots = shared.slots.lock();
            let slot = match result {
                Some(tex) => {
                    let [w, h] = tex.size();
                    Slot::Ready { tex, used: frame, bytes: (w * h * 4) as u64, label }
                }
                None => Slot::Failed { label },
            };
            slots.insert(key, slot);
            evict(&mut slots, frame, BUDGET.load(Ordering::Relaxed));
        }
        ctx.request_repaint();
    }
}

/// Drop least-recently-used ready textures while the total exceeds `budget` bytes
/// (textures used in the last [`KEEP_FRAMES`] frames stay), then publish the totals.
fn evict(slots: &mut HashMap<ThumbKey, Slot>, frame: u64, budget: u64) {
    let mut total: u64 = slots.values().map(|s| if let Slot::Ready { bytes, .. } = s { *bytes } else { 0 }).sum();
    if total > budget {
        let mut by_age: Vec<(u64, u64, ThumbKey)> = slots
            .iter()
            .filter_map(|(k, s)| match s {
                Slot::Ready { used, bytes, .. } if frame.saturating_sub(*used) > KEEP_FRAMES => {
                    Some((*used, *bytes, k.clone()))
                }
                _ => None,
            })
            .collect();
        by_age.sort_by_key(|(u, _, _)| *u);
        // Free a little extra (1/8 of the budget) so eviction doesn't run on every decode.
        let target = budget - budget / 8;
        for (_, bytes, k) in by_age {
            if total <= target {
                break;
            }
            slots.remove(&k);
            total -= bytes;
        }
    }
    USED_BYTES.store(total, Ordering::Relaxed);
    READY.store(slots.values().filter(|s| matches!(s, Slot::Ready { .. })).count() as u64, Ordering::Relaxed);
}

/// Make one thumbnail (disk cache first): `(picture, label)`.
fn produce(key: &ThumbKey, kind: Kind) -> (Option<ColorImage>, Option<String>) {
    use super::thumb_disk;
    let label = if kind == Kind::Video { super::thumb_video::duration_label(&key.path) } else { None };
    let disk_key = thumb_disk::cache_key(kind, &key.path, key.size, key.mtime, key.px);
    if let Some(img) = thumb_disk::get(disk_key) {
        return (Some(img), label);
    }
    // Explorer's own cache (read-only): no decode, and videos / PDFs get a picture even
    // without ffmpeg / pdfium.
    if kind != Kind::Folder
        && OS_CACHE.load(Ordering::Relaxed)
        && let Some(img) = os_cached(key)
    {
        thumb_disk::put(disk_key, &img);
        return (Some(img), label);
    }
    if kind == Kind::Video && !(VIDEO_FRAMES.load(Ordering::Relaxed) && super::thumb_video::ffmpeg_path().is_some()) {
        return (None, label);
    }
    if kind == Kind::Pdf && !pdf_available() {
        return (None, None);
    }
    let img = match kind {
        Kind::Image => decode(key),
        Kind::Video => super::thumb_video::frame(&key.path, key.px),
        Kind::Pdf => pdf_first_page(key),
        Kind::Folder => super::thumb_folder::mosaic(&key.path, key.px, |p, px| {
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX);
            decode(&ThumbKey { path: p.to_path_buf(), mtime: None, size, px })
        }),
    };
    if let Some(i) = &img {
        thumb_disk::put(disk_key, i);
    }
    (img, label)
}

/// The picture Explorer cached for `key.path`, fitted into `key.px` (`None` when it has
/// none at a useful size, or off Windows).
fn os_cached(key: &ThumbKey) -> Option<ColorImage> {
    let t = soyuz_fs::thumbcache::lookup(&key.path, key.px)?;
    let img = image::load_from_memory(&t.data).ok()?;
    let fit = if img.width() > key.px || img.height() > key.px { img.thumbnail(key.px, key.px) } else { img };
    let rgba = fit.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// PDF thumbnails are on and the pdfium library is found (else the PDF icon shows).
fn pdf_available() -> bool {
    if !PDF_PAGES.load(Ordering::Relaxed) {
        return false;
    }
    let dirs = super::preview_pdf::pdfium_dirs(&PDFIUM_DIR.lock());
    super::preview_pdf::find_pdfium(&dirs).is_some()
}

/// The first page rendered by pdfium, fitted into `key.px` square (on the pdfium thread).
#[cfg(feature = "pdf")]
fn pdf_first_page(key: &ThumbKey) -> Option<ColorImage> {
    use super::preview_pdf::{PdfSrc, pdfium, pdfium_dirs};
    if key.size > MAX_FILE_BYTES {
        return None;
    }
    let dirs = pdfium_dirs(&PDFIUM_DIR.lock());
    let src = PdfSrc::File(key.path.clone());
    let page = pdfium::render(&src, &dirs, MAX_FILE_BYTES, 0, key.px, key.px, &|| false).ok()?;
    let fit = image::RgbaImage::from_raw(page.width as u32, page.height as u32, page.rgba)?;
    let fit = if fit.width() > key.px || fit.height() > key.px {
        image::DynamicImage::ImageRgba8(fit).thumbnail(key.px, key.px).to_rgba8()
    } else {
        fit
    };
    Some(ColorImage::from_rgba_unmultiplied([fit.width() as usize, fit.height() as usize], fit.as_raw()))
}

#[cfg(not(feature = "pdf"))]
fn pdf_first_page(_key: &ThumbKey) -> Option<ColorImage> {
    None
}

fn decode(key: &ThumbKey) -> Option<ColorImage> {
    if key.size > MAX_FILE_BYTES {
        return None;
    }
    let mut reader = image::ImageReader::open(&key.path).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = image::ImageDecoder::orientation(&mut decoder).ok();
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    let thumb = img.thumbnail(key.px, key.px).to_rgba8();
    let size = [thumb.width() as usize, thumb.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, thumb.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_keeps_the_budget_and_recent_textures() {
        let ctx = egui::Context::default();
        let mut slots = HashMap::new();
        let tex = |i: usize| {
            ctx.load_texture(format!("t{i}"), ColorImage::filled([16, 16], egui::Color32::RED), TextureOptions::LINEAR)
        };
        // 10 textures of 1 KiB each, used in frames 0..10.
        for i in 0..10u64 {
            let key = ThumbKey { path: format!("/t{i}").into(), mtime: None, size: 1, px: 64 };
            slots.insert(key, Slot::Ready { tex: tex(i as usize), used: i, bytes: 1024, label: None });
        }
        evict(&mut slots, 10, 4 * 1024);
        let left: Vec<u64> =
            slots.values().filter_map(|s| if let Slot::Ready { used, .. } = s { Some(*used) } else { None }).collect();
        assert!(left.iter().all(|u| *u >= 7), "oldest evicted first: {left:?}");
        assert!(memory().0 <= 4 * 1024);
        // Everything recent: nothing can go even over budget.
        evict(&mut slots, 9, 1024);
        assert_eq!(slots.len(), left.len());
    }

    #[test]
    fn buckets() {
        assert_eq!(bucket(32.0, 1.0), 64);
        assert_eq!(bucket(96.0, 1.0), 128);
        assert_eq!(bucket(96.0, 2.0), 256);
        assert_eq!(bucket(256.0, 2.0), 512);
    }

    #[test]
    fn extensions() {
        assert!(is_thumbnailable(Path::new("a/b.JPG")));
        assert!(!is_thumbnailable(Path::new("a/b.txt")));
        assert!(!is_thumbnailable(Path::new("noext")));
    }

    #[test]
    fn decodes_a_png() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.png");
        let img = image::RgbaImage::from_pixel(300, 150, image::Rgba([10, 20, 30, 255]));
        img.save(&p).unwrap();
        let key = ThumbKey { path: p, mtime: None, size: 1, px: 64 };
        let c = decode(&key).unwrap();
        assert_eq!(c.size, [64, 32]);
    }
}
