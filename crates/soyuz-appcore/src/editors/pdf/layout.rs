//! Page layout of the PDF viewer: pages stacked top to bottom, centred horizontally,
//! `gap` points apart (and from the edges).
//!
//! Coordinates:
//! * **doc space**: view points (the frontend's logical pixels) from the top-left of the
//!   whole document at the current zoom;
//! * **page points**: rendered page points of one page (its own `/Rotate` applied, no
//!   view rotation), what the engine's text boxes and links use;
//! * **view-page points**: page points turned by the view rotation (still unscaled).

use crate::services::pdf::PageGeom;

/// `[x0, y0, x1, y1]`.
pub type Rect = [f32; 4];

/// View points per page point at 100 %: a page shows at its printed size on a 96 dpi
/// screen (1 pt = 1/72 in).
pub const ACTUAL: f32 = 96.0 / 72.0;

/// Where every page sits in doc space.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub rects: Vec<Rect>,
    /// `[width, height]` of the whole document (at least the view width).
    pub size: [f32; 2],
    pub gap: f32,
}

impl Layout {
    /// Lay out `geoms` turned `rot` quarter turns at `scale` view points per page point.
    pub fn build(geoms: &[PageGeom], rot: u8, scale: f32, gap: f32, view_w: f32) -> Layout {
        let sizes: Vec<(f32, f32)> = geoms
            .iter()
            .map(|g| {
                let (w, h) = g.rotated(rot);
                (w * scale, h * scale)
            })
            .collect();
        let max_w = sizes.iter().map(|s| s.0).fold(0.0, f32::max);
        let width = (max_w + 2.0 * gap).max(view_w);
        let mut y = gap;
        let mut rects = Vec::with_capacity(sizes.len());
        for (w, h) in sizes {
            let x0 = ((width - w) / 2.0).max(gap.min(width / 2.0));
            rects.push([x0, y, x0 + w, y + h]);
            y += h + gap;
        }
        Layout { rects, size: [width, y.max(gap)], gap }
    }

    pub fn len(&self) -> usize {
        self.rects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// The page at doc height `y` (a gap belongs to the page above it's nearer to).
    pub fn page_at_y(&self, y: f32) -> Option<usize> {
        if self.rects.is_empty() {
            return None;
        }
        let k = self.rects.partition_point(|r| r[3] + self.gap / 2.0 < y);
        Some(k.min(self.rects.len() - 1))
    }

    /// Pages intersecting `view` (doc space), in order.
    pub fn visible(&self, view: Rect) -> std::ops::Range<usize> {
        let a = self.rects.partition_point(|r| r[3] <= view[1]);
        let b = self.rects.partition_point(|r| r[1] < view[3]);
        a..b.max(a)
    }
}

/// Scale (view points per page point) at which the widest page fills `view_w`.
pub fn fit_width(geoms: &[PageGeom], rot: u8, gap: f32, view_w: f32) -> f32 {
    let w = geoms.iter().map(|g| g.rotated(rot).0).fold(0.0, f32::max).max(1.0);
    ((view_w - 2.0 * gap).max(16.0) / w).max(0.01)
}

/// Scale at which the largest page fits wholly in `view`.
pub fn fit_page(geoms: &[PageGeom], rot: u8, gap: f32, view: [f32; 2]) -> f32 {
    let w = geoms.iter().map(|g| g.rotated(rot).0).fold(0.0, f32::max).max(1.0);
    let h = geoms.iter().map(|g| g.rotated(rot).1).fold(0.0, f32::max).max(1.0);
    let sw = (view[0] - 2.0 * gap).max(16.0) / w;
    let sh = (view[1] - 2.0 * gap).max(16.0) / h;
    sw.min(sh).max(0.01)
}

/// Page points → view-page points (view rotation `rot` clockwise, page `geom`).
pub fn rotate_point(geom: &PageGeom, rot: u8, p: [f32; 2]) -> [f32; 2] {
    let (w, h) = (geom.w_pt, geom.h_pt);
    match rot % 4 {
        0 => p,
        1 => [h - p[1], p[0]],
        2 => [w - p[0], h - p[1]],
        _ => [p[1], w - p[0]],
    }
}

/// View-page points → page points (inverse of [`rotate_point`]).
pub fn unrotate_point(geom: &PageGeom, rot: u8, p: [f32; 2]) -> [f32; 2] {
    let (w, h) = (geom.w_pt, geom.h_pt);
    match rot % 4 {
        0 => p,
        1 => [p[1], h - p[0]],
        2 => [w - p[0], h - p[1]],
        _ => [w - p[1], p[0]],
    }
}

/// A page-point rectangle turned by the view rotation (still a normalized rectangle).
pub fn rotate_rect(geom: &PageGeom, rot: u8, r: Rect) -> Rect {
    let a = rotate_point(geom, rot, [r[0], r[1]]);
    let b = rotate_point(geom, rot, [r[2], r[3]]);
    [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter() -> PageGeom {
        PageGeom { w_pt: 612.0, h_pt: 792.0, rotation: 0 }
    }

    #[test]
    fn stacks_and_centres_pages() {
        let small = PageGeom { w_pt: 306.0, h_pt: 396.0, rotation: 0 };
        let l = Layout::build(&[letter(), small], 0, 1.0, 10.0, 400.0);
        assert_eq!(l.size, [632.0, 10.0 + 792.0 + 10.0 + 396.0 + 10.0]);
        assert_eq!(l.rects[0], [10.0, 10.0, 622.0, 802.0]);
        assert_eq!(l.rects[1][0], (632.0 - 306.0) / 2.0);
        assert_eq!(l.page_at_y(0.0), Some(0));
        assert_eq!(l.page_at_y(806.0), Some(0), "the upper half of the gap");
        assert_eq!(l.page_at_y(809.0), Some(1));
        assert_eq!(l.page_at_y(1e9), Some(1));
        assert_eq!(l.visible([0.0, 700.0, 100.0, 900.0]), 0..2);
        assert_eq!(l.visible([0.0, 803.0, 100.0, 811.0]), 1..1, "only the gap");
        // A wide view centres the page.
        let l = Layout::build(&[letter()], 0, 1.0, 10.0, 1000.0);
        assert_eq!(l.rects[0][0], 194.0);
    }

    #[test]
    fn fits_and_rotation_maths() {
        let g = letter();
        assert!((fit_width(&[g], 0, 10.0, 632.0) - 1.0).abs() < 1e-6);
        assert!((fit_width(&[g], 1, 10.0, 812.0) - 1.0).abs() < 1e-6, "rotated: the height is the width");
        assert!((fit_page(&[g], 0, 0.0, [612.0, 396.0]) - 0.5).abs() < 1e-6);
        for rot in 0..4 {
            let p = [100.0, 30.0];
            let q = rotate_point(&g, rot, p);
            assert_eq!(unrotate_point(&g, rot, q), p, "rot {rot}");
        }
        assert_eq!(rotate_point(&g, 1, [0.0, 0.0]), [792.0, 0.0], "top-left goes to the top-right");
        assert_eq!(rotate_rect(&g, 2, [0.0, 0.0, 10.0, 20.0]), [602.0, 772.0, 612.0, 792.0]);
    }
}
