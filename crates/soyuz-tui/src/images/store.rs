//! The terminal UI's texture store: the [`Uploader`] the core's workers (previews,
//! thumbnails, editors) hand decoded pixels to. Pixels are kept as straight RGBA under a
//! unique id in a bounded store (least recently drawn evicted first); the handle inside
//! the [`Tex`] removes its pixels when the last clone is dropped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use image::RgbaImage;
use image::imageops::FilterType;
use parking_lot::Mutex;
use soyuz_appcore::services::preview_gfx::{Pixels, Tex, Uploader};
use soyuz_core::config::Config;

/// Longest side kept: larger uploads are scaled down on the worker thread (no terminal
/// shows more pixels than this in one image, and encoding stays cheap).
pub const MAX_SIDE: u32 = 2048;
/// The smallest store budget, whatever the settings say.
pub const MIN_CAP: u64 = 16 << 20;

/// The store budget in bytes from the settings: the decoded thumbnail budget plus the
/// decoded preview budget (`[preview] thumb_cache_mib` + `preview_cache_mib`), the same
/// memory the GUI spends on these textures.
pub fn cap_for(config: &Config) -> u64 {
    let mib = config.preview.thumb_cache_mib.saturating_add(config.preview.preview_cache_mib);
    mib.saturating_mul(1 << 20).max(MIN_CAP)
}

struct Entry {
    img: Arc<RgbaImage>,
    bytes: u64,
    used: u64,
}

#[derive(Default)]
struct Inner {
    map: HashMap<u64, Entry>,
    bytes: u64,
    cap: u64,
    tick: u64,
}

impl Inner {
    fn evict(&mut self, keep: u64) {
        while self.bytes > self.cap && self.map.len() > 1 {
            let Some(oldest) =
                self.map.iter().filter(|(id, _)| **id != keep).min_by_key(|(_, e)| e.used).map(|(id, _)| *id)
            else {
                break;
            };
            if let Some(e) = self.map.remove(&oldest) {
                self.bytes -= e.bytes;
            }
        }
    }
}

/// Uploaded pixels by id.
pub struct Store {
    inner: Mutex<Inner>,
    next: AtomicU64,
}

impl Store {
    /// An empty store holding at most `cap` bytes (at least one image).
    pub fn new(cap: u64) -> Arc<Store> {
        Arc::new(Store { inner: Mutex::new(Inner { cap, ..Inner::default() }), next: AtomicU64::new(1) })
    }

    /// Change the budget (evicts at once when it shrank).
    pub fn set_cap(&self, cap: u64) {
        let mut g = self.inner.lock();
        if g.cap != cap {
            g.cap = cap;
            g.evict(0);
        }
    }

    fn insert(&self, img: RgbaImage) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let bytes = img.as_raw().len() as u64;
        let mut g = self.inner.lock();
        g.tick += 1;
        let used = g.tick;
        g.map.insert(id, Entry { img: Arc::new(img), bytes, used });
        g.bytes += bytes;
        g.evict(id);
        id
    }

    /// The pixels of `id` (marks them recently used); `None` once evicted or freed.
    pub fn get(&self, id: u64) -> Option<Arc<RgbaImage>> {
        let mut g = self.inner.lock();
        g.tick += 1;
        let tick = g.tick;
        let e = g.map.get_mut(&id)?;
        e.used = tick;
        Some(e.img.clone())
    }

    /// Whether `id` is still stored.
    pub fn contains(&self, id: u64) -> bool {
        self.inner.lock().map.contains_key(&id)
    }

    fn remove(&self, id: u64) {
        let mut g = self.inner.lock();
        if let Some(e) = g.map.remove(&id) {
            g.bytes -= e.bytes;
        }
    }

    /// `(images, bytes)` stored now.
    pub fn stats(&self) -> (usize, u64) {
        let g = self.inner.lock();
        (g.map.len(), g.bytes)
    }
}

/// The native handle inside a [`Tex`] made by [`uploader`]: an id in the store. Dropping
/// the last clone of the `Tex` frees the pixels.
pub struct TuiTex {
    id: u64,
    store: Weak<Store>,
}

impl TuiTex {
    /// The id the pixels are stored under (0: nothing stored, the upload was invalid).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The pixels, when still stored.
    pub fn pixels(&self) -> Option<Arc<RgbaImage>> {
        if self.id == 0 {
            return None;
        }
        self.store.upgrade()?.get(self.id)
    }
}

impl Drop for TuiTex {
    fn drop(&mut self) {
        if self.id != 0
            && let Some(s) = self.store.upgrade()
        {
            s.remove(self.id);
        }
    }
}

