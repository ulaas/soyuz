//! Thumbnail pipeline: worker threads decode + downscale images and upload the
//! texture themselves (through the frontend's [`Uploader`]), so the UI thread only looks
//! textures up. Requests are served newest-first and dropped once their rows have been
//! scrolled away for a while, so fast scrolling never builds a backlog. An LRU keeps the
//! decoded textures within a byte budget (`[preview] thumb_cache_mib`, see
//! [`set_budget_bytes`]) counted as GPU memory ([`gpu_bytes`]: pixels plus the per-texture
//! overhead) and under [`MAX_TEXTURES`]; textures not drawn for `[preview]
//! thumb_idle_trim_secs` are freed by a timer even while no frames run
//! ([`set_idle_trim_secs`]). [`memory`] reports the current use for the About dialog.
//!
//! Besides pictures, workers make video thumbnails (the FFmpeg libraries, `thumb_video`;
//! without them a duration label only), PDF first pages (pdfium, when the library is found; otherwise PDFs
//! keep their icon), folder mosaics (`thumb_folder`) and program icons (the icon in an
//! `.exe`'s resources, `soyuz_fs::exe_icon`); every decoded result also goes
//! through the on-disk cache (`thumb_disk`) so it survives restarts. On Linux / BSD,
//! pictures, video frames and PDF pages Soyuz makes itself are also written to the shared
//! freedesktop cache (`[appearance] write_os_thumbnails`, [`set_write_os_cache`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use image::RgbaImage;
use parking_lot::{Condvar, Mutex};

use super::preview_gfx::{Tex, Uploader};
use crate::host::Wake;

/// Default texture budget (bytes) until the config is applied.
pub const DEFAULT_BUDGET: u64 = 64 * 1024 * 1024;
/// Textures used within this many frames are never evicted for the byte budget (a
/// budget smaller than one screen of thumbnails must not thrash).
const KEEP_FRAMES: u64 = 2;
/// GPU allocation granularity of a texture: Direct3D 12 places every texture at a 64 KiB
/// aligned offset, so even a 64 px thumbnail (16 KiB of pixels) takes 64 KiB.
const TEXTURE_ALIGN: u64 = 64 * 1024;
/// Fixed GPU cost per texture besides its pixels: on Direct3D 12 every egui texture has
/// its own bind group, and wgpu gives each bind group a 64 KiB "internal sampler index"
/// buffer. Measured: ~300 thumbnails held 20 MB of pixels but 42 MB of GPU memory.
/// Other backends need less; the same estimate is used everywhere (budget, About).
const TEXTURE_OVERHEAD: u64 = 64 * 1024;
/// Most ready textures at once, whatever their size. Each takes descriptors from the
/// Direct3D 12 heap (capped at 131072 by `[performance] gpu_memory = "low"`); running it
/// out makes bind-group creation fail, which wgpu turns into a panic. Kept well under
/// half the heap so the rest of the UI always has room.
pub const MAX_TEXTURES: usize = 16_384;
const _: () = assert!(MAX_TEXTURES * 4 <= 131_072, "well under half the capped descriptor heap");
/// Most remembered failures (no picture; maybe a video duration label). They cost no
/// GPU memory but would otherwise pile up while browsing.
const MAX_FAILED: usize = 4096;

static BUDGET: AtomicU64 = AtomicU64::new(DEFAULT_BUDGET);
static USED_BYTES: AtomicU64 = AtomicU64::new(0);
static READY: AtomicU64 = AtomicU64::new(0);
/// `[preview] thumb_idle_trim_secs` (0 = off).
static IDLE_TRIM_SECS: AtomicU64 = AtomicU64::new(60);

/// Set the texture memory budget (`[preview] thumb_cache_mib`).
pub fn set_budget_bytes(bytes: u64) {
    BUDGET.store(bytes.max(1024 * 1024), Ordering::Relaxed);
}

/// Free thumbnails not drawn for `secs` seconds (`[preview] thumb_idle_trim_secs`; 0 =
/// keep them until the byte budget needs the room).
pub fn set_idle_trim_secs(secs: u64) {
    IDLE_TRIM_SECS.store(secs, Ordering::Relaxed);
}

/// `(bytes, textures)` of decoded thumbnails currently cached (bytes: the GPU estimate
/// of [`gpu_bytes`]).
pub fn memory() -> (u64, u64) {
    (USED_BYTES.load(Ordering::Relaxed), READY.load(Ordering::Relaxed))
}

