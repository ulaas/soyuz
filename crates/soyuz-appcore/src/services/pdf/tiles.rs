//! Tiles of a zoomed page and the texture cache that holds them.
//!
//! A page at a zoom is cut into [`TILE`]-pixel squares (large on purpose: every region
//! render interprets the whole page). The viewer asks for the tiles that intersect the
//! viewport, centre-out, and keeps uploaded textures in a [`TexCache`] with a byte
//! budget that evicts the entries farthest from the viewport first.

use std::collections::HashMap;
use std::hash::Hash;

use super::{PageGeom, Region};
use crate::services::preview_gfx::Tex;

/// Side of a full tile in pixels.
pub const TILE: u32 = 1024;

/// One tile: page, zoom (pixels per point × 1000), view rotation (quarter turns
/// clockwise), tile column / row and the dark-pages flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub page: u32,
    pub scale_milli: u32,
    pub rot: u8,
    pub tx: u32,
    pub ty: u32,
    pub invert: bool,
}

/// A whole-page bitmap (base layer or thumbnail): page, view rotation, dark-pages flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PageKey {
    pub page: u32,
    pub rot: u8,
    pub invert: bool,
}

impl TileKey {
    /// Pixels per point.
    pub fn scale(&self) -> f32 {
        self.scale_milli as f32 / 1000.0
    }

    /// The pixel rectangle this tile covers on page `geom` (clipped to the page), or
    /// `None` when the tile lies past the page edge.
    pub fn region(&self, geom: &PageGeom) -> Option<Region> {
        let (pw, ph) = geom.px(self.scale(), self.rot);
        let (x, y) = (self.tx.checked_mul(TILE)?, self.ty.checked_mul(TILE)?);
        if x >= pw || y >= ph {
            return None;
        }
        Some(Region {
            page: self.page,
            scale: self.scale(),
            rot_quarter: self.rot,
            x,
            y,
            w: TILE.min(pw - x),
            h: TILE.min(ph - y),
        })
    }
}

/// Tiles of page `page` (geometry `geom`) that intersect `view` (`[x0, y0, x1, y1]` in
/// that page's pixels at the zoom), nearest the view centre first.
pub fn visible_tiles(
    page: u32,
    geom: &PageGeom,
    scale_milli: u32,
    rot: u8,
    invert: bool,
    view: [f32; 4],
) -> Vec<TileKey> {
    let scale = scale_milli as f32 / 1000.0;
    let (pw, ph) = geom.px(scale, rot);
    let x0 = view[0].max(0.0);
    let y0 = view[1].max(0.0);
    let x1 = view[2].min(pw as f32);
    let y1 = view[3].min(ph as f32);
    if x1 <= x0 || y1 <= y0 {
        return Vec::new();
    }
    let t = TILE as f32;
    let (c0, c1) = ((x0 / t) as u32, ((x1 - 0.001) / t) as u32);
    let (r0, r1) = ((y0 / t) as u32, ((y1 - 0.001) / t) as u32);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let mut out: Vec<(f32, TileKey)> = Vec::new();
    for ty in r0..=r1 {
        for tx in c0..=c1 {
            let mx = (tx as f32 + 0.5) * t;
            let my = (ty as f32 + 0.5) * t;
            let d = (mx - cx).powi(2) + (my - cy).powi(2);
            out.push((d, TileKey { page, scale_milli, rot, tx, ty, invert }));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, k)| k).collect()
}

struct Entry {
    tex: Tex,
    page: u32,
    used: u64,
}

/// Uploaded textures under a byte budget. Keys are any hashable id ([`TileKey`],
/// [`PageKey`] or the caller's own enum).
pub struct TexCache<K> {
    map: HashMap<K, Entry>,
    bytes: u64,
    budget: u64,
    tick: u64,
}

impl<K: Eq + Hash + Clone> TexCache<K> {
    pub fn new(budget_bytes: u64) -> Self {
        TexCache { map: HashMap::new(), bytes: 0, budget: budget_bytes, tick: 0 }
    }

    pub fn set_budget(&mut self, budget_bytes: u64) {
        self.budget = budget_bytes;
    }

