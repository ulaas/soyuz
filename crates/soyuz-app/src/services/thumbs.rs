//! Thumbnail pipeline (PLAN §3): worker threads decode + downscale images and upload the
//! texture themselves (`egui::Context` is `Send + Sync`), so the UI thread only looks
//! textures up. Requests are served newest-first and dropped once their rows have been
//! scrolled away for a while, so fast scrolling never builds a backlog. An LRU keeps the
//! decoded textures within a byte budget (`[preview] thumb_cache_mib`, see
//! [`set_budget_bytes`]); [`memory`] reports the current use for the About dialog.

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

#[derive(Clone)]
enum Slot {
    Pending { wanted: u64 },
    Ready { tex: TextureHandle, used: u64, bytes: u64 },
    Failed,
}

struct Shared {
    queue: Mutex<Vec<ThumbKey>>,
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
    pub fn get(&self, key: &ThumbKey) -> Option<TextureHandle> {
        if !self.enabled {
            return None;
        }
        let frame = self.shared.frame.load(Ordering::Relaxed);
        let mut slots = self.shared.slots.lock();
        match slots.get_mut(key) {
            Some(Slot::Ready { tex, used, .. }) => {
                *used = frame;
                return Some(tex.clone());
            }
            Some(Slot::Failed) => return None,
            Some(Slot::Pending { wanted }) => {
                *wanted = frame;
                return None;
            }
            None => {}
        }
        slots.insert(key.clone(), Slot::Pending { wanted: frame });
        drop(slots);
        self.shared.queue.lock().push(key.clone());
        self.shared.cv.notify_one();
        None
    }

    /// Number of ready textures (for diagnostics/tests).
    pub fn ready_count(&self) -> usize {
        self.shared.slots.lock().values().filter(|s| matches!(s, Slot::Ready { .. })).count()
    }
}

fn worker(shared: &Shared, ctx: &Context) {
    loop {
        let key = {
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
        let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(&key))).ok().flatten();
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
                    Slot::Ready { tex, used: frame, bytes: (w * h * 4) as u64 }
                }
                None => Slot::Failed,
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
            slots.insert(key, Slot::Ready { tex: tex(i as usize), used: i, bytes: 1024 });
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