/// Estimated GPU memory of a texture with `pixel_bytes` of RGBA8 pixels: rounded up to
/// [`TEXTURE_ALIGN`], plus [`TEXTURE_OVERHEAD`].
pub fn gpu_bytes(pixel_bytes: u64) -> u64 {
    pixel_bytes.div_ceil(TEXTURE_ALIGN).max(1) * TEXTURE_ALIGN + TEXTURE_OVERHEAD
}

/// Seconds since the first call (the idle-trim clock).
fn now_secs() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs()
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
    /// `used`: frame last drawn; `used_at`: [`now_secs`] of that frame; `bytes`: the
    /// [`gpu_bytes`] estimate.
    Ready {
        tex: Tex,
        used: u64,
        used_at: u64,
        bytes: u64,
        label: Option<String>,
    },
    /// No picture; videos may still have a duration label for the placeholder.
    Failed {
        used: u64,
        used_at: u64,
        label: Option<String>,
    },
}

impl Slot {
    /// `(frame, seconds)` last drawn (`None`: pending).
    fn last_use(&self) -> Option<(u64, u64)> {
        match self {
            Slot::Ready { used, used_at, .. } | Slot::Failed { used, used_at, .. } => Some((*used, *used_at)),
            Slot::Pending { .. } => None,
        }
    }
}