    /// Bytes held.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn contains(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    /// The texture, marked as used.
    pub fn get(&mut self, k: &K) -> Option<&Tex> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.used = t;
            &e.tex
        })
    }

    /// Insert (or replace) the texture for `k` on page `page`. Call [`TexCache::trim`]
    /// afterwards to enforce the budget.
    pub fn insert(&mut self, k: K, page: u32, tex: Tex) {
        self.tick += 1;
        self.bytes += tex.bytes();
        if let Some(old) = self.map.insert(k, Entry { tex, page, used: self.tick }) {
            self.bytes -= old.tex.bytes();
        }
    }

    pub fn remove(&mut self, k: &K) -> Option<Tex> {
        let e = self.map.remove(k)?;
        self.bytes -= e.tex.bytes();
        Some(e.tex)
    }

    /// Drop entries until the cache fits its budget: farthest page from `focus_page`
    /// first, least recently used among equals. `keep` entries (visible ones) stay even
    /// over budget. Returns how many were dropped.
    pub fn trim(&mut self, focus_page: f32, keep: impl Fn(&K) -> bool) -> usize {
        if self.bytes <= self.budget {
            return 0;
        }
        let mut order: Vec<(f32, u64, K)> = self
            .map
            .iter()
            .filter(|(k, _)| !keep(k))
            .map(|(k, e)| ((e.page as f32 - focus_page).abs(), e.used, k.clone()))
            .collect();
        // Farthest first, then oldest.
        order.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut dropped = 0;
        for (_, _, k) in order {
            if self.bytes <= self.budget {
                break;
            }
            self.remove(&k);
            dropped += 1;
        }
        dropped
    }

    /// Drop every entry `f` rejects.
    pub fn retain(&mut self, mut f: impl FnMut(&K) -> bool) {
        let mut freed = 0;
        self.map.retain(|k, e| {
            let keep = f(k);
            if !keep {
                freed += e.tex.bytes();
            }
            keep
        });
        self.bytes -= freed;
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::preview_gfx::{Pixels, Uploader, no_upload};

    fn tex(up: &Uploader, side: usize) -> Tex {
        let px = vec![255u8; side * side * 4];
        Tex::upload(up, "t".into(), Pixels { size: [side, side], rgba: &px, premultiplied: true })
    }

    #[test]
    fn tiles_cover_the_view_centre_out() {
        let g = PageGeom { w_pt: 612.0, h_pt: 792.0, rotation: 0 };
        // 300 %: 1836 × 2376 px → 2 × 3 tiles.
        let all = visible_tiles(4, &g, 3000, 0, false, [0.0, 0.0, 5000.0, 5000.0]);
        assert_eq!(all.len(), 6);
        let k = visible_tiles(4, &g, 3000, 0, false, [1000.0, 1000.0, 1100.0, 1100.0]);
        assert_eq!(k.len(), 4, "a view straddling the tile corner");
        assert_eq!((k[0].tx, k[0].ty), (1, 1), "the tile whose centre is nearest the view's centre first");
        assert_eq!((k[3].tx, k[3].ty), (0, 0));
        let far = TileKey { page: 4, scale_milli: 3000, rot: 0, tx: 1, ty: 2, invert: false };
        assert_eq!(
            far.region(&g),
            Some(Region { page: 4, scale: 3.0, rot_quarter: 0, x: 1024, y: 2048, w: 812, h: 328 })
        );
        assert_eq!(TileKey { tx: 2, ..far }.region(&g), None);
        let rotated = visible_tiles(0, &g, 3000, 1, false, [0.0, 0.0, 5000.0, 5000.0]);
        assert_eq!(rotated.iter().map(|k| k.tx).max(), Some(2), "rotated: 2376 px wide");
        assert!(visible_tiles(0, &g, 1000, 0, false, [700.0, 0.0, 900.0, 10.0]).is_empty(), "right of the page");
    }

    #[test]
    fn cache_evicts_farthest_pages_first() {
        let up = no_upload();
        let one = tex(&up, 16).bytes();
        let mut c: TexCache<PageKey> = TexCache::new(one * 3);
        let key = |page| PageKey { page, rot: 0, invert: false };
        for p in [0, 5, 9, 10] {
            c.insert(key(p), p, tex(&up, 16));
        }
        assert_eq!(c.bytes(), one * 4);
        assert_eq!(c.trim(9.0, |_| false), 1);
        assert!(!c.contains(&key(0)), "page 0 is farthest from page 9");
        // Visible entries survive even over budget.
        c.set_budget(0);
        c.trim(0.0, |k| k.page == 10);
        assert_eq!(c.len(), 1);
        assert!(c.get(&key(10)).is_some());
        c.insert(key(10), 10, tex(&up, 8));
        assert_eq!(c.bytes(), tex(&up, 8).bytes(), "replacing frees the old texture");
        c.retain(|_| false);
        assert_eq!((c.len(), c.bytes()), (0, 0));
    }
}