/// The stored pixels behind `tex` (`None`: not ours, invalid, evicted).
pub fn lookup(tex: &Tex) -> Option<(u64, Arc<RgbaImage>)> {
    let h = tex.handle::<TuiTex>()?;
    Some((h.id, h.pixels()?))
}

/// Straight RGBA from `px` (un-premultiplied), scaled down to [`MAX_SIDE`]. `None` when
/// the buffer does not match the size.
pub fn straight(px: Pixels<'_>) -> Option<RgbaImage> {
    let [w, h] = px.size;
    if w == 0 || h == 0 || px.rgba.len() != w.checked_mul(h)?.checked_mul(4)? {
        return None;
    }
    let mut img = RgbaImage::from_raw(u32::try_from(w).ok()?, u32::try_from(h).ok()?, px.rgba.to_vec())?;
    if px.premultiplied {
        for p in img.pixels_mut() {
            let a = u32::from(p[3]);
            if a != 0 && a != 255 {
                for c in &mut p.0[..3] {
                    *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
    }
    let (w, h) = img.dimensions();
    if w.max(h) > MAX_SIDE {
        let s = f64::from(MAX_SIDE) / f64::from(w.max(h));
        let nw = ((f64::from(w) * s).round() as u32).max(1);
        let nh = ((f64::from(h) * s).round() as u32).max(1);
        img = image::imageops::resize(&img, nw, nh, FilterType::Triangle);
    }
    Some(img)
}

/// The terminal UI's [`Uploader`] into `store` (called on worker threads).
pub fn uploader(store: &Arc<Store>) -> Uploader {
    let store = Arc::downgrade(store);
    Arc::new(move |_name, px| {
        let id = match (straight(px), store.upgrade()) {
            (Some(img), Some(s)) => s.insert(img),
            _ => 0,
        };
        Arc::new(TuiTex { id, store: store.clone() })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(u: &Uploader, w: usize, h: usize, v: u8) -> Tex {
        let rgba = vec![v; w * h * 4];
        Tex::upload(u, "t".into(), Pixels { size: [w, h], rgba: &rgba, premultiplied: false })
    }

    #[test]
    fn upload_lookup_drop() {
        let s = Store::new(1 << 20);
        let u = uploader(&s);
        let t = up(&u, 4, 2, 7);
        let (id, img) = lookup(&t).expect("stored");
        assert!(id > 0);
        assert_eq!(img.dimensions(), (4, 2));
        let t2 = t.clone();
        drop(t);
        assert!(s.contains(id), "a clone keeps it");
        drop(t2);
        assert!(!s.contains(id), "the last clone frees it");
        assert_eq!(s.stats(), (0, 0));
    }

    #[test]
    fn lru_evicts_least_recently_drawn() {
        // 3 images of 4x4x4 = 64 bytes; room for 2.
        let s = Store::new(130);
        let u = uploader(&s);
        let a = up(&u, 4, 4, 1);
        let b = up(&u, 4, 4, 2);
        assert!(lookup(&a).is_some(), "a drawn after b");
        let c = up(&u, 4, 4, 3);
        assert!(lookup(&b).is_none(), "b was least recently used");
        assert!(lookup(&a).is_some() && lookup(&c).is_some());
        assert_eq!(s.stats(), (2, 128));
        s.set_cap(64);
        assert_eq!(s.stats().0, 1, "shrinking evicts");
    }

    #[test]
    fn invalid_and_foreign_textures() {
        let s = Store::new(1 << 20);
        let u = uploader(&s);
        let bad = Tex::upload(&u, "x".into(), Pixels { size: [3, 3], rgba: &[0; 5], premultiplied: false });
        assert!(lookup(&bad).is_none());
        let foreign = up(&soyuz_appcore::services::preview_gfx::no_upload(), 2, 2, 0);
        assert!(lookup(&foreign).is_none());
        let empty = up(&u, 0, 3, 0);
        assert!(lookup(&empty).is_none());
    }

    #[test]
    fn premultiplied_is_unmultiplied_and_big_is_scaled() {
        let px = [50u8, 0, 100, 128];
        let img = straight(Pixels { size: [1, 1], rgba: &px, premultiplied: true }).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [100, 0, 199, 128]);
        let big = vec![0u8; 4096 * 8 * 4];
        let img = straight(Pixels { size: [4096, 8], rgba: &big, premultiplied: false }).unwrap();
        assert_eq!(img.dimensions(), (MAX_SIDE, 4));
    }

    #[test]
    fn cap_follows_preview_budgets() {
        let mut c = Config::default();
        c.preview.thumb_cache_mib = 100;
        c.preview.preview_cache_mib = 28;
        assert_eq!(cap_for(&c), 128 << 20);
        c.preview.thumb_cache_mib = 0;
        c.preview.preview_cache_mib = 0;
        assert_eq!(cap_for(&c), MIN_CAP);
    }
}