/// `[appearance] video_thumbnails` (read by the workers).
static VIDEO_FRAMES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turn video frame decoding on / off (`[appearance] video_thumbnails`; needs the FFmpeg
/// libraries, `services::media`).
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

/// `[appearance] program_icons` (read by the views and the workers).
static PROGRAM_ICONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Programs show the icon from their own resources (`[appearance] program_icons`).
pub fn set_program_icons(on: bool) {
    PROGRAM_ICONS.store(on, Ordering::Relaxed);
}

/// A file whose own icon is drawn instead of its glyph (when it has one): `.exe` / `.scr`
/// with `[appearance] program_icons` on.
pub fn has_program_icon(path: &Path) -> bool {
    PROGRAM_ICONS.load(Ordering::Relaxed) && soyuz_fs::exe_icon::has_icon_ext(path)
}

/// Pixel size of a program icon drawn `points` wide: exact (not bucketed) up to 64 px,
/// so the image made for that size is picked and the list rows stay crisp.
pub fn icon_px(points: f32, pixels_per_point: f32) -> u32 {
    let px = (points * pixels_per_point).ceil() as u32;
    if px <= 64 { px.max(16) } else { bucket(points, pixels_per_point) }
}

/// `[appearance] os_thumbnail_cache` (read by the workers).
static OS_CACHE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Use the OS thumbnail cache first: Explorer's `thumbcache_*.db` on Windows, the
/// freedesktop cache (`~/.cache/thumbnails`) on Linux / BSD. Off on macOS (no reader).
pub fn set_os_cache(on: bool) {
    OS_CACHE.store(on && OS_CACHE_SUPPORTED, Ordering::Relaxed);
}

/// This platform has an OS thumbnail cache reader in `soyuz_fs::thumbcache`.
const OS_CACHE_SUPPORTED: bool = cfg!(any(windows, all(unix, not(target_os = "macos"))));

/// `[appearance] write_os_thumbnails` (read by the workers).
static WRITE_OS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Write the thumbnails Soyuz makes itself into the freedesktop cache, at the size class
/// matching each request (Linux / BSD only; ignored elsewhere).
pub fn set_write_os_cache(on: bool) {
    WRITE_OS.store(on && WRITE_OS_SUPPORTED, Ordering::Relaxed);
}

/// This platform has a shared thumbnail cache Soyuz writes to.
const WRITE_OS_SUPPORTED: bool = cfg!(all(unix, not(target_os = "macos")));

/// A PDF that gets a first-page thumbnail (when pdfium turns out to be available).
pub fn is_pdf_thumbnailable(path: &Path) -> bool {
    PDF_PAGES.load(Ordering::Relaxed) && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

struct Shared {
    queue: Mutex<Vec<(ThumbKey, Kind)>>,
    cv: Condvar,
    slots: Mutex<HashMap<ThumbKey, Slot>>,
    frame: AtomicU64,
    /// [`now_secs`] at the latest [`Thumbs::begin_frame`].
    frame_secs: AtomicU64,
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
    /// `upload` makes the frontend's textures (on the workers); `wake` asks for a frame
    /// when one is ready.
    pub fn new(wake: Wake, upload: Uploader, threads: usize) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Vec::new()),
            cv: Condvar::new(),
            slots: Mutex::new(HashMap::new()),
            frame: AtomicU64::new(0),
            frame_secs: AtomicU64::new(now_secs()),
        });
        for i in 0..threads.max(1) {
            let shared = shared.clone();
            let (wake, upload) = (wake.clone(), upload.clone());
            let spawned =
                thread::Builder::new().name(format!("soyuz-thumbs-{i}")).spawn(move || worker(&shared, &wake, &upload));
            if let Err(e) = spawned {
                log::error!("cannot spawn thumbnail thread: {e}");
            }
        }
        let (trim_shared, trim_wake) = (shared.clone(), wake.clone());
        let spawned = thread::Builder::new()
            .name("soyuz-thumbs-trim".into())
            .spawn(move || idle_trimmer(&trim_shared, &trim_wake));
        if let Err(e) = spawned {
            log::error!("cannot spawn the thumbnail trim thread: {e}");
        }
        Thumbs { shared, enabled: true }
    }

    /// Call once per frame before any [`Thumbs::get`].
    pub fn begin_frame(&self) {
        self.shared.frame.fetch_add(1, Ordering::Relaxed);
        self.shared.frame_secs.store(now_secs(), Ordering::Relaxed);
    }

    /// The texture for `key` if ready; otherwise requests it (newest requests first).
    /// Pictures and videos (by extension).
    pub fn get(&self, key: &ThumbKey) -> Option<Tex> {
        self.get_labeled(key).0
    }

    /// Like [`Self::get`], plus the label to show on it (a video's duration). A video
    /// without a frame (no FFmpeg libraries) returns `(None, Some(duration))`.
    pub fn get_labeled(&self, key: &ThumbKey) -> (Option<Tex>, Option<String>) {
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
    pub fn get_folder(&self, key: &ThumbKey) -> Option<Tex> {
        self.request(key, Kind::Folder).0
    }

    /// The icon in the program `key.path` (see [`has_program_icon`]); `None` while it
    /// loads or when the program has none (its glyph stays).
    pub fn get_program_icon(&self, key: &ThumbKey) -> Option<Tex> {
        self.request(key, Kind::ProgramIcon).0
    }

    fn request(&self, key: &ThumbKey, kind: Kind) -> (Option<Tex>, Option<String>) {
        if !self.enabled {
            return (None, None);
        }
        let frame = self.shared.frame.load(Ordering::Relaxed);
        let now = self.shared.frame_secs.load(Ordering::Relaxed);
        let mut slots = self.shared.slots.lock();
        match slots.get_mut(key) {
            Some(Slot::Ready { tex, used, used_at, label, .. }) => {
                (*used, *used_at) = (frame, now);
                return (Some(tex.clone()), label.clone());
            }
            Some(Slot::Failed { used, used_at, label }) => {
                (*used, *used_at) = (frame, now);
                return (None, label.clone());
            }
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

fn worker(shared: &Shared, wake: &Wake, upload: &Uploader) {
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
            Tex::upload_rgba(upload, name, &img)
        });
        let frame = shared.frame.load(Ordering::Relaxed);
        let used_at = shared.frame_secs.load(Ordering::Relaxed);
        {
            let mut slots = shared.slots.lock();
            let slot = match result {
                Some(tex) => {
                    let bytes = gpu_bytes(tex.bytes());
                    Slot::Ready { tex, used: frame, used_at, bytes, label }
                }
                None => Slot::Failed { used: frame, used_at, label },
            };
            slots.insert(key, slot);
            evict(&mut slots, frame, BUDGET.load(Ordering::Relaxed));
        }
        wake();
    }
}

/// Frees thumbnails not drawn for `[preview] thumb_idle_trim_secs`, checking a few times
/// per idle period (so one lives at most ~1.25 × it), then runs a frame: egui releases
/// dropped textures only at the end of one, and an idle app runs none by itself.
fn idle_trimmer(shared: &Shared, wake: &Wake) {
    loop {
        let idle = IDLE_TRIM_SECS.load(Ordering::Relaxed);
        thread::sleep(Duration::from_secs(if idle == 0 { 15 } else { (idle / 4).clamp(1, 15) }));
        if idle == 0 {
            continue;
        }
        let frame = shared.frame.load(Ordering::Relaxed);
        let removed = {
            let mut slots = shared.slots.lock();
            let n = trim_idle(&mut slots, frame, now_secs(), idle);
            if n > 0 {
                publish(&slots);
            }
            n
        };
        if removed > 0 {
            wake();
        }
    }
}

/// Drop least-recently-used ready textures while the total exceeds `budget` bytes
/// (textures used in the last [`KEEP_FRAMES`] frames stay) or their number exceeds
/// [`MAX_TEXTURES`] (a hard limit: recent ones go too), and the oldest failures beyond
/// [`MAX_FAILED`]; then publish the totals.
fn evict(slots: &mut HashMap<ThumbKey, Slot>, frame: u64, budget: u64) {
    evict_with(slots, frame, budget, MAX_TEXTURES, MAX_FAILED);
}

fn evict_with(slots: &mut HashMap<ThumbKey, Slot>, frame: u64, budget: u64, max_ready: usize, max_failed: usize) {
    let ready = slots.values().filter(|s| matches!(s, Slot::Ready { .. })).count();
    if ready > max_ready {
        let mut by_age: Vec<(u64, ThumbKey)> = slots
            .iter()
            .filter_map(|(k, s)| match s {
                Slot::Ready { used, .. } => Some((*used, k.clone())),
                _ => None,
            })
            .collect();
        by_age.sort_by_key(|(u, _)| *u);
        // Down to 7/8 of the cap, so this doesn't run on every decode either.
        let excess = ready - (max_ready - max_ready / 8);
        for (_, k) in by_age.into_iter().take(excess) {
            slots.remove(&k);
        }
    }
    let failed = slots.values().filter(|s| matches!(s, Slot::Failed { .. })).count();
    if failed > max_failed {
        let mut by_age: Vec<(u64, ThumbKey)> = slots
            .iter()
            .filter_map(|(k, s)| match s {
                Slot::Failed { used, .. } if frame.saturating_sub(*used) > KEEP_FRAMES => Some((*used, k.clone())),
                _ => None,
            })
            .collect();
        by_age.sort_by_key(|(u, _)| *u);
        let excess = failed - (max_failed - max_failed / 8);
        for (_, k) in by_age.into_iter().take(excess) {
            slots.remove(&k);
        }
    }
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
    publish(slots);
}

/// Publish the totals [`memory`] reports.
fn publish(slots: &HashMap<ThumbKey, Slot>) {
    let (mut bytes, mut count) = (0, 0);
    for s in slots.values() {
        if let Slot::Ready { bytes: b, .. } = s {
            bytes += b;
            count += 1;
        }
    }
    USED_BYTES.store(bytes, Ordering::Relaxed);
    READY.store(count, Ordering::Relaxed);
}

/// Drop the textures and failures not drawn for `idle` seconds at `now` ([`now_secs`]).
/// Those drawn in the latest frame (`frame`) stay however old: an idle app runs no
/// frames, but what it showed last is still on screen. Returns how many went.
///
/// Nothing more is freed while every window is minimized: eframe runs no egui pass then,
/// so dropped textures would only be released on restore, just when the visible ones
/// are needed again.
fn trim_idle(slots: &mut HashMap<ThumbKey, Slot>, frame: u64, now: u64, idle: u64) -> usize {
    let before = slots.len();
    slots.retain(|_, s| match s.last_use() {
        Some((used, used_at)) => used >= frame || now.saturating_sub(used_at) < idle,
        None => true,
    });
    before - slots.len()
}

/// Make one thumbnail (disk cache first): `(picture, label)`.
fn produce(key: &ThumbKey, kind: Kind) -> (Option<RgbaImage>, Option<String>) {
    use super::thumb_disk;
    let label = if kind == Kind::Video { super::thumb_video::duration_label(&key.path) } else { None };
    // Video frames are keyed by the library build that made them (`media::source_tag`).
    let source = if kind == Kind::Video { super::media::source_tag() } else { String::new() };
    let disk_key = thumb_disk::cache_key_with_source(kind, &key.path, key.size, key.mtime, key.px, &source);
    if let Some(img) = thumb_disk::get(disk_key) {
        return (Some(img), label);
    }
    // The OS cache (Explorer's / freedesktop, read-only): no decode, and videos / PDFs
    // get a picture even without the FFmpeg libraries / pdfium.
    if !matches!(kind, Kind::Folder | Kind::ProgramIcon)
        && OS_CACHE.load(Ordering::Relaxed)
        && let Some(img) = os_cached(key)
    {
        thumb_disk::put(disk_key, &img);
        return (Some(img), label);
    }
    if kind == Kind::Video && !(VIDEO_FRAMES.load(Ordering::Relaxed) && super::media::runtime().is_some()) {
        return (None, label);
    }
    if kind == Kind::Pdf && !pdf_available() {
        return (None, None);
    }
    let img = match kind {
        Kind::Image | Kind::Video | Kind::Pdf if WRITE_OS.load(Ordering::Relaxed) => make_shared(key, kind),
        Kind::Image => decode(key),
        Kind::Video => super::thumb_video::frame(&key.path, key.px),
        Kind::Pdf => pdf_first_page(key),
        Kind::ProgramIcon => program_icon(key),
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

/// Make the picture at the freedesktop size class matching `key.px` (never upscaled),
/// write it to the shared cache unless the file changed meanwhile, then fit it into
/// `key.px` for display (as [`decode`] / [`thumb_video::frame`] / [`pdf_first_page`]
/// would have made it).
///
/// [`thumb_video::frame`]: super::thumb_video::frame
fn make_shared(key: &ThumbKey, kind: Kind) -> Option<RgbaImage> {
    let (_, class_px) = soyuz_fs::thumbcache::freedesktop_class(key.px);
    let before = std::fs::metadata(&key.path).and_then(|m| m.modified()).ok();
    let class_key = ThumbKey { px: class_px, ..key.clone() };
    let rgba = match kind {
        Kind::Image => {
            let img = decode_full(key)?;
            if img.width() > class_px || img.height() > class_px { img.thumbnail(class_px, class_px) } else { img }
                .to_rgba8()
        }
        Kind::Video => super::thumb_video::frame(&key.path, class_px)?,
        Kind::Pdf => pdf_first_page(&class_key)?,
        Kind::Folder | Kind::ProgramIcon => return None,
    };
    if before.is_some() {
        share_thumb(&key.path, &rgba, class_px, before);
    }
    let fit = if kind == Kind::Image || rgba.width() > key.px || rgba.height() > key.px {
        image::DynamicImage::ImageRgba8(rgba).thumbnail(key.px, key.px).to_rgba8()
    } else {
        rgba
    };
    Some(fit)
}

/// Encode `rgba` and write it as the freedesktop thumbnail of `path` (class `class_px`),
/// unless a valid one of that class is already there. Errors are only logged.
fn share_thumb(path: &Path, rgba: &image::RgbaImage, class_px: u32, before: Option<SystemTime>) {
    if soyuz_fs::thumbcache::lookup(path, class_px).is_some_and(|t| t.db_size == class_px) {
        return;
    }
    let png = match encode_png(rgba) {
        Ok(png) => png,
        Err(e) => {
            log::debug!("thumbnail PNG for {}: {e}", path.display());
            return;
        }
    };
    if let Err(e) = soyuz_fs::thumbcache::write_freedesktop(path, &png, class_px, before) {
        log::debug!("cannot share the thumbnail of {}: {e}", path.display());
    }
}

/// 8-bit RGBA PNG of `rgba`.
fn encode_png(rgba: &image::RgbaImage) -> image::ImageResult<Vec<u8>> {
    let mut png = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut png);
    image::ImageEncoder::write_image(enc, rgba.as_raw(), rgba.width(), rgba.height(), image::ExtendedColorType::Rgba8)?;
    Ok(png)
}

/// The picture the OS cached for `key.path` (Explorer's `thumbcache_*.db` or the
/// freedesktop cache), fitted into `key.px` (`None` when it has none at a useful size,
/// or on macOS).
fn os_cached(key: &ThumbKey) -> Option<RgbaImage> {
    let t = soyuz_fs::thumbcache::lookup(&key.path, key.px)?;
    fit_os_thumb(&t, key.px)
}

/// Decode an OS-cached picture and fit it into `px` (never upscaled).
fn fit_os_thumb(t: &soyuz_fs::thumbcache::OsThumb, px: u32) -> Option<RgbaImage> {
    let img = image::load_from_memory(&t.data).ok()?;
    let fit = if img.width() > px || img.height() > px { img.thumbnail(px, px) } else { img };
    Some(fit.to_rgba8())
}

/// PDF thumbnails are on and the pdfium library is found (else the PDF icon shows).
fn pdf_available() -> bool {
    if !PDF_PAGES.load(Ordering::Relaxed) {
        return false;
    }
    let dirs = super::preview_pdf::pdfium_dirs(&PDFIUM_DIR.lock());
    super::preview_pdf::pdfium_available(&dirs)
}

/// The first page rendered by pdfium, fitted into `key.px` square (on the pdfium thread).
#[cfg(feature = "pdf")]
fn pdf_first_page(key: &ThumbKey) -> Option<RgbaImage> {
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
    Some(fit)
}

#[cfg(not(feature = "pdf"))]
fn pdf_first_page(_key: &ThumbKey) -> Option<RgbaImage> {
    None
}

/// The program's icon image closest to `key.px`, scaled to fit it (Lanczos, so small
/// icons stay sharp).
fn program_icon(key: &ThumbKey) -> Option<RgbaImage> {
    let ico = soyuz_fs::exe_icon::icon(&key.path, key.px)?;
    let img = image::load_from_memory_with_format(&ico, image::ImageFormat::Ico).ok()?;
    let img = if img.width() == key.px && img.height() == key.px {
        img
    } else {
        img.resize(key.px, key.px, image::imageops::FilterType::Lanczos3)
    };
    Some(img.to_rgba8())
}

fn decode(key: &ThumbKey) -> Option<RgbaImage> {
    Some(decode_full(key)?.thumbnail(key.px, key.px).to_rgba8())
}

/// The whole picture, EXIF orientation applied (within the decode limits).
fn decode_full(key: &ThumbKey) -> Option<image::DynamicImage> {
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
    Some(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviction_keeps_the_budget_and_recent_textures() {
        let mut slots = HashMap::new();
        let up = super::super::preview_gfx::no_upload();
        let tex = |i: usize| Tex::upload_rgba(&up, format!("t{i}"), &RgbaImage::new(16, 16));
        // 10 textures of 1 KiB each, used in frames 0..10.
        for i in 0..10u64 {
            let key = ThumbKey { path: format!("/t{i}").into(), mtime: None, size: 1, px: 64 };
            slots.insert(key, Slot::Ready { tex: tex(i as usize), used: i, used_at: 0, bytes: 1024, label: None });
        }
        evict(&mut slots, 10, 4 * 1024);
        let left: Vec<u64> =
            slots.values().filter_map(|s| if let Slot::Ready { used, .. } = s { Some(*used) } else { None }).collect();
        assert!(left.iter().all(|u| *u >= 7), "oldest evicted first: {left:?}");
        // (Summed here: `memory()` is process-wide, and other tests evict concurrently.)
        let total: u64 = slots.values().map(|s| if let Slot::Ready { bytes, .. } = s { *bytes } else { 0 }).sum();
        assert!(total <= 4 * 1024);
        // Everything recent: nothing can go even over budget.
        evict(&mut slots, 9, 1024);
        assert_eq!(slots.len(), left.len());
    }

    fn key(i: usize) -> ThumbKey {
        ThumbKey { path: format!("/k{i}").into(), mtime: None, size: 1, px: 64 }
    }

    fn ready(used: u64, used_at: u64, px: u32) -> Slot {
        let up = super::super::preview_gfx::no_upload();
        let tex = Tex::upload_rgba(&up, "t".into(), &RgbaImage::new(px, px));
        Slot::Ready { bytes: gpu_bytes(tex.bytes()), tex, used, used_at, label: None }
    }

    #[test]
    fn gpu_cost_counts_alignment_and_bind_group() {
        // 64 px: 16 KiB of pixels in one 64 KiB block, plus the 64 KiB sampler buffer.
        assert_eq!(gpu_bytes(64 * 64 * 4), 128 * 1024);
        // 256 px: exactly 256 KiB of pixels.
        assert_eq!(gpu_bytes(256 * 256 * 4), 320 * 1024);
        assert_eq!(gpu_bytes(256 * 256 * 4 + 1), 384 * 1024);
        assert_eq!(gpu_bytes(0), 128 * 1024, "even an empty texture takes a block");
        // 300 thumbnails of 128 px (64 KiB of pixels): about what Direct3D 12 showed (42 MB).
        let est = 300 * gpu_bytes(128 * 128 * 4);
        assert!((38_000_000..46_000_000).contains(&est), "{est}");
        // The budget sees the estimate: ten 64 px textures take 1.25 MiB, not 160 KiB.
        let mut slots: HashMap<ThumbKey, Slot> = (0..10).map(|i| (key(i), ready(i as u64, 0, 64))).collect();
        evict(&mut slots, 100, 640 * 1024);
        assert_eq!(slots.len(), 4, "7/8 of 640 KiB holds four 128 KiB textures");
    }

    #[test]
    fn texture_count_is_capped_even_for_recent_textures() {
        let mut slots: HashMap<ThumbKey, Slot> = (0..20).map(|i| (key(i), ready(i as u64, 0, 1))).collect();
        // All drawn within KEEP_FRAMES and far under the byte budget: only the cap applies.
        evict_with(&mut slots, 19, u64::MAX, 16, 100);
        let left: Vec<u64> = slots.values().filter_map(|s| s.last_use().map(|u| u.0)).collect();
        assert_eq!(left.len(), 14, "down to 7/8 of the cap");
        assert!(left.iter().all(|u| *u >= 6), "oldest first: {left:?}");
    }

    #[test]
    fn failures_are_evicted_too() {
        let mut slots: HashMap<ThumbKey, Slot> =
            (0..20).map(|i| (key(i), Slot::Failed { used: i as u64, used_at: 0, label: None })).collect();
        slots.insert(key(100), ready(0, 0, 1));
        evict_with(&mut slots, 20, u64::MAX, 100, 8);
        let failed: Vec<u64> =
            slots.values().filter_map(|s| if let Slot::Failed { used, .. } = s { Some(*used) } else { None }).collect();
        assert_eq!(failed.len(), 7);
        assert!(failed.iter().all(|u| *u >= 13), "oldest failures first: {failed:?}");
        assert_eq!(slots.values().filter(|s| matches!(s, Slot::Ready { .. })).count(), 1, "textures untouched");
    }

    #[test]
    fn idle_trim_frees_what_was_not_drawn_lately() {
        let mut slots: HashMap<ThumbKey, Slot> = HashMap::new();
        slots.insert(key(0), ready(5, 100, 64)); // drawn in frame 5 at t=100
        slots.insert(key(1), ready(9, 100, 64)); // drawn in the latest frame (9), also at t=100
        slots.insert(key(2), ready(8, 150, 64)); // drawn recently
        slots.insert(key(3), Slot::Failed { used: 3, used_at: 90, label: None });
        slots.insert(key(4), Slot::Pending { wanted: 1 });
        // t=155 with 60 s: only the failure from t=90 is that old.
        assert_eq!(trim_idle(&mut slots, 9, 155, 60), 1);
        assert!(!slots.contains_key(&key(3)));
        // t=200: the frame-5 texture goes; the latest frame's stays though just as old
        // (an idle app still shows it); pending requests are the workers' business.
        assert_eq!(trim_idle(&mut slots, 9, 200, 60), 1);
        assert!(!slots.contains_key(&key(0)));
        assert!(slots.contains_key(&key(1)) && slots.contains_key(&key(2)) && slots.contains_key(&key(4)));
        // t=300 after another frame (10) that drew nothing: the rest goes too.
        assert_eq!(trim_idle(&mut slots, 10, 300, 60), 2);
        assert_eq!(slots.len(), 1);
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
    fn os_cache_is_supported_where_a_reader_exists() {
        set_os_cache(true);
        assert_eq!(OS_CACHE.load(Ordering::Relaxed), cfg!(any(windows, target_os = "linux")));
        set_os_cache(false);
        assert!(!OS_CACHE.load(Ordering::Relaxed));
        set_os_cache(true);
    }

    #[test]
    fn os_thumbs_are_fitted_not_upscaled() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(256, 192, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let t = soyuz_fs::thumbcache::OsThumb { data: png, width: 256, height: 192, db_size: 256 };
        assert_eq!(fit_os_thumb(&t, 128).unwrap().dimensions(), (128, 96));
        assert_eq!(fit_os_thumb(&t, 512).unwrap().dimensions(), (256, 192));
        let bad = soyuz_fs::thumbcache::OsThumb { data: b"junk".to_vec(), width: 1, height: 1, db_size: 96 };
        assert!(fit_os_thumb(&bad, 64).is_none());
    }

    #[test]
    fn write_os_cache_only_where_supported() {
        set_write_os_cache(true);
        assert_eq!(WRITE_OS.load(Ordering::Relaxed), cfg!(all(unix, not(target_os = "macos"))));
        set_write_os_cache(false);
        assert!(!WRITE_OS.load(Ordering::Relaxed));
    }

    /// What the workers encode is accepted by the freedesktop writer, read back by the
    /// lookup and still decodes as a picture with its text chunks in place.
    #[cfg(unix)]
    #[test]
    fn shared_thumbs_round_trip_through_a_private_cache() {
        let d = tempfile::tempdir().unwrap();
        let (cache, trash) = (d.path().join("thumbnails"), d.path().join("Trash"));
        let src = d.path().join("pic.png");
        image::RgbaImage::from_pixel(600, 300, image::Rgba([200, 100, 50, 255])).save(&src).unwrap();
        let before = std::fs::metadata(&src).unwrap().modified().ok();
        let key = ThumbKey { path: src.clone(), mtime: None, size: 1, px: 64 };
        let (_, class_px) = soyuz_fs::thumbcache::freedesktop_class(key.px);
        let rgba = decode_full(&key).unwrap().thumbnail(class_px, class_px).to_rgba8();
        assert_eq!(rgba.dimensions(), (128, 64));
        let png = encode_png(&rgba).unwrap();
        soyuz_fs::thumbcache::write_freedesktop_in(&cache, &trash, &src, &png, class_px, before, Some("image/png"))
            .unwrap()
            .unwrap();
        let t = soyuz_fs::thumbcache::lookup_freedesktop(&cache, &src, 128).unwrap();
        assert_eq!((t.width, t.height), (128, 64));
        let back = image::load_from_memory(&t.data).unwrap().to_rgba8();
        assert_eq!(back.get_pixel(5, 5).0, [200, 100, 50, 255]);
    }

    /// Without the FFmpeg libraries a video gets its duration label and no picture (the
    /// pre-Phase-V behaviour minus the `ffmpeg` program); with them, a 24-byte "mp4"
    /// still decodes to nothing.
    #[test]
    fn video_without_a_runtime_is_label_only() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("clip.mp4");
        // A minimal MP4 header with an mvhd box: 1 s at timescale 1000 (no frames).
        let mut mp4 = Vec::new();
        mp4.extend_from_slice(b"\0\0\0\x18ftypisom\0\0\0\0isomiso2");
        let mvhd = {
            let mut b = Vec::new();
            b.extend_from_slice(&[0, 0, 0, 0]); // version + flags
            b.extend_from_slice(&[0; 8]); // creation / modification
            b.extend_from_slice(&1000u32.to_be_bytes()); // timescale
            b.extend_from_slice(&1000u32.to_be_bytes()); // duration
            let mut boxed = ((b.len() + 8) as u32).to_be_bytes().to_vec();
            boxed.extend_from_slice(b"mvhd");
            boxed.extend(b);
            boxed
        };
        let mut moov = ((mvhd.len() + 8) as u32).to_be_bytes().to_vec();
        moov.extend_from_slice(b"moov");
        moov.extend(mvhd);
        mp4.extend(moov);
        std::fs::write(&p, &mp4).unwrap();
        let size = mp4.len() as u64;
        // Never read the user's Explorer / freedesktop cache from a test.
        set_os_cache(false);
        set_video_frames(true);
        let key = ThumbKey { path: p, mtime: None, size, px: 64 };
        let (img, label) = produce(&key, Kind::Video);
        assert!(img.is_none());
        assert_eq!(label.as_deref(), Some("0:01"));
        // The feature switch alone keeps the label too.
        set_video_frames(false);
        let (img, label) = produce(&key, Kind::Video);
        assert!(img.is_none());
        assert_eq!(label.as_deref(), Some("0:01"));
        set_video_frames(true);
        set_os_cache(true);
    }

    #[test]
    fn program_icon_px() {
        assert_eq!(icon_px(18.0, 1.0), 18);
        assert_eq!(icon_px(18.0, 1.5), 27);
        assert_eq!(icon_px(10.0, 1.0), 16);
        assert_eq!(icon_px(96.0, 1.0), 128);
        set_program_icons(true);
        assert!(has_program_icon(Path::new("C:/a/setup.exe")));
        assert!(!has_program_icon(Path::new("C:/a/notes.txt")));
    }

    /// A real program of the running system (read-only), decoded at a list-row size.
    #[cfg(windows)]
    #[test]
    fn decodes_a_windows_program_icon() {
        let Some(root) = std::env::var_os("SystemRoot") else { return };
        let path = Path::new(&root).join("System32").join("cmd.exe");
        let Ok(meta) = std::fs::metadata(&path) else { return };
        let key = ThumbKey { path, mtime: meta.modified().ok(), size: meta.len(), px: 27 };
        let img = program_icon(&key).expect("cmd.exe icon decodes");
        assert_eq!((img.width(), img.height()), (27, 27));
        assert!(img.pixels().any(|p| p.0[3] > 0), "not fully transparent");
    }

    #[test]
    fn decodes_a_png() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.png");
        let img = image::RgbaImage::from_pixel(300, 150, image::Rgba([10, 20, 30, 255]));
        img.save(&p).unwrap();
        let key = ThumbKey { path: p, mtime: None, size: 1, px: 64 };
        let c = decode(&key).unwrap();
        assert_eq!(c.dimensions(), (64, 32));
    }
}
